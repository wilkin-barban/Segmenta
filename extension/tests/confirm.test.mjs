// Capture confirmation (S10): the browser download is paused first and is only
// cancelled after the host accepted the task. A rejected, expired or failed
// confirmation always resumes the browser download.
import assert from 'node:assert/strict';
import { loadShared } from './helpers.mjs';

const EXTENSION_ID = 'abcdefghijklmnopabcdefghijklmnop';
const ZIP_URL = 'https://downloads.example.org/tools/archive.zip';
// Kept in sync with the background cache constant; the background test below
// fails loudly when the two drift.
const CACHE_VERSION = '0.1.2';
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
  const storageData = {
    segmenta_settings: settings,
    segmenta_settings_version: typeof options.cacheVersion === 'string' ? options.cacheVersion : CACHE_VERSION,
  };
  let nextTaskId = 0;

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
        windowSequence += 1;
        const id = windowSequence;
        createdWindows.push({ id: id, url: data && data.url });
        events.push('window:create');
        if (cb) cb({ id: id });
      },
      remove(id, cb) {
        removedWindows.push(id);
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
    assertBefore(env.events, 'native:download.add', 'cancel:202');
    assertBefore(env.events, 'cancel:202', 'erase:202');
    assert.deepEqual(env.canceled, [202]);
    assert.deepEqual(env.erased, [{ id: 202 }]);
    assert.deepEqual(env.resumed, []);
    assert.deepEqual(env.methods(), ['download.add', 'system.open']);
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
    assert.deepEqual(env.methods(), ['download.add', 'system.open']);
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
    assert.deepEqual(env.methods(), ['torrent.add', 'system.open']);
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
    assert.deepEqual(env.events, ['pause:209', 'native:download.add', 'cancel:209', 'erase:209', 'notify', 'native:system.open']);
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
    assert.deepEqual(env.methods(), ['download.add', 'system.open'], 'the download is only added once');
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
}
