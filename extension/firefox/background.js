// Segmenta background service worker (Chromium classic MV3 worker, reused by
// Firefox through background.scripts).
//
// Responsibilities:
//   * validate every inbound message (sender, type, url, size);
//   * translate an allowlisted message into one JSON-RPC 2.0 envelope;
//   * correlate the native reply by id with a 10 s timeout;
//   * never forward arbitrary methods, urls or commands to the host;
//   * never leave a browser download paused without a watchdog that survives the
//     suspension of this worker;
//   * reconcile the downloads that were already in progress before this worker
//     existed, through the same capture path a live download takes.
if (typeof importScripts === 'function') {
  importScripts('./shared/browser-api.js', './shared/protocol.js', './shared/validate.js');
}

(function (global) {
  'use strict';

  const API = global.SegmentaApi;
  const PROTOCOL = global.SegmentaProtocol;
  const VALIDATE = global.SegmentaValidate;

  const HOST_NAME = 'io.github.wilkin.segmenta';
  const NATIVE_TIMEOUT_MS = 10000;
  // Bumped whenever the settings contract gains keys the extension reads, so an
  // older cache can never silently answer with the previous defaults (S11).
  const SETTINGS_CACHE_VERSION = '0.1.3';
  // HLS/DASH sites are opt-in for the future optional host permission; until the
  // user grants a site, only the fixed allowlist below is accepted.
  const STREAM_ALLOWLIST = [];

  const handledDownloadIds = new Set();

  // ---- Capture confirmation state (S10) --------------------------------------
  const CONFIRM_PAGE = 'confirm.html';
  const CAPTURE_CONFIRM_TIMEOUT_MS = 120000;
  const MAX_PENDING_CONFIRMS = 20;
  // The browser copy is the safety net until the daemon demonstrably holds bytes:
  // both waits below are bounded and every expiry keeps the browser download.
  const TASK_OWNERSHIP_TIMEOUT_MS = 60000;
  const TASK_POLL_INTERVAL_MS = 1000;
  const CONFIRM_WINDOW_TIMEOUT_MS = 5000;
  // The watchdog that survives MV3 service-worker suspension. `alarms` is the only
  // timer the browser keeps across a suspended worker, and 1 minute is Chrome's
  // production minimum period; the ~120 s decision timeout stays the deadline the
  // persisted record carries, never the only guard.
  const PENDING_CAPTURE_SCHEMA = 1;
  const PENDING_CAPTURE_KEY = 'segmenta_pending_captures_v' + PENDING_CAPTURE_SCHEMA;
  const CAPTURE_WATCHDOG_ALARM = 'segmenta-capture-watchdog';
  const CAPTURE_WATCHDOG_PERIOD_MINUTES = 1;
  // A terminal state means the daemon will never hold bytes for this task.
  const TASK_FAILED_STATUSES = ['failed', 'error', 'cancelled'];
  // downloadId -> { windowId, deadline, timer }. The browser download stays paused
  // while the entry exists; every exit path resumes it. This map is only the fast
  // path: the persisted record plus the alarms watchdog outlive this worker.
  const pendingConfirms = new Map();
  // Download ids whose confirmation was already resolved (accept/reject/timeout).
  const decidedDownloads = new Set();
  // Download ids whose browser copy was already handed over to the daemon: a
  // reconciliation must never capture the same download a second time.
  const capturedDownloadIds = new Set();
  // Overridable for the Node tests only; production always uses the constants.
  let confirmTimeoutMs = CAPTURE_CONFIRM_TIMEOUT_MS;
  const captureTiming = {
    ownershipTimeoutMs: TASK_OWNERSHIP_TIMEOUT_MS,
    pollIntervalMs: TASK_POLL_INTERVAL_MS,
    windowTimeoutMs: CONFIRM_WINDOW_TIMEOUT_MS,
  };

  function trackHandledId(id) {
    if (handledDownloadIds.size >= 100) {
      const first = handledDownloadIds.values().next().value;
      handledDownloadIds.delete(first);
    }
    handledDownloadIds.add(id);
  }

  function trackDecidedId(id) {
    if (decidedDownloads.size >= 100) {
      const first = decidedDownloads.values().next().value;
      decidedDownloads.delete(first);
    }
    decidedDownloads.add(id);
  }

  function trackCapturedId(id) {
    if (capturedDownloadIds.size >= 100) {
      const first = capturedDownloadIds.values().next().value;
      capturedDownloadIds.delete(first);
    }
    capturedDownloadIds.add(id);
  }

  function extensionId() {
    const runtime = API && API.resolveRuntime ? API.resolveRuntime() : null;
    const id = runtime && runtime.runtime ? runtime.runtime.id : '';
    return typeof id === 'string' ? id : '';
  }

  function hostFailure() {
    return { ok: false, error: { code: 'ERR_HOST_UNAVAILABLE', message: 'native host is not available' } };
  }

  function failure(code, message) {
    return { ok: false, error: { code: code, message: message } };
  }

  function withTimeout(promise, timeoutMs) {
    return new Promise((resolve, reject) => {
      let settled = false;
      const timer = setTimeout(() => {
        if (settled) {
          return;
        }
        settled = true;
        reject(Object.assign(new Error('native host timeout'), { code: 'ERR_HOST_TIMEOUT' }));
      }, timeoutMs);
      promise.then(
        (value) => {
          if (settled) {
            return;
          }
          settled = true;
          clearTimeout(timer);
          resolve(value);
        },
        (err) => {
          if (settled) {
            return;
          }
          settled = true;
          clearTimeout(timer);
          reject(err);
        },
      );
    });
  }

  // Sends exactly one envelope built from the allowlisted method table and
  // returns {ok:true,data} or {ok:false,error} for the message layer.
  async function callHost(build) {
    let envelope;
    try {
      envelope = build();
    } catch (err) {
      return failure(err && err.code ? err.code : 'ERR_INVALID_PARAMS', 'request could not be built');
    }
    let reply;
    try {
      reply = await withTimeout(API.sendNative(envelope), NATIVE_TIMEOUT_MS);
    } catch (err) {
      const code = err && err.code === 'ERR_HOST_TIMEOUT' ? 'ERR_HOST_TIMEOUT' : 'ERR_HOST_UNAVAILABLE';
      return failure(code, code === 'ERR_HOST_TIMEOUT' ? 'native host timed out' : 'native host is not available');
    }
    let verdict;
    try {
      verdict = PROTOCOL.validateHostReply(reply, envelope.id);
    } catch (err) {
      return failure('ERR_HOST_PROTOCOL', 'native host reply was rejected');
    }
    if (!verdict.ok) {
      return {
        ok: false,
        error: {
          code: PROTOCOL.errorCodeName(verdict.error.code),
          message: PROTOCOL.sanitizeText(verdict.error.message, 300),
          host_code: verdict.error.code,
        },
      };
    }
    return { ok: true, data: verdict.result };
  }

  // ---- "Added to Segmenta" toast and task focus (S8) --------------------------
  // After a successful add the user must see where the task landed and get Segmenta
  // in front focused on it. Both steps are best effort on purpose: the task already
  // exists in the daemon, so a muted notification or a missing GUI can never turn a
  // successful add into a failed one.
  const NOTIFICATION_ID = 'segmenta-task-added';
  const NOTIFICATION_TIMEOUT_MS = 2000;
  const NOTIFICATION_ICON = 'icons/icon-128.png';
  const CAPTURE_KEPT_NOTIFICATION_ID = 'segmenta-capture-kept';
  // ASCII source, Spanish literal: the catalogs supply the real title.
  const TASK_ADDED_TITLE_FALLBACK = 'A\u00f1adida a Segmenta';
  const TASK_ADDED_BODY_FALLBACK = 'La tarea se abri\u00f3 en Segmenta.';
  const CAPTURE_KEPT_TITLE_FALLBACK = 'Descarga conservada en el navegador';
  const CAPTURE_KEPT_BODY_FALLBACK =
    'Segmenta no recibi\u00f3 los datos. La descarga contin\u00faa en tu navegador.';

  function message(key, substitutions) {
    if (!API || typeof API.getMessage !== 'function') {
      return '';
    }
    const text = substitutions === undefined ? API.getMessage(key) : API.getMessage(key, substitutions);
    return typeof text === 'string' ? text : '';
  }

  /** Folder that contains a path; the same helper the confirmation window uses. */
  function parentDirectory(value) {
    return PROTOCOL.parentDirectory(value);
  }

  /** Folder the task lands in: destination_dir, else the parent of final_path. */
  function destinationFolder(data) {
    if (!data || typeof data !== 'object') {
      return '';
    }
    const destination = typeof data.destination_dir === 'string' ? data.destination_dir.trim() : '';
    if (destination) {
      return destination;
    }
    return parentDirectory(typeof data.final_path === 'string' ? data.final_path : '');
  }

  function taskAddedBody(folder) {
    if (folder) {
      return message('taskAddedBody', [folder]) || folder;
    }
    return message('taskAddedBodyGeneric') || TASK_ADDED_BODY_FALLBACK;
  }

  function notificationsApi() {
    const runtime = API && typeof API.resolveRuntime === 'function' ? API.resolveRuntime() : null;
    return runtime && runtime.notifications ? runtime.notifications : null;
  }

  // Resolves to true when the notification was handed to the browser; it never
  // rejects. An absent API, a refused permission, a create() error and a silent
  // browser are all reported as "no notification" instead of an error.
  function notifyUser(notificationId, title, body) {
    const notifications = notificationsApi();
    if (!notifications || typeof notifications.create !== 'function') {
      return Promise.resolve(false);
    }
    const options = {
      type: 'basic',
      iconUrl: NOTIFICATION_ICON,
      title: title,
      message: body,
    };
    return new Promise((resolve) => {
      let settled = false;
      let timer = null;
      const finish = (value) => {
        if (settled) {
          return;
        }
        settled = true;
        if (timer) {
          clearTimeout(timer);
        }
        resolve(value);
      };
      timer = setTimeout(() => finish(false), NOTIFICATION_TIMEOUT_MS);
      try {
        const returned = notifications.create(notificationId, options, () => finish(true));
        if (returned && typeof returned.then === 'function') {
          returned.then(() => finish(true), () => finish(false));
        } else if (typeof returned !== 'undefined') {
          finish(true);
        }
      } catch (err) {
        finish(false);
      }
    });
  }

  function showTaskNotification(folder) {
    return notifyUser(
      NOTIFICATION_ID,
      message('taskAddedTitle') || TASK_ADDED_TITLE_FALLBACK,
      taskAddedBody(folder),
    );
  }

  /** Tells the user the browser copy was kept because Segmenta never got the data. */
  function showCaptureKeptNotification() {
    return notifyUser(
      CAPTURE_KEPT_NOTIFICATION_ID,
      message('captureKeptTitle') || CAPTURE_KEPT_TITLE_FALLBACK,
      message('captureKeptBody') || CAPTURE_KEPT_BODY_FALLBACK,
    );
  }

  function openTask(taskId) {
    return callHost(() => PROTOCOL.buildSystemOpen({ taskId: taskId }));
  }

  /** Notify first (with the destination folder), then focus the task. Never throws. */
  async function announceAddedTask(data) {
    const taskId = data && typeof data.task_id === 'string' ? data.task_id.trim() : '';
    if (!taskId) {
      return false;
    }
    try {
      await showTaskNotification(destinationFolder(data));
    } catch (err) {}
    try {
      await openTask(taskId);
    } catch (err) {}
    return true;
  }

  // Calls the host once; on success it also announces the new task without ever
  // altering the reply the caller (popup/panel/content script) receives.
  async function callHostAndAnnounce(build) {
    const reply = await callHost(build);
    if (reply && reply.ok) {
      await announceAddedTask(reply.data);
    }
    return reply;
  }

  async function getCachedSettings() {
    const cacheVersion = SETTINGS_CACHE_VERSION;
    try {
      if (API.storage && typeof API.storage.get === 'function') {
        const stored = await API.storage.get(['segmenta_settings', 'segmenta_settings_version']);
        if (stored && stored.segmenta_settings && stored.segmenta_settings_version === cacheVersion) {
          return stored.segmenta_settings;
        }
      }
    } catch (err) {}

    const reply = await callHost(() => PROTOCOL.envelope('settings.get', {}));
    if (reply.ok && reply.data) {
      try {
        if (API.storage && typeof API.storage.set === 'function') {
          await API.storage.set({ segmenta_settings: reply.data, segmenta_settings_version: cacheVersion });
        }
      } catch (err) {}
      return reply.data;
    }
    return null;
  }

  function matchExtension(filenameOrUrl, extensionsList) {
    if (!Array.isArray(extensionsList) || extensionsList.length === 0) {
      return false;
    }
    let ext = '';
    try {
      const parsed = new URL(filenameOrUrl);
      const pathname = parsed.pathname;
      const lastSegment = pathname.split('/').pop() || '';
      const dotIdx = lastSegment.lastIndexOf('.');
      if (dotIdx !== -1) {
        ext = lastSegment.slice(dotIdx + 1).toLowerCase();
      }
    } catch (err) {
      const dotIdx = filenameOrUrl.lastIndexOf('.');
      if (dotIdx !== -1) {
        ext = filenameOrUrl.slice(dotIdx + 1).toLowerCase();
      }
    }
    if (!ext) return false;

    for (const pattern of extensionsList) {
      const pat = String(pattern).trim().toLowerCase().replace(/^\./, '');
      if (!pat) continue;
      if (pat.endsWith('*')) {
        const prefix = pat.slice(0, -1);
        if (ext.startsWith(prefix)) {
          return true;
        }
      } else {
        if (ext === pat) {
          return true;
        }
      }
    }
    return false;
  }

  function isHostDenylisted(url, denylist) {
    if (!Array.isArray(denylist) || denylist.length === 0) {
      return false;
    }
    try {
      const parsed = new URL(url);
      const hostname = parsed.hostname.toLowerCase();
      for (const domain of denylist) {
        const d = String(domain).trim().toLowerCase();
        if (!d) continue;
        if (hostname === d || hostname.endsWith('.' + d)) {
          return true;
        }
      }
    } catch (err) {}
    return false;
  }

  // ---- Capture with confirmation (S10) ---------------------------------------
  // A browser download is never cancelled before Segmenta owns it: it is paused,
  // the user answers in our own window, and only a successful add discards the
  // browser copy. Every other outcome (reject, timeout, closed window, host
  // failure) resumes the browser download, so the file is never lost.

  /** `capture_confirm` defaults to true when the daemon does not report it. */
  function captureConfirmEnabled(settings) {
    return !(settings && settings.capture_confirm === false);
  }

  /**
   * Effective destination folder reported by the daemon. `Settings` only carries
   * `download_dir`; the per-task folder arrives later with the add reply. The
   * extension never invents a path: without it the window shows it as pending.
   */
  function captureFolder(settings) {
    if (!settings || typeof settings !== 'object') {
      return '';
    }
    return typeof settings.download_dir === 'string' ? settings.download_dir.trim() : '';
  }

  function buildConfirmUrl(baseUrl, entry) {
    const params = new URLSearchParams();
    params.set('download', String(entry.downloadId));
    params.set('url', entry.url);
    if (entry.filename) {
      params.set('filename', entry.filename);
    }
    if (entry.referrer) {
      params.set('referrer', entry.referrer);
    }
    if (entry.folder) {
      params.set('folder', entry.folder);
    }
    return baseUrl + '?' + params.toString();
  }

  function confirmPageUrl() {
    const api = API && typeof API.resolveRuntime === 'function' ? API.resolveRuntime() : null;
    const runtime = api && api.runtime ? api.runtime : null;
    if (runtime && typeof runtime.getURL === 'function') {
      return runtime.getURL(CONFIRM_PAGE);
    }
    return CONFIRM_PAGE;
  }

  async function pauseDownload(downloadId) {
    if (!API.downloads || typeof API.downloads.pause !== 'function') {
      return false;
    }
    try {
      await API.downloads.pause(downloadId);
      return true;
    } catch (err) {
      return false;
    }
  }

  async function resumeDownload(downloadId) {
    if (!API.downloads || typeof API.downloads.resume !== 'function') {
      return false;
    }
    try {
      await API.downloads.resume(downloadId);
      return true;
    } catch (err) {
      return false;
    }
  }

  /** Cancels and erases the browser copy. Only called once the daemon holds bytes. */
  async function discardDownload(downloadId) {
    try {
      if (API.downloads && typeof API.downloads.cancel === 'function') {
        await API.downloads.cancel(downloadId);
      }
    } catch (err) {}
    try {
      if (API.downloads && typeof API.downloads.erase === 'function') {
        await API.downloads.erase({ id: downloadId });
      }
    } catch (err) {}
  }

  // ---- Suspension-safe capture watchdog (S10) --------------------------------
  // An MV3 service worker is suspended at will, so an in-memory timer can never be
  // the only guard for a paused browser download. Every pause is preceded by a
  // persisted pending record plus a periodic alarm, a fresh worker rehydrates both,
  // and no record survives an exit: accept, reject, closed window, timeout or a
  // failed host all remove it and clear the alarm once the list is empty.

  // Serialises pending-record mutations: pause, decision, sweep and alarm can run
  // concurrently in one worker, and a read-modify-write must not interleave.
  let pendingStoreChain = Promise.resolve();

  function withPendingStore(work) {
    const run = pendingStoreChain.then(work, work);
    pendingStoreChain = run.then(
      () => {},
      () => {},
    );
    return run;
  }

  async function readPendingRecords() {
    const stored = await API.storage.get([PENDING_CAPTURE_KEY]);
    const value = stored ? stored[PENDING_CAPTURE_KEY] : undefined;
    if (value === undefined || value === null) {
      return [];
    }
    // A scalar instead of a list is unreadable state, not an empty one: keep it so
    // the sweep can drop it instead of silently leaking the paused download.
    return Array.isArray(value) ? value : [value];
  }

  // An empty list is the "no pending record" state, written through the same
  // storage.set path the settings cache already uses: one proven adapter call
  // instead of a second one that could behave differently per browser.
  async function writePendingRecords(records) {
    await API.storage.set({ [PENDING_CAPTURE_KEY]: Array.isArray(records) ? records : [] });
    return true;
  }

  /** Accepts only a well-formed record; anything else is healed by the sweep. */
  function parsePendingRecord(raw) {
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
      return { ok: false, downloadId: null };
    }
    const downloadId = Number(raw.downloadId);
    if (!Number.isInteger(downloadId) || downloadId < 0) {
      return { ok: false, downloadId: null };
    }
    const deadline = Number(raw.deadline);
    // `null` means "the confirmation window is not recorded yet", never window 0:
    // Number(null) is 0 and an integer, which would make a live window look gone.
    const hasWindow = raw.windowId !== null && raw.windowId !== undefined;
    const windowId = Number(raw.windowId);
    const windowIsKnown = !hasWindow || (Number.isInteger(windowId) && windowId > 0);
    const valid = Number(raw.v) === PENDING_CAPTURE_SCHEMA
      && Number.isFinite(deadline)
      && deadline > 0
      && typeof raw.url === 'string'
      && raw.url.length > 0
      && windowIsKnown;
    if (!valid) {
      return { ok: false, downloadId: downloadId };
    }
    return {
      ok: true,
      downloadId: downloadId,
      record: {
        v: PENDING_CAPTURE_SCHEMA,
        downloadId: downloadId,
        url: raw.url,
        filename: typeof raw.filename === 'string' ? raw.filename : '',
        referrer: typeof raw.referrer === 'string' ? raw.referrer : '',
        windowId: hasWindow && Number.isInteger(windowId) ? windowId : null,
        deadline: deadline,
      },
    };
  }

  async function createCaptureWatchdogAlarm() {
    if (!API.alarms || typeof API.alarms.create !== 'function') {
      throw new Error('segmenta: alarms api unavailable');
    }
    await API.alarms.create(CAPTURE_WATCHDOG_ALARM, {
      delayInMinutes: CAPTURE_WATCHDOG_PERIOD_MINUTES,
      periodInMinutes: CAPTURE_WATCHDOG_PERIOD_MINUTES,
    });
    return true;
  }

  async function clearCaptureWatchdogAlarm() {
    if (!API.alarms || typeof API.alarms.clear !== 'function') {
      return false;
    }
    try {
      await API.alarms.clear(CAPTURE_WATCHDOG_ALARM);
      return true;
    } catch (err) {
      return false;
    }
  }

  /**
   * Persists the pending capture and arms the watchdog before anything is paused.
   * False means "do not pause": without a record and a live alarm the download must
   * stay untouched in the browser, even at the cost of skipping the capture.
   */
  async function armPendingCapture(entry) {
    const record = {
      v: PENDING_CAPTURE_SCHEMA,
      downloadId: entry.downloadId,
      url: entry.url,
      filename: entry.filename || '',
      referrer: entry.referrer || '',
      windowId: null,
      deadline: Date.now() + confirmTimeoutMs,
    };
    try {
      await withPendingStore(async () => {
        const records = (await readPendingRecords()).filter(
          (candidate) => parsePendingRecord(candidate).downloadId !== record.downloadId,
        );
        records.push(record);
        await writePendingRecords(records);
      });
    } catch (err) {
      return false;
    }
    try {
      await createCaptureWatchdogAlarm();
    } catch (err) {
      // No watchdog means no pause: undo the record we just wrote.
      await resolvePendingCapture(entry.downloadId);
      return false;
    }
    entry.deadline = record.deadline;
    return true;
  }

  /** Records the confirmation window so a fresh worker can check it still exists. */
  async function rememberPendingWindow(downloadId, windowId) {
    if (!Number.isInteger(windowId)) {
      return false;
    }
    try {
      await withPendingStore(async () => {
        const records = await readPendingRecords();
        let changed = false;
        const updated = records.map((candidate) => {
          const parsed = parsePendingRecord(candidate);
          if (!parsed.ok || parsed.record.downloadId !== downloadId) {
            return candidate;
          }
          changed = true;
          return Object.assign({}, parsed.record, { windowId: windowId });
        });
        if (changed) {
          await writePendingRecords(updated);
        }
      });
      return true;
    } catch (err) {
      return false;
    }
  }

  /**
   * Removes one pending record and clears the alarm when nothing is left. The
   * browser copy is resumed first when asked, so a crash in between still leaves the
   * watchdog in place instead of an orphan paused download.
   */
  async function resolvePendingCapture(downloadId, options) {
    if (options && options.resume) {
      await resumeDownload(downloadId);
    }
    let remaining = null;
    try {
      remaining = await withPendingStore(async () => {
        const records = await readPendingRecords();
        const kept = records.filter(
          (candidate) => parsePendingRecord(candidate).downloadId !== downloadId,
        );
        await writePendingRecords(kept);
        return kept;
      });
    } catch (err) {
      return -1;
    }
    if (remaining.length === 0) {
      await clearCaptureWatchdogAlarm();
    }
    return remaining.length;
  }

  /** True when the confirmation window still exists; unknown means "leave it alone". */
  async function confirmWindowStillOpen(windowId) {
    if (!Number.isInteger(windowId)) {
      return true;
    }
    const windows = API && API.windows ? API.windows : null;
    if (!windows || typeof windows.get !== 'function') {
      return true;
    }
    try {
      const found = await windows.get(windowId);
      return !!found;
    } catch (err) {
      return false;
    }
  }

  /**
   * Rehydrates every pending record and finishes the job: an expired deadline, a
   * vanished confirmation window or an unreadable payload resumes the browser
   * download; a record whose window is still open is kept and the alarm re-armed.
   */
  async function sweepPendingCaptures() {
    const summary = { resumed: [], kept: 0, dropped: 0 };
    let records;
    try {
      records = await withPendingStore(readPendingRecords);
    } catch (err) {
      // Storage unreadable: the in-memory timers remain, nothing is resumed blind.
      return summary;
    }
    const now = Date.now();
    const resumeIds = [];
    for (const raw of records) {
      const parsed = parsePendingRecord(raw);
      if (!parsed.ok) {
        if (parsed.downloadId !== null) {
          resumeIds.push(parsed.downloadId);
        }
        continue;
      }
      const record = parsed.record;
      if (record.deadline <= now) {
        resumeIds.push(record.downloadId);
        continue;
      }
      // The window id is remembered right after it is created. A record without one
      // still has its deadline, so keep it instead of resuming a live window.
      if (record.windowId === null) {
        continue;
      }
      if (!(await confirmWindowStillOpen(record.windowId))) {
        resumeIds.push(record.downloadId);
      }
    }
    for (const downloadId of resumeIds) {
      await resumeDownload(downloadId);
      summary.resumed.push(downloadId);
    }
    // The window checks await, so a capture can be armed meanwhile. The write is
    // therefore computed from a fresh read inside the lock and removes only the ids
    // this sweep decided to resume: a record armed during the sweep is never dropped
    // and its alarm is never cleared.
    let kept;
    try {
      kept = await withPendingStore(async () => {
        const current = await readPendingRecords();
        const survivors = current.filter((raw) => {
          const parsed = parsePendingRecord(raw);
          if (!parsed.ok) {
            summary.dropped += 1;
            return false;
          }
          return !resumeIds.includes(parsed.record.downloadId);
        });
        await writePendingRecords(survivors);
        return survivors.length;
      });
    } catch (err) {
      return summary;
    }
    summary.kept = kept;
    if (kept === 0) {
      await clearCaptureWatchdogAlarm();
    } else {
      try {
        await createCaptureWatchdogAlarm();
      } catch (err) {}
    }
    return summary;
  }

  // ---- Cold-start reconciliation (R1) ----------------------------------------
  // A download the browser started while no worker was alive never fires
  // `downloads.onCreated`, so its capture would silently be lost. On worker start
  // (`runtime.onStartup`, `runtime.onInstalled`) and on a throttled watchdog tick
  // the worker asks the browser what is still in progress and routes every item
  // through the same gate a live download takes: same settings, same denylist, same
  // confirmation window, same ownership and same watchdog rule. The scan is bounded
  // and fails closed: an API error leaves every download alone, it never throws.
  const MAX_RECONCILE_DOWNLOADS = 20;
  // One minute is the alarm period; scanning on every tick would turn the watchdog
  // into a permanent poll. A fresh worker resets this, so it always reconciles once.
  const RECONCILE_THROTTLE_MS = 5 * 60 * 1000;
  let lastReconcileAt = 0;
  let reconcileInFlight = null;

  function emptyReconcileSummary() {
    return { examined: 0, considered: 0, skipped: 0, failed: false };
  }

  /** The in-progress list, or a rejection the caller turns into "do nothing". */
  async function searchInProgressDownloads() {
    const downloads = API && API.downloads ? API.downloads : null;
    if (!downloads || typeof downloads.search !== 'function') {
      throw new Error('segmenta: downloads.search unavailable');
    }
    const found = await downloads.search({ state: 'in_progress' });
    if (!Array.isArray(found)) {
      throw new Error('segmenta: downloads.search returned no list');
    }
    return found;
  }

  /** Ids already paused, already resolved or already captured are never revisited. */
  function reconcileSkipReason(id, pendingIds) {
    if (handledDownloadIds.has(id)) {
      return 'handled';
    }
    if (decidedDownloads.has(id)) {
      return 'decided';
    }
    if (capturedDownloadIds.has(id)) {
      return 'captured';
    }
    if (pendingIds.has(id)) {
      return 'pending';
    }
    return '';
  }

  async function readPendingDownloadIds() {
    const records = await withPendingStore(readPendingRecords);
    const ids = new Set();
    for (const raw of records) {
      const parsed = parsePendingRecord(raw);
      if (parsed.downloadId !== null && parsed.downloadId !== undefined) {
        ids.add(parsed.downloadId);
      }
    }
    return ids;
  }

  async function runReconcile() {
    const summary = emptyReconcileSummary();
    let found;
    try {
      found = await searchInProgressDownloads();
    } catch (err) {
      summary.failed = true;
      return summary;
    }
    let pendingIds;
    try {
      pendingIds = await readPendingDownloadIds();
    } catch (err) {
      // Unreadable pending state: never pause anything blind.
      summary.failed = true;
      return summary;
    }
    for (const candidate of found) {
      if (summary.examined >= MAX_RECONCILE_DOWNLOADS) {
        break;
      }
      const id = candidate && typeof candidate === 'object' ? Number(candidate.id) : NaN;
      if (!Number.isInteger(id) || id < 0) {
        continue;
      }
      summary.examined += 1;
      if (reconcileSkipReason(id, pendingIds)) {
        summary.skipped += 1;
        continue;
      }
      let considered;
      try {
        considered = await considerDownload(candidate);
      } catch (err) {
        // The first browser API failure ends this scan; the remaining downloads keep
        // running untouched in the browser and the next tick retries them.
        summary.failed = true;
        break;
      }
      if (considered) {
        summary.considered += 1;
        pendingIds.add(id);
      } else {
        summary.skipped += 1;
      }
    }
    return summary;
  }

  /**
   * Bounded, throttled cold-start reconciliation. It always resolves with a summary:
   * the lifecycle listeners it runs from are no place for a browser API exception.
   */
  function reconcileInProgressDownloads(options) {
    const force = !!(options && options.force);
    if (reconcileInFlight) {
      return reconcileInFlight;
    }
    if (!force && Date.now() - lastReconcileAt < RECONCILE_THROTTLE_MS) {
      return Promise.resolve(emptyReconcileSummary());
    }
    lastReconcileAt = Date.now();
    const run = runReconcile().then(
      (summary) => {
        reconcileInFlight = null;
        return summary;
      },
      () => {
        reconcileInFlight = null;
        const failed = emptyReconcileSummary();
        failed.failed = true;
        return failed;
      },
    );
    reconcileInFlight = run;
    return run;
  }

  /** Reconcile first, then heal the pending records the reconciliation skipped. */
  function reconcileThenSweep(options) {
    const reconcile = reconcileInProgressDownloads(options);
    return reconcile.then(
      () => sweepPendingCaptures(),
      () => sweepPendingCaptures(),
    );
  }

  function onCaptureWatchdogAlarm(alarm) {
    if (!alarm || alarm.name !== CAPTURE_WATCHDOG_ALARM) {
      return false;
    }
    // The tick is also the only periodic signal a suspended worker gets, so it both
    // heals the pending records and (throttled) reconciles what it missed.
    return reconcileThenSweep();
  }

  if (API && API.alarms && API.alarms.onAlarm && typeof API.alarms.onAlarm.addListener === 'function') {
    API.alarms.onAlarm.addListener(onCaptureWatchdogAlarm);
  }

  // A fresh browser run starts with no windows, so any leftover record is healed
  // by the same sweep instead of waiting for the first alarm; the same boot also
  // reconciles the downloads that already existed before this worker.
  if (API && API.runtime && API.runtime.onStartup && typeof API.runtime.onStartup.addListener === 'function') {
    API.runtime.onStartup.addListener(() => reconcileThenSweep({ force: true }));
  }

  // An install or an update is a worker start too: a download in progress during it
  // must still be captured.
  if (API && API.runtime && API.runtime.onInstalled && typeof API.runtime.onInstalled.addListener === 'function') {
    API.runtime.onInstalled.addListener(() => reconcileInProgressDownloads({ force: true }));
  }

  function addCapturedDownload(entry) {
    if (PROTOCOL.parseTorrentUrl(entry.url)) {
      return callHost(() => PROTOCOL.buildTorrentAdd({ url: entry.url }));
    }
    return callHost(() =>
      PROTOCOL.buildDownloadAdd({
        url: entry.url,
        filename: entry.filename,
        referrer: entry.referrer,
      }),
    );
  }

  function sleep(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  function taskStatusName(data) {
    return data && typeof data.status === 'string' ? data.status.trim().toLowerCase() : '';
  }

  /** True once the daemon holds bytes for the task, or already finished it. */
  function taskOwnsBytes(data) {
    if (!data || typeof data !== 'object') {
      return false;
    }
    const status = taskStatusName(data);
    if (status === 'completed') {
      return true;
    }
    const bytes = Number(data.downloaded_bytes);
    return status === 'downloading' && Number.isFinite(bytes) && bytes > 0;
  }

  function taskFailed(data) {
    return TASK_FAILED_STATUSES.indexOf(taskStatusName(data)) !== -1;
  }

  /**
   * Bounded wait for the daemon to demonstrably own the file, over the same
   * `media.status` path the panel already uses. A host error is treated as "not yet"
   * so only the budget can end the wait: a transient status failure never loses the
   * download.
   */
  async function waitForTaskOwnership(taskId) {
    const deadline = Date.now() + captureTiming.ownershipTimeoutMs;
    for (;;) {
      const reply = await callHost(() => PROTOCOL.buildStatus({ taskId: taskId }));
      if (reply.ok) {
        if (taskOwnsBytes(reply.data)) {
          return true;
        }
        if (taskFailed(reply.data)) {
          return false;
        }
      }
      const remaining = deadline - Date.now();
      if (remaining <= 0) {
        return false;
      }
      await sleep(Math.min(captureTiming.pollIntervalMs, remaining));
    }
  }

  /** Best effort: drop a task the extension created but cannot own. No file deleted. */
  async function dropCapturedTask(taskId) {
    if (!taskId) {
      return false;
    }
    const reply = await callHost(() => PROTOCOL.buildDownloadCancel({ taskId: taskId }));
    return !!(reply && reply.ok);
  }

  /**
   * Keeps the browser copy: resume it, drop the orphan task and, when no window
   * can explain it, tell the user the download stayed in the browser.
   */
  async function keepBrowserCopy(entry, taskId) {
    await resumeDownload(entry.downloadId);
    await dropCapturedTask(taskId);
    if (entry.inform) {
      try {
        await showCaptureKeptNotification();
      } catch (err) {}
    }
    return failure(
      'ERR_CAPTURE_NOT_OWNED',
      'the task never received data; the browser copy was kept',
    );
  }

  /**
   * Safe capture order: add, wait until the task demonstrably holds bytes, then
   * cancel+erase and announce. Anything short of proven ownership resumes the
   * browser download, which still owns the file.
   */
  async function completeCapture(entry) {
    const reply = await addCapturedDownload(entry);
    if (!reply.ok) {
      await resumeDownload(entry.downloadId);
      return reply;
    }
    const taskId = reply.data && typeof reply.data.task_id === 'string' ? reply.data.task_id.trim() : '';
    const owned = taskId ? await waitForTaskOwnership(taskId) : false;
    if (!owned) {
      return keepBrowserCopy(entry, taskId);
    }
    await discardDownload(entry.downloadId);
    trackCapturedId(entry.downloadId);
    await announceAddedTask(reply.data);
    return reply;
  }

  function forgetConfirm(downloadId) {
    const pending = pendingConfirms.get(downloadId);
    if (!pending) {
      return null;
    }
    pendingConfirms.delete(downloadId);
    if (pending.timer) {
      clearTimeout(pending.timer);
    }
    return pending;
  }

  async function closeConfirmWindow(windowId) {
    if (windowId === null || !API.windows || typeof API.windows.remove !== 'function') {
      return false;
    }
    try {
      await API.windows.remove(windowId);
      return true;
    } catch (err) {
      return false;
    }
  }

  /** Timeout, closed window or an abandoned capture: keep the file. */
  async function abandonConfirm(downloadId) {
    const pending = forgetConfirm(downloadId);
    if (!pending) {
      return false;
    }
    trackDecidedId(downloadId);
    await closeConfirmWindow(pending.windowId);
    // Resume first, then drop the record: a crash in between still leaves the
    // watchdog, never an orphan paused download.
    await resolvePendingCapture(downloadId, { resume: true });
    return true;
  }

  async function openConfirmWindow(entry) {
    const windows = API && API.windows ? API.windows : null;
    if (!windows || typeof windows.create !== 'function') {
      await resumeDownload(entry.downloadId);
      return false;
    }
    if (pendingConfirms.size >= MAX_PENDING_CONFIRMS) {
      await abandonConfirm(pendingConfirms.keys().next().value);
    }
    let created = null;
    // windows.create is the last unguarded await of the pause -> handoff stretch: a
    // browser that never answers must not leave the download paused forever.
    const attempt = new Promise((resolve, reject) => {
      try {
        resolve(
          windows.create({
            url: buildConfirmUrl(confirmPageUrl(), entry),
            type: 'popup',
            width: 460,
            height: 380,
          }),
        );
      } catch (err) {
        reject(err);
      }
    });
    try {
      created = await withTimeout(attempt, captureTiming.windowTimeoutMs);
    } catch (err) {
      // A window that shows up after the timeout would be orphaned: close it and
      // let the browser download continue.
      attempt.then(
        (late) => {
          if (late && typeof late.id === 'number') {
            closeConfirmWindow(late.id);
          }
        },
        () => {},
      );
      await resumeDownload(entry.downloadId);
      return false;
    }
    const windowId = created && typeof created.id === 'number' ? created.id : null;
    // The record already bounds the wait with the persisted deadline; this timer is
    // only the fast path while the worker is still alive.
    const deadline = Number.isFinite(entry.deadline) ? entry.deadline : Date.now() + confirmTimeoutMs;
    const timer = setTimeout(() => {
      abandonConfirm(entry.downloadId);
    }, Math.max(0, deadline - Date.now()));
    pendingConfirms.set(entry.downloadId, { windowId: windowId, deadline: deadline, timer: timer });
    await rememberPendingWindow(entry.downloadId, windowId);
    return true;
  }

  function onWindowRemoved(windowId) {
    for (const [downloadId, pending] of pendingConfirms) {
      if (pending.windowId === windowId) {
        abandonConfirm(downloadId);
        return;
      }
    }
    // A worker that was suspended lost the in-memory map, so the sweep is the only
    // way to notice that a paused download has no window left to decide on it.
    return sweepPendingCaptures();
  }

  /** Handles the answer coming back from confirm.html. */
  async function decideCapture(value) {
    const downloadId = value.downloadId;
    if (decidedDownloads.has(downloadId)) {
      return failure('ERR_CAPTURE_DECIDED', 'this download was already decided');
    }
    trackDecidedId(downloadId);
    forgetConfirm(downloadId);

    if (value.decision !== 'accept') {
      await resolvePendingCapture(downloadId, { resume: true });
      return { ok: true, data: { accepted: false, download_id: downloadId } };
    }

    const entry = {
      downloadId: downloadId,
      url: value.url,
      filename: value.filename || '',
      referrer: value.referrer || '',
      folder: '',
    };
    let reply;
    let settled = false;
    try {
      reply = await completeCapture(entry);
      settled = true;
    } finally {
      // Every exit clears the record: a completed capture, a kept browser copy or an
      // unexpected failure (which then resumes the download before dropping it).
      await resolvePendingCapture(downloadId, { resume: !settled });
    }
    if (!reply.ok) {
      return reply;
    }
    return { ok: true, data: Object.assign({ accepted: true, download_id: downloadId }, reply.data) };
  }

  function setConfirmTimeoutMs(ms) {
    const numeric = Number(ms);
    if (Number.isFinite(numeric) && numeric > 0) {
      confirmTimeoutMs = numeric;
    }
    return confirmTimeoutMs;
  }

  /** Overridable bounded waits (tests only); production keeps the constants. */
  function setCaptureTiming(overrides) {
    const input = overrides && typeof overrides === 'object' ? overrides : {};
    for (const key of ['ownershipTimeoutMs', 'pollIntervalMs', 'windowTimeoutMs']) {
      const numeric = Number(input[key]);
      if (Number.isFinite(numeric) && numeric > 0) {
        captureTiming[key] = numeric;
      }
    }
    return Object.assign({}, captureTiming);
  }

  async function considerDownload(downloadItem) {
    if (!downloadItem || !downloadItem.id || !downloadItem.url) {
      return false;
    }
    if (handledDownloadIds.has(downloadItem.id)) {
      return false;
    }
    trackHandledId(downloadItem.id);

    const url = downloadItem.url;
    const parsed = PROTOCOL.parseHttpUrl(url);
    const isMagnet = url.startsWith('magnet:');
    if (!parsed && !isMagnet) {
      return false;
    }

    if (parsed) {
      const scheme = parsed.protocol;
      if (scheme === 'blob:' || scheme === 'data:' || scheme === 'file:' || scheme === 'chrome-extension:' || scheme === 'moz-extension:') {
        return false;
      }
    }

    if (downloadItem.byExtensionId && downloadItem.byExtensionId === extensionId()) {
      return false;
    }

    const settings = await getCachedSettings();
    if (!settings) {
      return false;
    }

    if (settings.capture_enabled === false) {
      return false;
    }

    if (parsed && isHostDenylisted(url, settings.domain_denylist)) {
      return false;
    }

    const isTorrentFile = PROTOCOL.parseTorrentUrl(url);
    if (isTorrentFile) {
      if (settings.capture_torrents === false) {
        return false;
      }
    } else if (isMagnet) {
      if (settings.capture_magnet === false) {
        return false;
      }
    } else {
      const filename = downloadItem.filename || downloadItem.url;
      if (!matchExtension(filename, settings.capture_extensions)) {
        return false;
      }
    }

    // Persist the pending capture and arm the watchdog before anything is paused:
    // an erased download cannot be resumed, and a paused one may outlive this
    // worker. Without a record and a live alarm the download stays untouched.
    const confirm = captureConfirmEnabled(settings);
    const entry = {
      downloadId: downloadItem.id,
      url: url,
      filename: typeof downloadItem.filename === 'string' ? downloadItem.filename : '',
      referrer: typeof downloadItem.referrer === 'string' ? downloadItem.referrer : '',
      folder: captureFolder(settings),
      // Without a window to report the outcome, the notification is the only way to
      // tell the user the browser copy was kept.
      inform: !confirm,
    };

    if (!(await armPendingCapture(entry))) {
      return false;
    }

    // Pause second: an erased download cannot be resumed, so the browser copy must
    // survive until the daemon owns the task.
    const paused = await pauseDownload(downloadItem.id);
    if (!paused) {
      await resolvePendingCapture(downloadItem.id);
      return false;
    }

    if (!confirm) {
      let settled = false;
      try {
        await completeCapture(entry);
        settled = true;
      } finally {
        await resolvePendingCapture(downloadItem.id, { resume: !settled });
      }
      return true;
    }

    const opened = await openConfirmWindow(entry);
    if (!opened) {
      await resolvePendingCapture(downloadItem.id);
      return false;
    }
    return true;
  }

  /** Live path: `downloads.onCreated` is only the trigger, the gate is shared. */
  function onDownloadCreated(downloadItem) {
    return considerDownload(downloadItem);
  }

  if (API && API.downloads && API.downloads.onCreated && typeof API.downloads.onCreated.addListener === 'function') {
    API.downloads.onCreated.addListener(onDownloadCreated);
  }

  if (API && API.windows && API.windows.onRemoved && typeof API.windows.onRemoved.addListener === 'function') {
    API.windows.onRemoved.addListener(onWindowRemoved);
  }

  function setupContextMenus() {
    if (API && API.contextMenus && typeof API.contextMenus.create === 'function') {
      API.contextMenus.create({
        id: 'segmenta-add-torrent',
        title: API.getMessage ? (API.getMessage('contextMenuAddTorrent') || 'Anadir a Segmenta') : 'Anadir a Segmenta',
        contexts: ['link'],
        targetUrlPatterns: ['magnet:*', 'http://*/*.torrent', 'https://*/*.torrent'],
      });
    }
  }

  // Context menus outlive a suspended worker, so creating the shortcut on every boot
  // is what keeps it present in both browsers.
  setupContextMenus();

  if (API && API.contextMenus && API.contextMenus.onClicked && typeof API.contextMenus.onClicked.addListener === 'function') {
    API.contextMenus.onClicked.addListener((info) => {
      if (info.menuItemId === 'segmenta-add-torrent' && info.linkUrl) {
        callHost(() => PROTOCOL.buildTorrentAdd({ url: info.linkUrl }));
      }
    });
  }

  function handle(message, sender) {
    const check = VALIDATE.validateInbound(message, sender, {
      extensionId: extensionId(),
      allowStreams: true,
      allowlist: STREAM_ALLOWLIST,
    });
    if (!check.ok) {
      return Promise.resolve({ ok: false, error: check.error });
    }
    const value = check.value;
    switch (value.type) {
      case 'segmenta.probe':
        return callHost(() =>
          PROTOCOL.buildProbe({
            url: value.url,
            kind: value.kind,
            limit: value.limit,
            allowlist: STREAM_ALLOWLIST,
          }),
        );
      case 'segmenta.add':
        return callHostAndAnnounce(() =>
          PROTOCOL.buildAdd({
            url: value.url,
            mode: value.mode,
            formatId: value.formatId,
            height: value.height,
            playlistLimit: value.playlistLimit,
            allowlist: STREAM_ALLOWLIST,
          }),
        );
      case 'segmenta.status':
        return callHost(() => PROTOCOL.buildStatus({ taskId: value.taskId }));
      // Focus an already existing task (S9): `callHost` only, never the announce
      // path, so nothing is added again and no toast is shown.
      case 'segmenta.open':
        return callHost(() => PROTOCOL.buildSystemOpen({ taskId: value.taskId }));
      case 'segmenta.capture':
        return callHostAndAnnounce(() =>
          PROTOCOL.buildDownloadAdd({
            url: value.url,
            filename: value.filename,
            referrer: value.referrer,
          }),
        );
      case 'segmenta.torrent.add':
        return callHostAndAnnounce(() => PROTOCOL.buildTorrentAdd({ url: value.url }));
      // Answer from confirm.html (S10): the window carries the paused download, so
      // the decision never depends on state a worker restart could have dropped.
      case 'segmenta.capture.decide':
        return decideCapture(value);
      default:
        return Promise.resolve(failure('ERR_INVALID_TYPE', 'message type is not allowed'));
    }
  }

  function onMessage(message, sender, sendResponse) {
    let promise;
    try {
      promise = handle(message, sender);
    } catch (err) {
      sendResponse(failure('ERR_INVALID_MESSAGE', 'message could not be handled'));
      return false;
    }
    promise.then(
      (result) => sendResponse(result),
      () => sendResponse(failure('ERR_INTERNAL', 'internal error')),
    );
    return true;
  }

  if (API && typeof API.addListener === 'function') {
    API.addListener(onMessage);
  }

  // Exported for debugging/tests only; the service worker never exposes these on
  // the page.
  global.SegmentaBackground = {
    HOST_NAME: HOST_NAME,
    NATIVE_TIMEOUT_MS: NATIVE_TIMEOUT_MS,
    NOTIFICATION_ID: NOTIFICATION_ID,
    SETTINGS_CACHE_VERSION: SETTINGS_CACHE_VERSION,
    CONFIRM_PAGE: CONFIRM_PAGE,
    CAPTURE_CONFIRM_TIMEOUT_MS: CAPTURE_CONFIRM_TIMEOUT_MS,
    TASK_OWNERSHIP_TIMEOUT_MS: TASK_OWNERSHIP_TIMEOUT_MS,
    TASK_POLL_INTERVAL_MS: TASK_POLL_INTERVAL_MS,
    CONFIRM_WINDOW_TIMEOUT_MS: CONFIRM_WINDOW_TIMEOUT_MS,
    PENDING_CAPTURE_SCHEMA: PENDING_CAPTURE_SCHEMA,
    PENDING_CAPTURE_KEY: PENDING_CAPTURE_KEY,
    CAPTURE_WATCHDOG_ALARM: CAPTURE_WATCHDOG_ALARM,
    CAPTURE_WATCHDOG_PERIOD_MINUTES: CAPTURE_WATCHDOG_PERIOD_MINUTES,
    CAPTURE_KEPT_NOTIFICATION_ID: CAPTURE_KEPT_NOTIFICATION_ID,
    MAX_RECONCILE_DOWNLOADS: MAX_RECONCILE_DOWNLOADS,
    RECONCILE_THROTTLE_MS: RECONCILE_THROTTLE_MS,
    handle: handle,
    hostFailure: hostFailure,
    onDownloadCreated: onDownloadCreated,
    considerDownload: considerDownload,
    reconcileInProgressDownloads: reconcileInProgressDownloads,
    reconcileThenSweep: reconcileThenSweep,
    runReconcile: runReconcile,
    getCachedSettings: getCachedSettings,
    matchExtension: matchExtension,
    isHostDenylisted: isHostDenylisted,
    parentDirectory: parentDirectory,
    destinationFolder: destinationFolder,
    taskAddedBody: taskAddedBody,
    showTaskNotification: showTaskNotification,
    showCaptureKeptNotification: showCaptureKeptNotification,
    announceAddedTask: announceAddedTask,
    callHostAndAnnounce: callHostAndAnnounce,
    captureFolder: captureFolder,
    captureConfirmEnabled: captureConfirmEnabled,
    buildConfirmUrl: buildConfirmUrl,
    openConfirmWindow: openConfirmWindow,
    completeCapture: completeCapture,
    waitForTaskOwnership: waitForTaskOwnership,
    taskOwnsBytes: taskOwnsBytes,
    dropCapturedTask: dropCapturedTask,
    decideCapture: decideCapture,
    onWindowRemoved: onWindowRemoved,
    armPendingCapture: armPendingCapture,
    resolvePendingCapture: resolvePendingCapture,
    sweepPendingCaptures: sweepPendingCaptures,
    onCaptureWatchdogAlarm: onCaptureWatchdogAlarm,
    parsePendingRecord: parsePendingRecord,
    confirmWindowStillOpen: confirmWindowStillOpen,
    createCaptureWatchdogAlarm: createCaptureWatchdogAlarm,
    clearCaptureWatchdogAlarm: clearCaptureWatchdogAlarm,
    setConfirmTimeoutMs: setConfirmTimeoutMs,
    setCaptureTiming: setCaptureTiming,
  };
})(typeof globalThis !== 'undefined' ? globalThis : this);
