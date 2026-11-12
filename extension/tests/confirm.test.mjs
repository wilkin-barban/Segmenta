// Capture confirmation (S10): the browser download is paused first and is only
// cancelled after the host accepted the task. A rejected, expired or failed
// confirmation always resumes the browser download.
import assert from 'node:assert/strict';
import { loadShared } from './helpers.mjs';

const EXTENSION_ID = 'abcdefghijklmnopabcdefghijklmnop';
const ZIP_URL = 'https://downloads.example.org/tools/archive.zip';
// Kept in sync with the background cache constant; the background test below
// fails loudly when the two drift.
const CACHE_VERSION = '0.1.3';
const SENDER = {
  id: EXTENSION_ID,
  url: `chrome-extension://${EXTENSION_ID}/confirm.html`,
  origin: `chrome-extension://${EXTENSION_ID}`,
};

// Window ids are unique across the whole suite so a stale pending entry from an
// earlier test can never be resumed by a later one.
let windowSequence = 500;

function setupFakeBrowser(settings, nativeReplies = {}, options = {}) {
  const canceled = [];
  const erased = [];
  const paused = [];
  const resumed = [];
  const createdWindows = [];
  const removedWindows = [];
  const nativeCalls = [];
  const notifications = [];
  const events = [];
  const failing = Array.isArray(options.failingMethods) ? options.failingMethods : [];
  // Task status replies, in order (the last one repeats). The default already
  // reports bytes, so the daemon owns the file and the browser copy can go.
  const statusReplies = Array.isArray(options.statusReplies) ? options.statusReplies.slice() : [];
  let statusCalls = 0;
  const storageData = {
    segmenta_settings: settings,
    segmenta_settings_version: typeof options.cacheVersion === 'string' ? options.cacheVersion : CACHE_VERSION,
  };
  // The capture watchdog: every pause is preceded by an alarm this fake records.
  const alarms = { created: [], cleared: [], active: new Set() };
  const openWindows = new Set();
  let nextTaskId = 0;

  function statusReply(taskId) {
    statusCalls += 1;
    const next = statusReplies.length > 1 ? statusReplies.shift() : statusReplies[0];
    return Object.assign(
      { task_id: taskId, status: 'downloading', downloaded_bytes: 4096 },
      next || {},
    );
  }

  function defaultTaskId() {
    nextTaskId += 1;
    return 'task-' + String(nextTaskId).padStart(6, '0');
  }

  const notificationsApi = {
    create(idOrOptions, optionsOrCallback, maybeCallback) {
      const notificationId = typeof idOrOptions === 'string' ? idOrOptions : '';
      const createOptions = typeof idOrOptions === 'string' ? optionsOrCallback : idOrOptions;
      const callback = typeof optionsOrCallback === 'function' ? optionsOrCallback : maybeCallback;
      notifications.push({ id: notificationId, title: createOptions.title, message: createOptions.message });
      events.push('notify');
      if (callback) callback(notificationId);
      return Promise.resolve(notificationId);
    },
  };

  globalThis.chrome = {
    runtime: {
      id: EXTENSION_ID,
      lastError: undefined,
      getURL(relative) {
        return `chrome-extension://${EXTENSION_ID}/${String(relative).replace(/^\/+/, '')}`;
      },
      onMessage: { listeners: [], addListener(fn) { this.listeners.push(fn); } },
      sendNativeMessage(host, message, callback) {
        nativeCalls.push({ host, message });
        events.push('native:' + message.method);
        if (failing.indexOf(message.method) !== -1) {
          callback({ jsonrpc: '2.0', id: message.id, error: { code: -32601, message: 'method not found' } });
          return;
        }
        const canned = nativeReplies[message.method];
        if (canned) {
          callback(Object.assign({}, canned, { id: message.id }));
          return;
        }
        const result = message.method === 'system.open'
          ? { opened: true }
          : message.method === 'settings.get'
            ? settings
            : message.method === 'media.status'
              ? statusReply(message.params.task_id)
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
          Object.assign(storageData, items);
          if (cb) cb();
        },
      },
    },
    alarms: {
      onAlarm: { listeners: [], addListener(fn) { this.listeners.push(fn); } },
      create(name, info, cb) {
        alarms.created.push({ name: name, info: info });
        alarms.active.add(name);
        if (cb) cb();
        return undefined;
      },
      clear(name, cb) {
        alarms.cleared.push(name);
        alarms.active.delete(name);
        if (cb) cb(true);
        return undefined;
      },
    },
    downloads: {
      onCreated: {
        listeners: [],
        addListener(fn) { this.listeners.push(fn); },
        removeListener(fn) { this.listeners = this.listeners.filter((l) => l !== fn); },
      },
      pause(id, cb) {
        if (options.pauseFails) throw new Error('download cannot be paused');
        paused.push(id);
        events.push('pause:' + id);
        if (cb) cb();
      },
      resume(id, cb) {
        if (options.resumeFails) throw new Error('download cannot be resumed');
        resumed.push(id);
        events.push('resume:' + id);
        if (cb) cb();
      },
      cancel(id, cb) {
        canceled.push(id);
        events.push('cancel:' + id);
        if (cb) cb();
      },
      erase(query, cb) {
        erased.push(query);
        events.push('erase:' + query.id);
        if (cb) cb();
      },
    },
    windows: {
      onRemoved: { listeners: [], addListener(fn) { this.listeners.push(fn); } },
      create(data, cb) {
        if (options.windowsFails) throw new Error('windows.create is not available');
        events.push('window:create');
        if (options.windowsHangs) {
          // A browser that accepts the call and never answers.
          return new Promise(() => {});
        }
        windowSequence += 1;
        const id = windowSequence;
        createdWindows.push({ id: id, url: data && data.url });
        openWindows.add(id);
        if (cb) cb({ id: id });
        return undefined;
      },
      get(id, cb) {
        if (openWindows.has(id)) {
          if (cb) cb({ id: id });
          return undefined;
        }
        globalThis.chrome.runtime.lastError = { message: 'No window with id: ' + id };
        try {
          if (cb) cb(undefined);
        } finally {
          globalThis.chrome.runtime.lastError = undefined;
        }
        return undefined;
      },
      remove(id, cb) {
        removedWindows.push(id);
        openWindows.delete(id);
        events.push('window:remove:' + id);
        if (cb) cb();
      },
    },
    contextMenus: {
      create(props, cb) { if (cb) cb(); },
      onClicked: { listeners: [], addListener(fn) { this.listeners.push(fn); } },
    },
  };

  if (options.notifications !== false) {
    globalThis.chrome.notifications = notificationsApi;
  }

  return {
    canceled,
    erased,
    paused,
    resumed,
    createdWindows,
    removedWindows,
    nativeCalls,
    notifications,
    events,
    storageData,
    alarms,
    statusCalls() {
      return statusCalls;
    },
    methods() {
      return nativeCalls.map((call) => call.message.method);
    },
  };
}

async function boot() {
  await loadShared('SegmentaApi', 'browser-api.js');
  await loadShared('SegmentaProtocol', 'protocol.js');
  await loadShared('SegmentaValidate', 'validate.js');
  await import(new URL('../chromium/background.js', import.meta.url).href);
  return globalThis.SegmentaBackground;
}

// Closes every window a test left open and waits for the resume to land, so no
// 120 s watchdog timer keeps the Node process alive between tests.
async function settle(bg, env) {
  for (const created of env.createdWindows) {
    bg.onWindowRemoved(created.id);
  }
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function settingsWith(extra) {
  return Object.assign({
    capture_enabled: true,
    capture_torrents: true,
    capture_magnet: true,
    capture_extensions: ['zip'],
    domain_denylist: [],
    download_dir: '/home/u/Descargas',
  }, extra || {});
}

function item(id, url) {
  return {
    id: id,
    url: url || ZIP_URL,
    filename: (url || ZIP_URL).split('/').pop(),
    referrer: 'https://downloads.example.org/tools/',
  };
}

function confirmQuery(env) {
  const created = env.createdWindows[0];
  assert.ok(created, 'the confirmation window was never created');
  const queryIndex = created.url.indexOf('?');
  assert.notEqual(queryIndex, -1, 'the confirmation window url must carry the pending download');
  return new URLSearchParams(created.url.slice(queryIndex + 1));
}

function indexOf(events, expected) {
  const index = events.indexOf(expected);
  assert.notEqual(index, -1, `missing ${expected} in ${JSON.stringify(events)}`);
  return index;
}

function assertBefore(events, first, second) {
  assert.ok(
    indexOf(events, first) < indexOf(events, second),
    `${first} must happen before ${second} in ${JSON.stringify(events)}`,
  );
}

function resetTiming(bg) {
  bg.setCaptureTiming({
    ownershipTimeoutMs: bg.TASK_OWNERSHIP_TIMEOUT_MS,
    pollIntervalMs: bg.TASK_POLL_INTERVAL_MS,
    windowTimeoutMs: bg.CONFIRM_WINDOW_TIMEOUT_MS,
  });
}

function statusEventIndices(events) {
  const out = [];
  events.forEach((event, index) => {
    if (event === 'native:media.status') out.push(index);
  });
  return out;
}

export default function suite({ test }) {
  test('capture confirm: the settings cache version was bumped for the new keys', async () => {
    setupFakeBrowser(settingsWith());
    const bg = await boot();
    assert.equal(bg.SETTINGS_CACHE_VERSION, CACHE_VERSION);
  });

  test('capture confirm: a stale cache re-reads the settings from the host', async () => {
    const settings = settingsWith({ capture_confirm: false });
    const env = setupFakeBrowser(settings, {}, { cacheVersion: '0.1.1' });
    const bg = await boot();

    const fresh = await bg.getCachedSettings();

    assert.deepEqual(env.methods(), ['settings.get']);
    assert.equal(fresh.capture_confirm, false);
    assert.equal(env.storageData.segmenta_settings_version, CACHE_VERSION);
  });

  test('capture confirm: the download is paused before the window opens and before any cancel', async () => {
    const env = setupFakeBrowser(settingsWith());
    const bg = await boot();

    await bg.onDownloadCreated(item(201));

    assert.deepEqual(env.paused, [201]);
    assert.deepEqual(env.events, ['pause:201', 'window:create']);
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
    assert.deepEqual(env.nativeCalls, []);
    assert.equal(env.createdWindows.length, 1);
    assert.match(env.createdWindows[0].url, /^chrome-extension:\/\/[a-p]{32}\/confirm\.html\?/);
    const query = confirmQuery(env);
    assert.equal(query.get('download'), '201');
    assert.equal(query.get('url'), ZIP_URL);
    assert.equal(query.get('filename'), 'archive.zip');
    // The effective folder comes from the daemon settings, never invented.
    assert.equal(query.get('folder'), '/home/u/Descargas');
    await settle(bg, env);
  });

  test('capture confirm: accept adds first and only then cancels and erases', async () => {
    const env = setupFakeBrowser(settingsWith());
    const bg = await boot();

    await bg.onDownloadCreated(item(202));
    const reply = await bg.handle(
      {
        type: 'segmenta.capture.decide',
        downloadId: 202,
        decision: 'accept',
        url: ZIP_URL,
        filename: 'archive.zip',
        referrer: 'https://downloads.example.org/tools/',
      },
      SENDER,
    );

    assert.equal(reply.ok, true);
    assert.equal(reply.data.accepted, true);
    assert.equal(reply.data.task_id, 'task-000001');
    assertBefore(env.events, 'pause:202', 'native:download.add');
    // The discard waits until the daemon demonstrably owns the file.
    assertBefore(env.events, 'native:download.add', 'native:media.status');
    assertBefore(env.events, 'native:media.status', 'cancel:202');
    assertBefore(env.events, 'cancel:202', 'erase:202');
    assert.deepEqual(env.canceled, [202]);
    assert.deepEqual(env.erased, [{ id: 202 }]);
    assert.deepEqual(env.resumed, []);
    assert.deepEqual(env.methods(), ['download.add', 'media.status', 'system.open']);
    assert.deepEqual(env.nativeCalls[0].message.params, {
      url: ZIP_URL,
      filename: 'archive.zip',
      headers: { Referer: 'https://downloads.example.org/tools/' },
    });
    assert.equal(env.notifications.length, 1);
  });

  test('capture confirm: accept still captures when the notifications API is missing', async () => {
    const env = setupFakeBrowser(settingsWith(), {}, { notifications: false });
    const bg = await boot();

    await bg.onDownloadCreated(item(203));
    const reply = await bg.handle(
      { type: 'segmenta.capture.decide', downloadId: 203, decision: 'accept', url: ZIP_URL },
      SENDER,
    );

    assert.equal(reply.ok, true);
    assert.deepEqual(env.canceled, [203]);
    assert.deepEqual(env.erased, [{ id: 203 }]);
    assert.deepEqual(env.methods(), ['download.add', 'media.status', 'system.open']);
  });

  test('capture confirm: a torrent confirmation goes through torrent.add', async () => {
    const env = setupFakeBrowser(settingsWith({ capture_extensions: [] }));
    const bg = await boot();
    const magnet = 'magnet:?xt=urn:btih:1234567890abcdef1234567890abcdef12345678';

    await bg.onDownloadCreated({ id: 204, url: magnet });
    const reply = await bg.handle(
      { type: 'segmenta.capture.decide', downloadId: 204, decision: 'accept', url: magnet },
      SENDER,
    );

    assert.equal(reply.ok, true);
    assert.deepEqual(env.methods(), ['torrent.add', 'media.status', 'system.open']);
    assert.deepEqual(env.canceled, [204]);
    assert.deepEqual(env.erased, [{ id: 204 }]);
  });

  test('capture confirm: reject resumes the browser download and touches nothing else', async () => {
    const env = setupFakeBrowser(settingsWith());
    const bg = await boot();

    await bg.onDownloadCreated(item(205));
    const reply = await bg.handle(
      { type: 'segmenta.capture.decide', downloadId: 205, decision: 'reject', url: ZIP_URL },
      SENDER,
    );

    assert.equal(reply.ok, true);
    assert.equal(reply.data.accepted, false);
    assert.deepEqual(env.resumed, [205]);
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
    assert.deepEqual(env.nativeCalls, []);
    assert.deepEqual(env.events, ['pause:205', 'window:create', 'resume:205']);
  });

  test('capture confirm: an expired confirmation resumes the download and closes the window', async () => {
    const env = setupFakeBrowser(settingsWith());
    const bg = await boot();

    try {
      bg.setConfirmTimeoutMs(5);
      await bg.onDownloadCreated(item(206));
      assert.deepEqual(env.resumed, []);
      await new Promise((resolve) => setTimeout(resolve, 40));
    } finally {
      bg.setConfirmTimeoutMs(bg.CAPTURE_CONFIRM_TIMEOUT_MS);
    }

    const windowId = env.createdWindows[0].id;
    assert.deepEqual(env.resumed, [206]);
    assert.deepEqual(env.removedWindows, [windowId]);
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
    assert.deepEqual(env.nativeCalls, []);
  });

  test('capture confirm: closing the window resumes the download', async () => {
    const env = setupFakeBrowser(settingsWith());
    const bg = await boot();

    await bg.onDownloadCreated(item(207));
    const windowId = env.createdWindows[0].id;
    bg.onWindowRemoved(windowId);
    await new Promise((resolve) => setTimeout(resolve, 0));

    assert.deepEqual(env.resumed, [207]);
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
    assert.deepEqual(env.nativeCalls, []);
    // Closing an already decided download is a no-op, never a second resume.
    env.events.length = 0;
    bg.onWindowRemoved(windowId);
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.deepEqual(env.events, []);
  });

  test('capture confirm: a failing host resumes the download instead of losing it', async () => {
    const env = setupFakeBrowser(settingsWith(), {}, { failingMethods: ['download.add'] });
    const bg = await boot();

    await bg.onDownloadCreated(item(208));
    const reply = await bg.handle(
      { type: 'segmenta.capture.decide', downloadId: 208, decision: 'accept', url: ZIP_URL },
      SENDER,
    );

    assert.equal(reply.ok, false);
    assert.deepEqual(env.resumed, [208]);
    assert.deepEqual(env.canceled, [], 'an errored add must never cancel the browser download');
    assert.deepEqual(env.erased, [], 'an errored add must never erase the browser download');
    assert.equal(env.notifications.length, 0);
  });

  test('capture confirm: capture_confirm false skips the window and keeps the safe order', async () => {
    const env = setupFakeBrowser(settingsWith({ capture_confirm: false }));
    const bg = await boot();

    await bg.onDownloadCreated(item(209));

    assert.deepEqual(env.createdWindows, [], 'no window when capture_confirm is false');
    assert.deepEqual(env.events, [
      'pause:209',
      'native:download.add',
      'native:media.status',
      'cancel:209',
      'erase:209',
      'notify',
      'native:system.open',
    ]);
  });

  test('capture confirm: capture_confirm false resumes when the host fails', async () => {
    const env = setupFakeBrowser(settingsWith({ capture_confirm: false }), {}, { failingMethods: ['download.add'] });
    const bg = await boot();

    await bg.onDownloadCreated(item(210));

    assert.deepEqual(env.events, ['pause:210', 'native:download.add', 'resume:210']);
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
  });

  test('capture confirm: an absent capture_confirm key defaults to asking', async () => {
    const settings = settingsWith();
    delete settings.capture_confirm;
    const env = setupFakeBrowser(settings);
    const bg = await boot();

    await bg.onDownloadCreated(item(211));

    assert.equal(env.createdWindows.length, 1);
    assert.deepEqual(env.canceled, []);
    await settle(bg, env);
  });

  test('capture confirm: the folder is reported as pending when the settings carry none', async () => {
    const settings = settingsWith({ download_dir: '' });
    const env = setupFakeBrowser(settings);
    const bg = await boot();

    await bg.onDownloadCreated(item(212));

    const query = confirmQuery(env);
    assert.equal(query.get('folder'), null, 'no folder may be invented');
    await settle(bg, env);
  });

  test('capture confirm: a download that cannot be paused is left untouched', async () => {
    const env = setupFakeBrowser(settingsWith(), {}, { pauseFails: true });
    const bg = await boot();

    await bg.onDownloadCreated(item(213));

    assert.deepEqual(env.createdWindows, []);
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
    assert.deepEqual(env.nativeCalls, []);
  });

  test('capture confirm: a second decision for the same download is refused', async () => {
    const env = setupFakeBrowser(settingsWith());
    const bg = await boot();

    await bg.onDownloadCreated(item(214));
    const first = await bg.handle(
      { type: 'segmenta.capture.decide', downloadId: 214, decision: 'accept', url: ZIP_URL },
      SENDER,
    );
    const second = await bg.handle(
      { type: 'segmenta.capture.decide', downloadId: 214, decision: 'accept', url: ZIP_URL },
      SENDER,
    );

    assert.equal(first.ok, true);
    assert.equal(second.ok, false);
    assert.equal(second.error.code, 'ERR_CAPTURE_DECIDED');
    assert.deepEqual(env.methods(), ['download.add', 'media.status', 'system.open'], 'the download is only added once');
    assert.deepEqual(env.canceled, [214]);
  });

  test('capture confirm: a confirmation window that cannot open resumes the download', async () => {
    const env = setupFakeBrowser(settingsWith(), {}, { windowsFails: true });
    const bg = await boot();

    await bg.onDownloadCreated(item(215));

    assert.deepEqual(env.resumed, [215]);
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
    assert.deepEqual(env.nativeCalls, []);
  });

  test('capture confirm: a download without id or url is ignored before any pause', async () => {
    const env = setupFakeBrowser(settingsWith());
    const bg = await boot();

    await bg.onDownloadCreated(null);
    await bg.onDownloadCreated({});
    await bg.onDownloadCreated({ id: 0, url: ZIP_URL });
    await bg.onDownloadCreated({ id: 218 });

    assert.deepEqual(env.events, []);
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.nativeCalls, []);
  });

  test('capture confirm: malformed decisions are rejected before touching anything', async () => {
    const env = setupFakeBrowser(settingsWith());
    const bg = await boot();

    await bg.onDownloadCreated(item(216));

    for (const payload of [
      { downloadId: -1, decision: 'accept', url: ZIP_URL },
      { downloadId: 'x', decision: 'accept', url: ZIP_URL },
      { downloadId: 1.5, decision: 'accept', url: ZIP_URL },
      { downloadId: 216, decision: 'maybe', url: ZIP_URL },
      { downloadId: 216, decision: 'accept', url: 'file:///etc/passwd' },
      { downloadId: 216, decision: 'accept' },
    ]) {
      const reply = await bg.handle(Object.assign({ type: 'segmenta.capture.decide' }, payload), SENDER);
      assert.equal(reply.ok, false, `should reject ${JSON.stringify(payload)}`);
      assert.equal(reply.error.code.startsWith('ERR_'), true);
    }
    assert.deepEqual(env.methods(), []);
    assert.deepEqual(env.canceled, []);
    // The paused download is still waiting for a real decision.
    assert.deepEqual(env.resumed, []);
    await settle(bg, env);
  });

  test('capture confirm: a decision from another extension is refused', async () => {
    const env = setupFakeBrowser(settingsWith());
    const bg = await boot();

    await bg.onDownloadCreated(item(217));
    const reply = await bg.handle(
      { type: 'segmenta.capture.decide', downloadId: 217, decision: 'accept', url: ZIP_URL },
      { id: 'ponmlkjihgfedcbaponmlkjihgfedcba', url: 'https://evil.test/', origin: 'https://evil.test' },
    );

    assert.equal(reply.ok, false);
    assert.equal(reply.error.code, 'ERR_INVALID_SENDER');
    assert.deepEqual(env.methods(), []);
    assert.deepEqual(env.canceled, []);
    await settle(bg, env);
  });

  // ---- Ownership safety net (S10) -------------------------------------------
  // A task id is not proof: the browser copy survives until the daemon shows bytes.

  async function acceptCapture(bg, downloadId) {
    return bg.handle(
      { type: 'segmenta.capture.decide', downloadId: downloadId, decision: 'accept', url: ZIP_URL },
      SENDER,
    );
  }

  test('capture confirm: accept erases only after the task reports bytes', async () => {
    const env = setupFakeBrowser(settingsWith(), {}, {
      statusReplies: [
        { status: 'downloading', downloaded_bytes: 0 },
        { status: 'downloading', downloaded_bytes: 8192 },
      ],
    });
    const bg = await boot();

    await bg.onDownloadCreated(item(220));
    bg.setCaptureTiming({ pollIntervalMs: 1 });
    let reply;
    try {
      reply = await acceptCapture(bg, 220);
    } finally {
      resetTiming(bg);
    }

    const polls = statusEventIndices(env.events);
    assert.equal(reply.ok, true);
    assert.equal(polls.length, 2, `two polls before the discard: ${JSON.stringify(env.events)}`);
    assert.equal(polls[0] < env.events.indexOf('cancel:220'), true, 'the first status precedes the cancel');
    assert.equal(polls[1] < env.events.indexOf('cancel:220'), true, 'the byte-bearing status precedes the cancel');
    assert.deepEqual(env.canceled, [220]);
    assert.deepEqual(env.erased, [{ id: 220 }]);
    assert.deepEqual(env.resumed, []);
  });

  test('capture confirm: a failed task resumes the download and never erases it', async () => {
    const env = setupFakeBrowser(settingsWith(), {}, {
      statusReplies: [{ status: 'failed', downloaded_bytes: 0 }],
    });
    const bg = await boot();

    await bg.onDownloadCreated(item(221));
    const reply = await acceptCapture(bg, 221);

    // The window must not pretend the capture worked.
    assert.equal(reply.ok, false);
    assert.equal(reply.error.code, 'ERR_CAPTURE_NOT_OWNED');
    assert.deepEqual(env.resumed, [221]);
    assert.deepEqual(env.canceled, [], 'a failed task must never cancel the browser download');
    assert.deepEqual(env.erased, [], 'a failed task must never erase the browser download');
    assert.equal(env.notifications.length, 0, 'the window already reports the failure');
    assert.deepEqual(env.methods(), ['download.add', 'media.status', 'download.cancel']);
    assert.deepEqual(env.nativeCalls[2].message.params, { task_id: 'task-000001', delete_file: false });
    assertBefore(env.events, 'resume:221', 'native:download.cancel');
  });

  test('capture confirm: an exhausted polling budget keeps the browser copy', async () => {
    const env = setupFakeBrowser(settingsWith(), {}, {
      statusReplies: [{ status: 'downloading', downloaded_bytes: 0 }],
    });
    const bg = await boot();

    await bg.onDownloadCreated(item(222));
    bg.setCaptureTiming({ ownershipTimeoutMs: 6, pollIntervalMs: 1 });
    let reply;
    try {
      reply = await acceptCapture(bg, 222);
    } finally {
      resetTiming(bg);
    }

    assert.equal(reply.ok, false);
    assert.equal(reply.error.code, 'ERR_CAPTURE_NOT_OWNED');
    assert.deepEqual(env.resumed, [222]);
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
    assert.equal(statusEventIndices(env.events).length > 0, true, 'the budget is spent polling');
  });

  test('capture confirm: capture_confirm false keeps the browser copy when the task fails', async () => {
    const env = setupFakeBrowser(settingsWith({ capture_confirm: false }), {}, {
      statusReplies: [{ status: 'failed', downloaded_bytes: 0 }],
    });
    const bg = await boot();

    await bg.onDownloadCreated(item(223));

    assert.deepEqual(env.createdWindows, [], 'no window when capture_confirm is false');
    assert.deepEqual(env.events, [
      'pause:223',
      'native:download.add',
      'native:media.status',
      'resume:223',
      'native:download.cancel',
      'notify',
    ]);
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
    // No window can explain it, so the user is told the browser copy was kept.
    assert.equal(env.notifications.length, 1);
    assert.equal(env.notifications[0].title, 'Descarga conservada en el navegador');
  });

  test('capture confirm: a windows.create that never answers resumes the download', async () => {
    const env = setupFakeBrowser(settingsWith(), {}, { windowsHangs: true });
    const bg = await boot();

    bg.setCaptureTiming({ windowTimeoutMs: 5 });
    try {
      await bg.onDownloadCreated(item(224));
    } finally {
      resetTiming(bg);
    }

    assert.deepEqual(env.events, ['pause:224', 'window:create', 'resume:224']);
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
    assert.deepEqual(env.nativeCalls, []);
    assert.deepEqual(env.createdWindows, []);
  });
  // ---- confirm.js page (S10) -------------------------------------------------
  // The window is a plain page: it renders the pending download, sends one
  // validated decision and only ever reports what the background answered. A
  // failed decision must never look like a successful capture.

  const PAGE_ELEMENT_IDS = [
    'title',
    'intro',
    'name-label',
    'host-label',
    'folder-label',
    'name',
    'host',
    'folder',
    'accept',
    'reject',
    'status',
  ];

  const pageApi = { api: null, send: null, sent: [] };
  let confirmPage = null;

  function installConfirmDom(search) {
    const nodes = new Map();
    for (const id of PAGE_ELEMENT_IDS) {
      nodes.set(id, {
        id: id,
        textContent: '',
        className: '',
        disabled: true,
        handlers: {},
        addEventListener(type, handler) {
          this.handlers[type] = handler;
        },
      });
    }
    globalThis.document = {
      readyState: 'loading',
      getElementById: (id) => (nodes.has(id) ? nodes.get(id) : null),
      addEventListener() {},
    };
    globalThis.location = { search: search };
    globalThis.close = () => {};
    return { nodes };
  }

  function pageNode(dom, id) {
    const element = dom.nodes.get(id);
    assert.ok(element, `missing page element ${id}`);
    return element;
  }

  async function flushPage() {
    await new Promise((resolve) => setImmediate(resolve));
  }

  function ensureConfirmApi() {
    if (!pageApi.api) {
      pageApi.api = {
        getMessage(key, substitutions) {
          return substitutions === undefined ? key : key + '|' + [].concat(substitutions).join('|');
        },
        sendMessage(message) {
          pageApi.sent.push(message);
          return pageApi.send(message);
        },
      };
      // confirm.js captures the adapter at load time, exactly like the page does.
      globalThis.SegmentaApi = pageApi.api;
    }
    return pageApi;
  }

  function installConfirmApi(reply) {
    pageApi.sent = [];
    pageApi.send = reply;
    return ensureConfirmApi();
  }

  async function loadConfirmPage(dom) {
    ensureConfirmApi();
    if (!confirmPage) {
      await loadShared('SegmentaProtocol', 'protocol.js');
      await import(new URL('../chromium/confirm.js', import.meta.url).href);
      confirmPage = globalThis.SegmentaConfirm;
    }
    confirmPage.boot();
    return { page: confirmPage, dom: dom };
  }

  test('confirm page: renders the pending download and reports the effective folder', async () => {
    const query = new URLSearchParams({
      download: '42',
      url: ZIP_URL,
      filename: 'archive.zip',
      referrer: 'https://downloads.example.org/tools/',
      folder: '/home/u/Descargas',
    });
    const dom = installConfirmDom('?' + query.toString());
    const api = installConfirmApi(() =>
      Promise.resolve({
        ok: true,
        data: {
          accepted: true,
          download_id: 42,
          task_id: 'task-000001',
          // No destination_dir on purpose: the window must not show the whole file
          // path as the folder (it takes the parent).
          final_path: '/home/u/Descargas/sub/archive.zip',
        },
      }),
    );
    const { page } = await loadConfirmPage(dom);

    assert.equal(pageNode(dom, 'title').textContent, 'confirmTitle');
    assert.equal(pageNode(dom, 'name').textContent, 'archive.zip');
    assert.equal(pageNode(dom, 'host').textContent, 'downloads.example.org');
    assert.equal(pageNode(dom, 'folder').textContent, '/home/u/Descargas');
    assert.equal(pageNode(dom, 'accept').disabled, false);
    assert.equal(pageNode(dom, 'reject').disabled, false);

    pageNode(dom, 'accept').handlers.click();
    await flushPage();

    assert.deepEqual(api.sent, [
      {
        type: 'segmenta.capture.decide',
        downloadId: 42,
        decision: 'accept',
        url: ZIP_URL,
        filename: 'archive.zip',
        referrer: 'https://downloads.example.org/tools/',
      },
    ]);
    // The parent of final_path, never the file itself.
    assert.equal(pageNode(dom, 'status').textContent, 'confirmAccepted|/home/u/Descargas/sub');
    assert.equal(pageNode(dom, 'status').className, 'status ok');
    assert.equal(pageNode(dom, 'accept').disabled, true);
    // The destination_dir branch wins when the daemon reports it.
    assert.equal(
      page.acceptedFolder({ destination_dir: ' /srv/segmenta ', final_path: '/x/y.bin' }, 'fallback'),
      '/srv/segmenta',
    );
    assert.equal(page.acceptedFolder({ final_path: '/x/y.bin' }, 'fallback'), '/x');
    assert.equal(page.acceptedFolder({}, 'fallback'), 'fallback');
  });

  test('confirm page: an unanswered folder stays pending and a failure is not success', async () => {
    const query = new URLSearchParams({ download: '43', url: ZIP_URL, filename: 'archive.zip' });
    const dom = installConfirmDom('?' + query.toString());
    const api = installConfirmApi(() =>
      Promise.resolve({ ok: false, error: { code: 'ERR_CAPTURE_NOT_OWNED', message: 'kept' } }),
    );
    await loadConfirmPage(dom);

    // Without a daemon folder the window says so instead of inventing one.
    assert.equal(pageNode(dom, 'folder').textContent, 'confirmFolderPending');

    pageNode(dom, 'accept').handlers.click();
    await flushPage();

    assert.equal(api.sent.length, 1);
    assert.equal(pageNode(dom, 'status').textContent, 'confirmFailed');
    assert.equal(pageNode(dom, 'status').className, 'status error');
    assert.equal(pageNode(dom, 'status').textContent.includes('confirmAccepted'), false);
    assert.equal(pageNode(dom, 'accept').disabled, true);
  });

  test('confirm page: rejecting asks the background to keep the browser copy', async () => {
    const query = new URLSearchParams({ download: '44', url: ZIP_URL });
    const dom = installConfirmDom('?' + query.toString());
    const api = installConfirmApi(() => Promise.resolve({ ok: true, data: { accepted: false } }));
    await loadConfirmPage(dom);

    pageNode(dom, 'reject').handlers.click();
    await flushPage();

    assert.deepEqual(api.sent, [
      { type: 'segmenta.capture.decide', downloadId: 44, decision: 'reject', url: ZIP_URL },
    ]);
    assert.equal(pageNode(dom, 'status').textContent, 'confirmRejected');
    assert.equal(pageNode(dom, 'status').className, 'status');
  });

  test('confirm page: a decision that cannot be sent is reported as a failure', async () => {
    const query = new URLSearchParams({ download: '45', url: ZIP_URL });
    const dom = installConfirmDom('?' + query.toString());
    installConfirmApi(() => Promise.reject(new Error('no receiver')));
    await loadConfirmPage(dom);

    pageNode(dom, 'accept').handlers.click();
    await flushPage();

    assert.equal(pageNode(dom, 'status').textContent, 'confirmFailed');
    assert.equal(pageNode(dom, 'status').className, 'status error');
  });
}
