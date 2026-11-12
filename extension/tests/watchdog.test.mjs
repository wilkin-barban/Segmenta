// Suspension-safe capture watchdog (S10).
//
// An MV3 service worker is suspended at will, so an in-memory timer can never be
// the only guard for a paused browser download. Every pause must be preceded by a
// persisted pending record plus an `alarms` watchdog, and a fresh worker must be
// able to rehydrate that record and finish the job.
//
// A "suspended worker" is modelled by loading the background module twice against
// the same fake `storage.local`: the first instance pauses and opens the window,
// the second one is fresh and is the only listener that ever sees the alarm.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { extFile } from './helpers.mjs';

const EXTENSION_ID = 'abcdefghijklmnopabcdefghijklmnop';
const ZIP_URL = 'https://downloads.example.org/tools/archive.zip';
// Kept in step with the background cache constant; the suite fails loudly when
// the two drift.
const CACHE_VERSION = '0.1.3';
// Versioned storage key plus an explicit schema version inside every payload.
const PENDING_KEY = 'segmenta_pending_captures_v1';
const PENDING_SCHEMA = 1;
const ALARM_NAME = 'segmenta-capture-watchdog';
const SENDER = {
  id: EXTENSION_ID,
  url: `chrome-extension://${EXTENSION_ID}/confirm.html`,
  origin: `chrome-extension://${EXTENSION_ID}`,
};

let windowSequence = 900;
let instanceSequence = 0;

function settingsWith(extra) {
  return Object.assign({
    capture_enabled: true,
    capture_torrents: true,
    capture_magnet: true,
    capture_extensions: ['zip'],
    domain_denylist: [],
    download_dir: '/home/u/Descargas',
    capture_confirm: true,
  }, extra || {});
}

function item(id) {
  return {
    id: id,
    url: ZIP_URL,
    filename: 'archive.zip',
    referrer: 'https://downloads.example.org/tools/',
  };
}

function setupFakeBrowser(settings, options = {}) {
  const timeline = [];
  const paused = [];
  const resumed = [];
  const canceled = [];
  const erased = [];
  const createdWindows = [];
  const removedWindows = [];
  const nativeCalls = [];
  const notifications = [];
  // Windows the browser still reports through windows.get.
  const openWindows = new Set();
  const alarms = { created: [], cleared: [], active: new Set(), listeners: [] };
  // Every downloads.search query the worker sent: the cold-start reconciliation must
  // ask the browser for state instead of waiting for an event that already happened.
  const searches = [];
  // A valid settings cache: the capture path must never depend on a host round
  // trip that a `storage.local.set` failure would otherwise mask.
  const storageData = {
    segmenta_settings: settings,
    segmenta_settings_version: CACHE_VERSION,
  };
  const failing = Array.isArray(options.failingMethods) ? options.failingMethods : [];
  let nextTaskId = 0;

  function defaultTaskId() {
    nextTaskId += 1;
    return 'task-' + String(nextTaskId).padStart(6, '0');
  }

  const chrome = {
    runtime: {
      id: EXTENSION_ID,
      lastError: undefined,
      onStartup: {
        listeners: [],
        addListener(fn) { this.listeners.push(fn); },
      },
      onInstalled: {
        listeners: [],
        addListener(fn) { this.listeners.push(fn); },
      },
      getURL(relative) {
        return `chrome-extension://${EXTENSION_ID}/${String(relative).replace(/^\/+/, '')}`;
      },
      sendNativeMessage(host, message, callback) {
        nativeCalls.push({ host, message });
        timeline.push('native:' + message.method);
        if (failing.indexOf(message.method) !== -1) {
          callback({ jsonrpc: '2.0', id: message.id, error: { code: -32601, message: 'method not found' } });
          return;
        }
        const result = message.method === 'system.open'
          ? { opened: true }
          : message.method === 'settings.get'
            ? settings
            : message.method === 'media.status'
              ? { task_id: message.params.task_id, status: 'downloading', downloaded_bytes: 4096 }
              : message.method === 'download.cancel'
                ? { task_id: message.params.task_id, status: 'cancelled' }
                : { task_id: defaultTaskId(), status: 'downloading' };
        callback({ jsonrpc: '2.0', id: message.id, result: result });
      },
    },
    storage: {
      local: {
        get(keys, cb) {
          const res = {};
          if (Array.isArray(keys)) {
            for (const key of keys) {
              if (storageData[key] !== undefined) res[key] = storageData[key];
            }
          }
          cb(res);
        },
        set(items, cb) {
          timeline.push('storage:set');
          if (options.storageSetFails) {
            throw new Error('storage.local.set failed');
          }
          Object.assign(storageData, items);
          if (cb) cb();
        },
      },
    },
    downloads: {
      onCreated: {
        listeners: [],
        addListener(fn) { this.listeners.push(fn); },
        removeListener(fn) { this.listeners = this.listeners.filter((l) => l !== fn); },
      },
      pause(id, cb) {
        paused.push(id);
        timeline.push('pause:' + id);
        if (cb) cb();
      },
      resume(id, cb) {
        resumed.push(id);
        timeline.push('resume:' + id);
        // Lets a test arm another capture in the middle of a sweep.
        if (typeof options.onResume === 'function') {
          options.onResume(id, storageData);
        }
        if (cb) cb();
      },
      cancel(id, cb) {
        canceled.push(id);
        timeline.push('cancel:' + id);
        if (cb) cb();
      },
      erase(query, cb) {
        erased.push(query);
        timeline.push('erase:' + query.id);
        if (cb) cb();
      },
      // What the browser already had in progress before this worker existed.
      search(query, cb) {
        searches.push(query);
        timeline.push('search');
        if (options.searchFails) {
          chrome.runtime.lastError = { message: 'downloads search failed' };
          try {
            if (cb) cb(undefined);
          } finally {
            chrome.runtime.lastError = undefined;
          }
          return undefined;
        }
        const list = typeof options.inProgress === 'function'
          ? options.inProgress(query)
          : options.inProgress;
        if (cb) cb(Array.isArray(list) ? list.slice() : []);
        return undefined;
      },
    },
    windows: {
      onRemoved: {
        listeners: [],
        addListener(fn) { this.listeners.push(fn); },
      },
      create(data, cb) {
        timeline.push('window:create');
        windowSequence += 1;
        const id = windowSequence;
        createdWindows.push({ id: id, url: data && data.url });
        openWindows.add(id);
        if (cb) cb({ id: id });
        return undefined;
      },
      get(id, cb) {
        timeline.push('window:get:' + id);
        if (openWindows.has(id)) {
          if (cb) cb({ id: id });
          return undefined;
        }
        chrome.runtime.lastError = { message: 'No window with id: ' + id };
        try {
          if (cb) cb(undefined);
        } finally {
          chrome.runtime.lastError = undefined;
        }
        return undefined;
      },
      remove(id, cb) {
        removedWindows.push(id);
        openWindows.delete(id);
        timeline.push('window:remove:' + id);
        if (cb) cb();
      },
    },
    alarms: {
      onAlarm: {
        listeners: [],
        addListener(fn) { this.listeners.push(fn); },
      },
      create(name, info, cb) {
        timeline.push('alarms:create');
        if (options.alarmsCreateFails) {
          throw new Error('alarms.create failed');
        }
        alarms.created.push({ name: name, info: info });
        alarms.active.add(name);
        if (cb) cb();
        // Firefox resolves `alarms.create` from its return value, never a promise.
        return options.alarmsPromiseStyle ? Promise.resolve() : undefined;
      },
      clear(name, cb) {
        timeline.push('alarms:clear');
        alarms.cleared.push(name);
        alarms.active.delete(name);
        if (cb) cb(true);
        return options.alarmsPromiseStyle ? Promise.resolve(true) : undefined;
      },
    },
    contextMenus: {
      create(props, cb) { if (cb) cb(); },
      onClicked: { listeners: [], addListener(fn) { this.listeners.push(fn); } },
    },
    notifications: {
      create(idOrOptions, optionsOrCallback, maybeCallback) {
        const notificationId = typeof idOrOptions === 'string' ? idOrOptions : '';
        const createOptions = typeof idOrOptions === 'string' ? optionsOrCallback : idOrOptions;
        const callback = typeof optionsOrCallback === 'function' ? optionsOrCallback : maybeCallback;
        notifications.push({ id: notificationId, title: createOptions.title, message: createOptions.message });
        timeline.push('notify');
        if (callback) callback(notificationId);
        return Promise.resolve(notificationId);
      },
    },
  };

  globalThis.chrome = chrome;

  return {
    timeline,
    paused,
    resumed,
    canceled,
    erased,
    createdWindows,
    removedWindows,
    nativeCalls,
    notifications,
    alarms,
    storageData,
    searches,
    /** Persisted pending records, as a list (never a raw scalar). */
    pendingRecords() {
      const raw = storageData[PENDING_KEY];
      if (raw === undefined || raw === null) {
        return [];
      }
      return Array.isArray(raw) ? raw : [raw];
    },
    /** The browser forgot the window without any event (post-restart case). */
    forgetWindow(windowId) {
      openWindows.delete(windowId);
    },
    /** Fires the alarm on the newest worker only: that is the fresh instance. */
    fireAlarm() {
      const listeners = chrome.alarms.onAlarm.listeners;
      assert.equal(listeners.length > 0, true, 'no worker registered an alarm listener');
      return listeners[listeners.length - 1]({ name: ALARM_NAME, scheduledTime: Date.now() });
    },
    fireOtherAlarm() {
      const listeners = chrome.alarms.onAlarm.listeners;
      return listeners[listeners.length - 1]({ name: 'unrelated-alarm', scheduledTime: Date.now() });
    },
    fireStartup() {
      const listeners = chrome.runtime.onStartup.listeners;
      assert.equal(listeners.length > 0, true, 'no worker registered an onStartup listener');
      return listeners[listeners.length - 1]();
    },
    fireInstalled() {
      const listeners = chrome.runtime.onInstalled.listeners;
      assert.equal(listeners.length > 0, true, 'no worker registered an onInstalled listener');
      return listeners[listeners.length - 1]();
    },
    fireWindowRemoved(windowId) {
      const listeners = chrome.windows.onRemoved.listeners;
      assert.equal(listeners.length > 0, true, 'no worker registered a window listener');
      // A removed window can no longer be reported by windows.get either.
      openWindows.delete(windowId);
      return listeners[listeners.length - 1](windowId);
    },
  };
}

/**
 * Loads another background instance: the same bytes, a fresh worker.
 *
 * `background.js` and its shared scripts are plain scripts, so Node loads them
 * through the CommonJS cache, which ignores query strings. Evaluating the sources
 * again in this context is the faithful model of a worker that was suspended and
 * later re-created, as `importScripts` would: the adapter, every in-memory map and
 * every timer start empty while `storage.local` survives.
 */
async function bootWorker() {
  instanceSequence += 1;
  const sources = [
    ['shared/browser-api.js', 'shared/browser-api.js'],
    ['shared/protocol.js', 'shared/protocol.js'],
    ['shared/validate.js', 'shared/validate.js'],
    ['chromium/background.js', 'background.js'],
  ];
  for (const [file, label] of sources) {
    const source = readFileSync(extFile(...file.split('/')), 'utf8');
    vm.runInThisContext(source, { filename: `${label}#worker-${instanceSequence}` });
  }
  assert.equal(typeof globalThis.SegmentaBackground, 'object');
  return globalThis.SegmentaBackground;
}

// A suspended worker keeps no timers: while this is installed the in-memory
// confirmation timer is never scheduled, so only persisted state can survive.
function freezeTimers() {
  const original = globalThis.setTimeout;
  globalThis.setTimeout = () => ({ unref() {}, ref() {} });
  return () => {
    globalThis.setTimeout = original;
  };
}

async function captureWithFrozenTimers(bg, id) {
  const restore = freezeTimers();
  try {
    await bg.onDownloadCreated(item(id));
  } finally {
    restore();
  }
}

function tick(ms) {
  return new Promise((resolve) => globalThis.setTimeout(resolve, ms === undefined ? 0 : ms));
}

async function accept(bg, downloadId) {
  return bg.handle(
    { type: 'segmenta.capture.decide', downloadId: downloadId, decision: 'accept', url: ZIP_URL },
    SENDER,
  );
}

function indexOf(timeline, expected) {
  const index = timeline.indexOf(expected);
  assert.notEqual(index, -1, `missing ${expected} in ${JSON.stringify(timeline)}`);
  return index;
}

export default function suite({ test }) {
  test('watchdog: a fresh worker resumes a download whose deadline passed', async () => {
    const env = setupFakeBrowser(settingsWith());
    const first = await bootWorker();
    first.setConfirmTimeoutMs(5);
    await captureWithFrozenTimers(first, 701);
    first.setConfirmTimeoutMs(first.CAPTURE_CONFIRM_TIMEOUT_MS);

    assert.deepEqual(env.paused, [701], 'the download is paused once');
    assert.equal(env.alarms.active.has(ALARM_NAME), true, 'the watchdog alarm must be live before the pause');
    const records = env.pendingRecords();
    assert.equal(records.length, 1);
    assert.equal(records[0].v, PENDING_SCHEMA, 'the pending payload carries its own schema version');
    assert.equal(records[0].downloadId, 701);
    assert.equal(records[0].url, ZIP_URL);
    assert.equal(records[0].filename, 'archive.zip');
    assert.equal(records[0].referrer, 'https://downloads.example.org/tools/');
    assert.equal(Number.isInteger(records[0].windowId), true, 'the record names the confirmation window');
    assert.equal(Number.isFinite(records[0].deadline), true, 'the record carries its own deadline');

    await tick(30);
    // The worker is gone; only storage and the alarm survive.
    await bootWorker();
    await env.fireAlarm();

    assert.deepEqual(env.resumed, [701], 'the expired capture is resumed by the fresh worker');
    assert.deepEqual(env.pendingRecords(), [], 'the record is removed');
    assert.equal(env.alarms.active.size, 0, 'no alarm remains');
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
  });

  test('watchdog: an alarm before the deadline leaves a live window alone', async () => {
    const env = setupFakeBrowser(settingsWith());
    const first = await bootWorker();
    await captureWithFrozenTimers(first, 702);
    const windowId = env.createdWindows[0].id;
    env.alarms.created.length = 0;

    await bootWorker();
    await env.fireAlarm();

    assert.deepEqual(env.resumed, [], 'a live confirmation window keeps the download paused');
    assert.deepEqual(env.canceled, []);
    assert.equal(env.pendingRecords().length, 1, 'the record is kept');
    assert.equal(env.pendingRecords()[0].windowId, windowId);
    assert.equal(env.alarms.active.has(ALARM_NAME), true, 'the watchdog is re-armed while a record remains');
    assert.equal(env.alarms.created.length, 1);
  });

  test('watchdog: an alarm resumes the download when the window is gone', async () => {
    const env = setupFakeBrowser(settingsWith());
    const first = await bootWorker();
    await captureWithFrozenTimers(first, 703);
    const windowId = env.createdWindows[0].id;
    env.forgetWindow(windowId);

    await bootWorker();
    await env.fireAlarm();

    assert.deepEqual(env.resumed, [703]);
    assert.deepEqual(env.pendingRecords(), []);
    assert.equal(env.alarms.active.size, 0);
  });

  test('watchdog: an unreadable record is dropped and its download resumed', async () => {
    const env = setupFakeBrowser(settingsWith());
    env.storageData[PENDING_KEY] = [
      { v: PENDING_SCHEMA, downloadId: 704, url: ZIP_URL, deadline: Date.now() - 1000, windowId: null },
      { v: 99, downloadId: 705, url: ZIP_URL, deadline: Date.now() + 60000, windowId: 1 },
      { v: PENDING_SCHEMA, downloadId: 'not-a-number', url: ZIP_URL },
      'garbage',
    ];
    await bootWorker();
    await env.fireAlarm();

    assert.deepEqual(env.resumed.sort(), [704, 705], 'every salvageable paused download is resumed');
    assert.deepEqual(env.pendingRecords(), [], 'no unreadable record survives the sweep');
    assert.equal(env.alarms.active.size, 0, 'the watchdog is cleared once no record remains');
  });

  test('watchdog: a record without a window id yet is kept, not resumed', async () => {
    // Number(null) is 0 and an integer: reading it as a window id would make a live
    // confirmation window look gone and resume the download before its deadline.
    const env = setupFakeBrowser(settingsWith());
    env.storageData[PENDING_KEY] = [
      { v: PENDING_SCHEMA, downloadId: 710, url: ZIP_URL, deadline: Date.now() + 60000, windowId: null },
    ];
    await bootWorker();
    await env.fireAlarm();

    assert.deepEqual(env.resumed, [], 'a record with an unrecorded window is not resumed');
    assert.equal(env.pendingRecords().length, 1, 'the record survives until its deadline');
    assert.equal(env.alarms.active.size, 1, 'and its watchdog stays armed');
  });

  test('watchdog: a capture armed while the sweep awaits is never dropped', async () => {
    // The sweep resumes first and writes afterwards. Writing the list it read before
    // those awaits would erase a capture armed in between, leaving it paused with no
    // watchdog at all.
    const armed = { v: PENDING_SCHEMA, downloadId: 721, url: ZIP_URL, deadline: Date.now() + 60000, windowId: null };
    const env = setupFakeBrowser(settingsWith(), {
      onResume: (id, storageData) => {
        const current = Array.isArray(storageData[PENDING_KEY]) ? storageData[PENDING_KEY] : [];
        storageData[PENDING_KEY] = current
          .filter((record) => record && record.downloadId !== id)
          .concat([armed]);
      },
    });
    env.storageData[PENDING_KEY] = [
      { v: PENDING_SCHEMA, downloadId: 720, url: ZIP_URL, deadline: Date.now() - 1000, windowId: 900 },
    ];
    await bootWorker();
    await env.fireAlarm();

    assert.deepEqual(env.resumed, [720], 'only the expired download is resumed');
    const ids = env.pendingRecords().map((record) => record.downloadId);
    assert.deepEqual(ids, [721], 'the capture armed during the sweep keeps its record');
    assert.equal(env.alarms.active.size, 1, 'and its watchdog is not cleared');
  });

  test('watchdog: a fresh browser run heals a leftover record on startup', async () => {
    const env = setupFakeBrowser(settingsWith());
    env.storageData[PENDING_KEY] = [
      { v: PENDING_SCHEMA, downloadId: 706, url: ZIP_URL, deadline: Date.now() - 5000, windowId: 4242 },
    ];
    await bootWorker();
    await env.fireStartup();

    assert.deepEqual(env.resumed, [706]);
    assert.deepEqual(env.pendingRecords(), []);
    assert.equal(env.alarms.active.size, 0);
  });

  test('watchdog: an unrelated alarm never touches the pending captures', async () => {
    const env = setupFakeBrowser(settingsWith());
    const first = await bootWorker();
    await captureWithFrozenTimers(first, 707);

    await bootWorker();
    await env.fireOtherAlarm();

    assert.deepEqual(env.resumed, []);
    assert.equal(env.pendingRecords().length, 1);
  });

  test('watchdog: nothing is paused when storage.local.set fails', async () => {
    const env = setupFakeBrowser(settingsWith(), { storageSetFails: true });
    const bg = await bootWorker();

    await bg.onDownloadCreated(item(708));

    assert.deepEqual(env.paused, [], 'the browser download is never paused without a persisted record');
    assert.deepEqual(env.createdWindows, []);
    assert.deepEqual(env.nativeCalls, []);
    assert.equal(env.alarms.active.size, 0, 'no orphan watchdog');
  });

  test('watchdog: nothing is paused when alarms.create fails', async () => {
    const env = setupFakeBrowser(settingsWith(), { alarmsCreateFails: true });
    const bg = await bootWorker();

    await bg.onDownloadCreated(item(709));

    assert.deepEqual(env.paused, [], 'never pause without a live watchdog');
    assert.deepEqual(env.createdWindows, []);
    assert.deepEqual(env.nativeCalls, []);
    assert.deepEqual(env.pendingRecords(), [], 'the record written before arming is removed again');
  });

  test('watchdog: the auto capture path arms the watchdog before pausing', async () => {
    const env = setupFakeBrowser(settingsWith({ capture_confirm: false }));
    const bg = await bootWorker();

    await bg.onDownloadCreated(item(710));

    assert.deepEqual(env.paused, [710]);
    assert.equal(
      indexOf(env.timeline, 'alarms:create') < indexOf(env.timeline, 'pause:710'),
      true,
      `the watchdog must exist at pause time: ${JSON.stringify(env.timeline)}`,
    );
    // The automatic capture finished, so the record and its alarm are gone.
    assert.deepEqual(env.pendingRecords(), []);
    assert.equal(env.alarms.active.size, 0);
    assert.deepEqual(env.canceled, [710]);
  });

  test('watchdog: the confirm path arms the watchdog before pausing', async () => {
    const env = setupFakeBrowser(settingsWith());
    const bg = await bootWorker();

    await captureWithFrozenTimers(bg, 711);

    assert.deepEqual(env.paused, [711]);
    assert.equal(
      indexOf(env.timeline, 'alarms:create') < indexOf(env.timeline, 'pause:711'),
      true,
      `the watchdog must exist at pause time: ${JSON.stringify(env.timeline)}`,
    );
    // Suspended before the answer: the record must still be there.
    assert.equal(env.pendingRecords().length, 1);
    assert.equal(env.alarms.active.has(ALARM_NAME), true);
    await bg.handle(
      { type: 'segmenta.capture.decide', downloadId: 711, decision: 'reject', url: ZIP_URL },
      SENDER,
    );
  });

  test('watchdog: accept clears the record and the alarm', async () => {
    const env = setupFakeBrowser(settingsWith());
    const bg = await bootWorker();

    await captureWithFrozenTimers(bg, 712);
    assert.equal(env.pendingRecords().length, 1, 'the pause is recorded before the answer');
    assert.equal(env.alarms.active.has(ALARM_NAME), true);
    const reply = await accept(bg, 712);

    assert.equal(reply.ok, true);
    assert.deepEqual(env.canceled, [712]);
    assert.deepEqual(env.pendingRecords(), [], 'an accepted capture leaves no record');
    assert.equal(env.alarms.active.size, 0, 'an accepted capture leaves no alarm');
  });

  test('watchdog: reject clears the record and the alarm', async () => {
    const env = setupFakeBrowser(settingsWith());
    const bg = await bootWorker();

    await captureWithFrozenTimers(bg, 713);
    assert.equal(env.pendingRecords().length, 1, 'the pause is recorded before the answer');
    assert.equal(env.alarms.active.has(ALARM_NAME), true);
    const reply = await bg.handle(
      { type: 'segmenta.capture.decide', downloadId: 713, decision: 'reject', url: ZIP_URL },
      SENDER,
    );

    assert.equal(reply.ok, true);
    assert.deepEqual(env.resumed, [713]);
    assert.deepEqual(env.pendingRecords(), []);
    assert.equal(env.alarms.active.size, 0);
  });

  test('watchdog: closing the window clears the record and the alarm', async () => {
    const env = setupFakeBrowser(settingsWith());
    const first = await bootWorker();
    await captureWithFrozenTimers(first, 714);
    const windowId = env.createdWindows[0].id;

    await bootWorker();
    env.fireWindowRemoved(windowId);
    await tick(10);

    assert.deepEqual(env.resumed, [714], 'a fresh worker heals a closed window immediately');
    assert.deepEqual(env.pendingRecords(), []);
    assert.equal(env.alarms.active.size, 0);
  });

  test('watchdog: a host failure during accept clears the record and keeps the file', async () => {
    const env = setupFakeBrowser(settingsWith(), { failingMethods: ['download.add'] });
    const bg = await bootWorker();

    await captureWithFrozenTimers(bg, 715);
    assert.equal(env.pendingRecords().length, 1, 'the pause is recorded before the answer');
    assert.equal(env.alarms.active.has(ALARM_NAME), true);
    const reply = await accept(bg, 715);

    assert.equal(reply.ok, false);
    assert.deepEqual(env.resumed, [715], 'the browser copy is resumed before the record goes');
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
    assert.deepEqual(env.pendingRecords(), []);
    assert.equal(env.alarms.active.size, 0);
  });

  // ---- Cold-start reconciliation (R1) ----------------------------------------
  // A download the browser started while no worker existed never fires
  // `downloads.onCreated`, so a fresh worker asks for what is still in progress and
  // routes it through the very same capture path a live download takes.

  function inProgressItem(id) {
    return Object.assign(item(id), { state: 'in_progress' });
  }

  test('reconcile: a download the browser already had is captured at startup', async () => {
    const env = setupFakeBrowser(settingsWith(), { inProgress: [inProgressItem(801)] });
    // The worker that never saw the download, then the restarted one that takes over.
    await bootWorker();
    const bg = await bootWorker();
    await env.fireStartup();

    assert.deepEqual(env.searches, [{ state: 'in_progress' }], 'the scan asks for in-progress downloads only');
    assert.deepEqual(env.paused, [801], 'the reconciled download is paused like a live one');
    assert.equal(env.createdWindows.length, 1, 'the normal confirmation window is opened');
    assert.match(env.createdWindows[0].url, /[?&]download=801(&|$)/);
    assert.equal(env.alarms.active.has(ALARM_NAME), true, 'the watchdog is armed before the pause');
    const records = env.pendingRecords();
    assert.equal(records.length, 1, 'the pause is recorded for a worker that never sees the answer');
    assert.equal(records[0].downloadId, 801);

    // The answer arrives in a window a restarted worker can resolve: same rules.
    const reply = await accept(bg, 801);

    assert.equal(reply.ok, true);
    assert.deepEqual(env.canceled, [801], 'the browser copy is discarded only after the add');
    assert.deepEqual(env.erased, [{ id: 801 }]);
    assert.deepEqual(env.pendingRecords(), []);
    assert.equal(env.alarms.active.size, 0);
  });

  test('reconcile: a non-matching extension, a denylisted host and a disabled capture are ignored', async () => {
    const cases = [
      ['a non-matching extension', settingsWith({ capture_extensions: ['mp4'] })],
      ['a denylisted host', settingsWith({ domain_denylist: ['example.org'] })],
      ['a disabled capture setting', settingsWith({ capture_enabled: false })],
    ];
    for (const [label, settings] of cases) {
      const env = setupFakeBrowser(settings, { inProgress: [inProgressItem(802)] });
      await bootWorker();
      await env.fireStartup();

      assert.equal(env.searches.length, 1, `${label}: the scan still runs`);
      assert.deepEqual(env.paused, [], `${label}: nothing is paused`);
      assert.deepEqual(env.createdWindows, [], `${label}: no confirmation window`);
      assert.deepEqual(env.nativeCalls, [], `${label}: nothing reaches the host`);
      assert.deepEqual(env.pendingRecords(), [], `${label}: no record is written`);
      assert.equal(env.alarms.active.size, 0, `${label}: no orphan watchdog`);
    }
  });

  test('reconcile: an id a live event already handled is not captured twice', async () => {
    const env = setupFakeBrowser(settingsWith({ capture_confirm: false }), {
      inProgress: [inProgressItem(803)],
    });
    const bg = await bootWorker();

    await bg.onDownloadCreated(item(803));
    await env.fireStartup();

    assert.deepEqual(env.paused, [803], 'the download is paused exactly once');
    assert.deepEqual(env.canceled, [803]);
    assert.deepEqual(env.nativeCalls.map((call) => call.message.method), [
      'download.add',
      'media.status',
      'system.open',
    ]);
  });

  test('reconcile: an id with a persisted pending record is not captured twice', async () => {
    const env = setupFakeBrowser(settingsWith(), { inProgress: [inProgressItem(804)] });
    env.storageData[PENDING_KEY] = [
      {
        v: PENDING_SCHEMA,
        downloadId: 804,
        url: ZIP_URL,
        filename: 'archive.zip',
        referrer: '',
        windowId: null,
        deadline: Date.now() + 60000,
      },
    ];
    await bootWorker();
    await env.fireStartup();

    assert.deepEqual(env.paused, [], 'an already paused download is never paused again');
    assert.deepEqual(env.createdWindows, [], 'no second confirmation window');
    assert.equal(env.pendingRecords().length, 1, 'the existing record is left to the sweep');
    assert.equal(env.alarms.active.has(ALARM_NAME), true);
  });

  test('reconcile: a downloads.search failure leaves everything untouched and does not throw', async () => {
    const env = setupFakeBrowser(settingsWith(), {
      inProgress: [inProgressItem(805)],
      searchFails: true,
    });
    await bootWorker();

    // A rejected scan must resolve, never reject: the lifecycle listener is not a
    // place where a browser API failure can become an exception.
    await env.fireStartup();

    assert.equal(env.searches.length, 1);
    assert.deepEqual(env.paused, [], 'the download keeps running in the browser');
    assert.deepEqual(env.createdWindows, []);
    assert.deepEqual(env.nativeCalls, []);
    assert.deepEqual(env.pendingRecords(), []);
    assert.equal(env.alarms.active.size, 0);
  });

  test('reconcile: never pauses without a persisted record plus a live alarm', async () => {
    const failures = [
      ['the record cannot be written', { storageSetFails: true }],
      ['the watchdog cannot be armed', { alarmsCreateFails: true }],
    ];
    for (const [label, options] of failures) {
      const env = setupFakeBrowser(settingsWith(), Object.assign({ inProgress: [inProgressItem(806)] }, options));
      await bootWorker();
      await env.fireStartup();

      assert.deepEqual(env.paused, [], `${label}: never pause`);
      assert.deepEqual(env.createdWindows, [], `${label}: no confirmation window`);
      assert.deepEqual(env.pendingRecords(), [], `${label}: no leftover record`);
    }
  });

  test('reconcile: the watchdog tick scans once, not on every alarm', async () => {
    const env = setupFakeBrowser(settingsWith({ capture_confirm: false }), {
      inProgress: [inProgressItem(807)],
    });
    await bootWorker();

    await env.fireAlarm();
    await env.fireAlarm();

    assert.deepEqual(env.paused, [807], 'the first tick captures the in-progress download');
    assert.equal(env.searches.length, 1, 'the second tick stays inside the throttle window');
  });

  test('reconcile: an install or update reconciles as well', async () => {
    const env = setupFakeBrowser(settingsWith({ capture_confirm: false }), {
      inProgress: [inProgressItem(808)],
    });
    await bootWorker();

    await env.fireInstalled();

    assert.deepEqual(env.paused, [808]);
    assert.equal(env.searches.length, 1);
    assert.deepEqual(env.canceled, [808]);
  });

  test('reconcile: the number of downloads examined per scan is capped', async () => {
    const many = [];
    const env = setupFakeBrowser(settingsWith({ capture_confirm: false }), {
      inProgress: () => many,
    });
    const bg = await bootWorker();
    const cap = bg.MAX_RECONCILE_DOWNLOADS;
    assert.equal(Number.isInteger(cap) && cap > 0, true, 'the cap must be a positive constant');
    for (let index = 0; index < cap + 5; index += 1) {
      many.push(inProgressItem(820 + index));
    }

    await env.fireStartup();

    assert.equal(env.paused.length, cap, `only the first ${cap} downloads are examined`);
  });

  test('watchdog: the alarms wrapper settles for promise-style browsers', async () => {
    const env = setupFakeBrowser(settingsWith(), { alarmsPromiseStyle: true });
    const bg = await bootWorker();

    await captureWithFrozenTimers(bg, 716);

    assert.deepEqual(env.paused, [716], 'a promise-style alarms.create must not block the capture');
    assert.equal(env.pendingRecords().length, 1);
    assert.equal(env.alarms.active.has(ALARM_NAME), true);
    await bg.handle(
      { type: 'segmenta.capture.decide', downloadId: 716, decision: 'reject', url: ZIP_URL },
      SENDER,
    );
    assert.equal(env.alarms.active.size, 0);
  });
}
