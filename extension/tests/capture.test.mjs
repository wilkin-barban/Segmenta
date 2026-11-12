import assert from 'node:assert/strict';
import { loadShared } from './helpers.mjs';

const EXTENSION_ID = 'abcdefghijklmnopabcdefghijklmnop';
const YOUTUBE_WATCH = 'https://www.youtube.com/watch?v=dQw4w9WgXcQ';
const SENDER = { id: EXTENSION_ID, url: YOUTUBE_WATCH };
const TASK_ADDED_TITLE = 'Añadida a Segmenta';

function setupFakeBrowser(settings, nativeReplies = {}, options = {}) {
  const canceled = [];
  const erased = [];
  const nativeCalls = [];
  // Everything the background does after a successful add, in call order, so the
  // "notify first, then focus" contract can be asserted exactly.
  const events = [];
  const notifications = [];
  const failing = Array.isArray(options.failingMethods) ? options.failingMethods : [];
  // Task status replies, in order (the last one repeats). The default already
  // reports bytes: the daemon owns the file, so the browser copy can go.
  const statusReplies = Array.isArray(options.statusReplies) ? options.statusReplies.slice() : [];
  let statusCalls = 0;
  // Keep in step with SegmentaBackground.SETTINGS_CACHE_VERSION (asserted below).
  const storageData = { segmenta_settings: settings, segmenta_settings_version: '0.1.3' };
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
      notifications.push({
        id: notificationId,
        title: createOptions.title,
        message: createOptions.message,
      });
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
            for (const k of keys) {
              if (storageData[k] !== undefined) res[k] = storageData[k];
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
        removeListener(fn) { this.listeners = this.listeners.filter(l => l !== fn); },
      },
      // A capture pauses first and only cancels once the host owns the file.
      pause(id, cb) {
        events.push('pause:' + id);
        if (cb) cb();
      },
      resume(id, cb) {
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
        events.push('window:create');
        if (options.windowsHangs) {
          // A browser that accepts the call and never answers.
          return new Promise(() => {});
        }
        openWindows.add(900);
        if (cb) cb({ id: 900 });
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

  return { canceled, erased, nativeCalls, notifications, events, storageData, alarms, statusCalls: () => statusCalls };
}

function statusEventIndices(events) {
  const out = [];
  events.forEach((event, index) => {
    if (event === 'native:media.status') out.push(index);
  });
  return out;
}

function resetTiming(bg) {
  bg.setCaptureTiming({
    ownershipTimeoutMs: bg.TASK_OWNERSHIP_TIMEOUT_MS,
    pollIntervalMs: bg.TASK_POLL_INTERVAL_MS,
    windowTimeoutMs: bg.CONFIRM_WINDOW_TIMEOUT_MS,
  });
}

export default function suite({ test }) {
  test('protocol buildDownloadAdd and buildTorrentAdd work correctly', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    const dl = P.buildDownloadAdd({ url: 'https://example.com/file.zip', filename: 'file.zip', referrer: 'https://example.com/', id: 1 });
    assert.deepEqual(dl, {
      jsonrpc: '2.0',
      id: 1,
      method: 'download.add',
      params: { url: 'https://example.com/file.zip', filename: 'file.zip', headers: { Referer: 'https://example.com/' } },
    });

    const tor = P.buildTorrentAdd({ url: 'https://example.com/file.torrent', id: 2 });
    assert.deepEqual(tor, {
      jsonrpc: '2.0',
      id: 2,
      method: 'torrent.add',
      params: { url: 'https://example.com/file.torrent' },
    });

    const mag = P.buildTorrentAdd({ url: 'magnet:?xt=urn:btih:1234567890abcdef1234567890abcdef12345678', id: 3 });
    assert.deepEqual(mag, {
      jsonrpc: '2.0',
      id: 3,
      method: 'torrent.add',
      params: { url: 'magnet:?xt=urn:btih:1234567890abcdef1234567890abcdef12345678' },
    });

    assert.throws(() => P.buildDownloadAdd({ url: 'ftp://example.com/x' }), /url/i);
    assert.throws(() => P.buildTorrentAdd({ url: 'http://example.com/file.zip' }), /url/i);
  });

  test('validate message types segmenta.capture and segmenta.torrent.add', async () => {
    const V = await loadShared('SegmentaValidate', 'validate.js');
    const options = { extensionId: EXTENSION_ID };
    const cap = V.validateMessage({ type: 'segmenta.capture', url: 'https://example.com/a.zip', filename: 'a.zip' }, options);
    assert.equal(cap.ok, true);
    assert.equal(cap.value.type, 'segmenta.capture');

    const tor = V.validateMessage({ type: 'segmenta.torrent.add', url: 'magnet:?xt=urn:btih:1234567890abcdef1234567890abcdef12345678' }, options);
    assert.equal(tor.ok, true);
    assert.equal(tor.value.type, 'segmenta.torrent.add');
  });

  test('background capture: permitted extension captured', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['zip'],
      domain_denylist: [],
      // The window itself is covered by confirm.test.mjs; these tests pin the
      // non-interactive safe order (pause -> add -> status -> cancel+erase).
      capture_confirm: false,
    };
    const env = setupFakeBrowser(settings);
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    await bg.onDownloadCreated({ id: 101, url: 'https://example.com/archive.zip', filename: 'archive.zip' });

    assert.deepEqual(env.canceled, [101]);
    assert.deepEqual(env.erased, [{ id: 101 }]);
    // Paused first, added second, owned third, discarded last: no other host
    // traffic is allowed.
    assert.deepEqual(env.events, [
      'pause:101',
      'native:download.add',
      'native:media.status',
      'cancel:101',
      'erase:101',
      'notify',
      'native:system.open',
    ]);
    assert.deepEqual(env.nativeCalls.map((call) => call.message.method), [
      'download.add',
      'media.status',
      'system.open',
    ]);
    assert.equal(env.nativeCalls[0].message.params.url, 'https://example.com/archive.zip');
    assert.deepEqual(env.nativeCalls[1].message.params, { task_id: 'task-000001' });
    assert.deepEqual(env.nativeCalls[2].message.params, { task_id: 'task-000001' });
  });

  test('background capture: denylisted host skipped', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['zip'],
      domain_denylist: ['blocked.example.com'],
    };
    const env = setupFakeBrowser(settings);
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    await bg.onDownloadCreated({ id: 102, url: 'https://blocked.example.com/archive.zip', filename: 'archive.zip' });

    assert.equal(env.canceled.length, 0);
    assert.equal(env.nativeCalls.length, 0);
  });

  test('background capture: disabled capture skipped', async () => {
    const settings = {
      capture_enabled: false,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['zip'],
      domain_denylist: [],
      // The window itself is covered by confirm.test.mjs; these tests pin the
      // non-interactive safe order (pause -> add -> status -> cancel+erase).
      capture_confirm: false,
    };
    const env = setupFakeBrowser(settings);
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    await bg.onDownloadCreated({ id: 103, url: 'https://example.com/archive.zip', filename: 'archive.zip' });

    assert.equal(env.canceled.length, 0);
    assert.equal(env.nativeCalls.length, 0);
  });

  test('background capture: blob and data ignored', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['zip'],
      domain_denylist: [],
      // The window itself is covered by confirm.test.mjs; these tests pin the
      // non-interactive safe order (pause -> add -> status -> cancel+erase).
      capture_confirm: false,
    };
    const env = setupFakeBrowser(settings);
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    await bg.onDownloadCreated({ id: 104, url: 'blob:https://example.com/123', filename: 'file.zip' });
    await bg.onDownloadCreated({ id: 105, url: 'data:text/plain;base64,AAAA', filename: 'file.zip' });

    assert.equal(env.canceled.length, 0);
    assert.equal(env.nativeCalls.length, 0);
  });

  test('background capture: exact-once handling (no loop)', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['zip'],
      domain_denylist: [],
      // The window itself is covered by confirm.test.mjs; these tests pin the
      // non-interactive safe order (pause -> add -> status -> cancel+erase).
      capture_confirm: false,
    };
    const env = setupFakeBrowser(settings);
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    await bg.onDownloadCreated({ id: 106, url: 'https://example.com/archive.zip', filename: 'archive.zip' });
    // Re-entry with same id should be ignored
    await bg.onDownloadCreated({ id: 106, url: 'https://example.com/archive.zip', filename: 'archive.zip' });

    assert.deepEqual(env.canceled, [106]);
    assert.deepEqual(env.events, [
      'pause:106',
      'native:download.add',
      'native:media.status',
      'cancel:106',
      'erase:106',
      'notify',
      'native:system.open',
    ]);
    assert.deepEqual(env.nativeCalls.map((call) => call.message.method), [
      'download.add',
      'media.status',
      'system.open',
    ]);
  });

  test('background capture: torrent routed to torrent.add', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['zip'],
      domain_denylist: [],
      // The window itself is covered by confirm.test.mjs; these tests pin the
      // non-interactive safe order (pause -> add -> status -> cancel+erase).
      capture_confirm: false,
    };
    const env = setupFakeBrowser(settings);
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    await bg.onDownloadCreated({ id: 107, url: 'https://example.com/file.torrent', filename: 'file.torrent' });
    await bg.onDownloadCreated({ id: 108, url: 'magnet:?xt=urn:btih:1234567890abcdef1234567890abcdef12345678' });

    assert.deepEqual(env.canceled, [107, 108]);
    assert.deepEqual(env.events, [
      'pause:107',
      'native:torrent.add',
      'native:media.status',
      'cancel:107',
      'erase:107',
      'notify',
      'native:system.open',
      'pause:108',
      'native:torrent.add',
      'native:media.status',
      'cancel:108',
      'erase:108',
      'notify',
      'native:system.open',
    ]);
    assert.deepEqual(env.nativeCalls.map((call) => call.message.method), [
      'torrent.add',
      'media.status',
      'system.open',
      'torrent.add',
      'media.status',
      'system.open',
    ]);
    assert.deepEqual(env.nativeCalls[2].message.params, { task_id: 'task-000001' });
    assert.deepEqual(env.nativeCalls[5].message.params, { task_id: 'task-000002' });
  });

  test('background capture: r0* and r1* prefix match', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['r0*', 'r1*'],
      domain_denylist: [],
      capture_confirm: false,
    };
    const env = setupFakeBrowser(settings);
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    await bg.onDownloadCreated({ id: 109, url: 'https://example.com/archive.r00', filename: 'archive.r00' });
    await bg.onDownloadCreated({ id: 110, url: 'https://example.com/archive.r15', filename: 'archive.r15' });
    await bg.onDownloadCreated({ id: 111, url: 'https://example.com/archive.zip', filename: 'archive.zip' });

    assert.deepEqual(env.canceled, [109, 110]);
    assert.deepEqual(env.events, [
      'pause:109',
      'native:download.add',
      'native:media.status',
      'cancel:109',
      'erase:109',
      'notify',
      'native:system.open',
      'pause:110',
      'native:download.add',
      'native:media.status',
      'cancel:110',
      'erase:110',
      'notify',
      'native:system.open',
    ]);
    assert.deepEqual(env.nativeCalls.map((call) => call.message.method), [
      'download.add',
      'media.status',
      'system.open',
      'download.add',
      'media.status',
      'system.open',
    ]);
  });

  test('background add: successful media.add focuses the task after the add reply', async () => {
    const taskId = '123e4567-e89b-12d3-a456-426614174000';
    const env = setupFakeBrowser({}, {
      'media.add': { jsonrpc: '2.0', id: 1, result: { task_id: taskId, status: 'downloading' } },
    });
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    const result = await bg.handle({ type: 'segmenta.add', url: YOUTUBE_WATCH, mode: 'video' }, SENDER);

    assert.equal(result.ok, true);
    assert.equal(result.data.task_id, taskId);
    assert.deepEqual(env.events, ['native:media.add', 'notify', 'native:system.open']);
    assert.deepEqual(env.nativeCalls[1].message, {
      jsonrpc: '2.0',
      id: env.nativeCalls[1].message.id,
      method: 'system.open',
      params: { task_id: taskId },
    });
  });

  test('background add: a failing system.open never changes the add result', async () => {
    const taskId = '123e4567-e89b-12d3-a456-426614174001';
    const env = setupFakeBrowser(
      {},
      { 'media.add': { jsonrpc: '2.0', id: 1, result: { task_id: taskId, status: 'downloading' } } },
      { failingMethods: ['system.open'] },
    );
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    const result = await bg.handle({ type: 'segmenta.add', url: YOUTUBE_WATCH, mode: 'video' }, SENDER);

    assert.equal(result.ok, true);
    assert.equal(result.data.task_id, taskId);
    assert.deepEqual(env.events, ['native:media.add', 'notify', 'native:system.open']);
  });

  test('background add: a missing notifications API never fails the add', async () => {
    const taskId = '123e4567-e89b-12d3-a456-426614174002';
    const env = setupFakeBrowser(
      {},
      { 'media.add': { jsonrpc: '2.0', id: 1, result: { task_id: taskId, status: 'downloading' } } },
      { notifications: false },
    );
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    const result = await bg.handle({ type: 'segmenta.add', url: YOUTUBE_WATCH, mode: 'audio' }, SENDER);

    assert.equal(result.ok, true);
    assert.equal(env.notifications.length, 0);
    assert.deepEqual(env.events, ['native:media.add', 'native:system.open']);
  });

  test('background add: an add without task_id is not focused', async () => {
    const env = setupFakeBrowser({}, {
      'media.add': { jsonrpc: '2.0', id: 1, result: { status: 'downloading' } },
    });
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    const result = await bg.handle({ type: 'segmenta.add', url: YOUTUBE_WATCH, mode: 'video' }, SENDER);

    assert.equal(result.ok, true);
    assert.deepEqual(env.events, ['native:media.add']);
  });

  test('background add: the notification carries the folder from final_path', async () => {
    const taskId = '123e4567-e89b-12d3-a456-426614174003';
    const env = setupFakeBrowser({}, {
      'download.add': {
        jsonrpc: '2.0',
        id: 1,
        result: { task_id: taskId, status: 'downloading', final_path: '/home/u/Descargas/ubuntu.iso' },
      },
      'media.add': {
        jsonrpc: '2.0',
        id: 2,
        result: { task_id: taskId, status: 'downloading', destination_dir: '/srv/segmenta/video' },
      },
    });
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    await bg.handle({ type: 'segmenta.capture', url: 'https://example.com/ubuntu.iso' }, SENDER);
    assert.equal(env.notifications.length, 1);
    assert.equal(env.notifications[0].title, TASK_ADDED_TITLE);
    assert.equal(env.notifications[0].message.includes('/home/u/Descargas'), true);
    assert.equal(env.notifications[0].message.includes('ubuntu.iso'), false);

    // destination_dir wins over final_path when the host sends both.
    await bg.handle({ type: 'segmenta.add', url: YOUTUBE_WATCH, mode: 'video' }, SENDER);
    assert.equal(env.notifications.length, 2);
    assert.equal(env.notifications[1].message.includes('/srv/segmenta/video'), true);
    assert.deepEqual(env.nativeCalls.map((call) => call.message.method), [
      'download.add',
      'system.open',
      'media.add',
      'system.open',
    ]);
  });

  test('background open: a valid request only focuses the task (no add, no toast)', async () => {
    const taskId = '123e4567-e89b-12d3-a456-426614174010';
    const env = setupFakeBrowser({});
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    const result = await bg.handle({ type: 'segmenta.open', taskId: taskId }, SENDER);

    assert.equal(result.ok, true);
    // One host call and nothing else: the task already exists.
    assert.deepEqual(env.events, ['native:system.open']);
    assert.equal(env.notifications.length, 0);
    assert.deepEqual(env.nativeCalls.map((call) => call.message.method), ['system.open']);
    assert.deepEqual(env.nativeCalls[0].message.params, { task_id: taskId });
  });

  test('background open: an invalid task id is rejected before touching the host', async () => {
    const env = setupFakeBrowser({});
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    for (const taskId of ['', '   ', 'a b', 'a;rm -rf /', 'short', null]) {
      const result = await bg.handle({ type: 'segmenta.open', taskId: taskId }, SENDER);
      assert.equal(result.ok, false, `should reject ${String(taskId)}`);
      assert.equal(result.error.code, 'ERR_INVALID_PARAMS', `should reject ${String(taskId)}`);
    }
    assert.deepEqual(env.nativeCalls, []);
    assert.deepEqual(env.events, []);
    assert.equal(env.notifications.length, 0);
  });

  test('background capture: the fake cache version matches the background constant', async () => {
    setupFakeBrowser({});
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);

    assert.equal(globalThis.SegmentaBackground.SETTINGS_CACHE_VERSION, '0.1.3');
  });

  test('background open: a failing host error is reported and nothing is announced', async () => {
    const env = setupFakeBrowser({}, {}, { failingMethods: ['system.open'] });
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    const bg = globalThis.SegmentaBackground;

    const result = await bg.handle({ type: 'segmenta.open', taskId: 'task-123456' }, SENDER);

    assert.equal(result.ok, false);
    assert.equal(result.error.code, 'ERR_UNKNOWN');
    assert.equal(result.error.host_code, -32601);
    assert.deepEqual(env.events, ['native:system.open']);
    assert.equal(env.notifications.length, 0);
  });

  // ---- Ownership safety net (S10) -------------------------------------------
  // The browser copy survives until the daemon demonstrably holds bytes.

  async function bootBackground() {
    await loadShared('SegmentaApi', 'browser-api.js');
    await loadShared('SegmentaProtocol', 'protocol.js');
    await loadShared('SegmentaValidate', 'validate.js');
    await import(new URL('../chromium/background.js', import.meta.url).href);
    return globalThis.SegmentaBackground;
  }

  test('background capture: the browser copy is erased only once the task reports bytes', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['zip'],
      domain_denylist: [],
      capture_confirm: false,
    };
    const env = setupFakeBrowser(settings, {}, {
      statusReplies: [
        { status: 'downloading', downloaded_bytes: 0 },
        { status: 'downloading', downloaded_bytes: 2048 },
      ],
    });
    const bg = await bootBackground();

    bg.setCaptureTiming({ pollIntervalMs: 1 });
    try {
      await bg.onDownloadCreated({ id: 120, url: 'https://example.com/archive.zip', filename: 'archive.zip' });
    } finally {
      resetTiming(bg);
    }

    const polls = statusEventIndices(env.events);
    assert.equal(polls.length, 2, `two polls must precede the discard: ${JSON.stringify(env.events)}`);
    assert.equal(env.events.indexOf('native:download.add') < polls[0], true, 'add comes before the first poll');
    assert.equal(polls[1] < env.events.indexOf('cancel:120'), true, 'no cancel before the task owns bytes');
    assert.equal(polls[1] < env.events.indexOf('erase:120'), true, 'no erase before the task owns bytes');
    assert.deepEqual(env.canceled, [120]);
    assert.deepEqual(env.erased, [{ id: 120 }]);
  });

  test('background capture: a task that reports completed counts as owned', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['zip'],
      domain_denylist: [],
      capture_confirm: false,
    };
    const env = setupFakeBrowser(settings, {}, {
      statusReplies: [{ status: 'completed', downloaded_bytes: 0 }],
    });
    const bg = await bootBackground();

    await bg.onDownloadCreated({ id: 121, url: 'https://example.com/archive.zip', filename: 'archive.zip' });

    assert.deepEqual(env.canceled, [121]);
    assert.deepEqual(env.erased, [{ id: 121 }]);
  });

  test('background capture: a failed task resumes the download and keeps the browser copy', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['zip'],
      domain_denylist: [],
      capture_confirm: false,
    };
    const env = setupFakeBrowser(settings, {}, {
      statusReplies: [{ status: 'failed', downloaded_bytes: 0 }],
    });
    const bg = await bootBackground();

    await bg.onDownloadCreated({ id: 122, url: 'https://example.com/archive.zip', filename: 'archive.zip' });

    assert.deepEqual(env.events, [
      'pause:122',
      'native:download.add',
      'native:media.status',
      'resume:122',
      'native:download.cancel',
      'notify',
    ]);
    assert.deepEqual(env.canceled, [], 'a failed task must never cancel the browser download');
    assert.deepEqual(env.erased, [], 'a failed task must never erase the browser download');
    // The orphan task is dropped without deleting any file.
    assert.deepEqual(env.nativeCalls[2].message.params, { task_id: 'task-000001', delete_file: false });
    assert.equal(env.notifications.length, 1);
    assert.equal(env.notifications[0].title, 'Descarga conservada en el navegador');
  });

  test('background capture: an exhausted polling budget keeps the browser copy', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['zip'],
      domain_denylist: [],
      capture_confirm: false,
    };
    const env = setupFakeBrowser(settings, {}, {
      statusReplies: [{ status: 'downloading', downloaded_bytes: 0 }],
    });
    const bg = await bootBackground();

    bg.setCaptureTiming({ ownershipTimeoutMs: 5, pollIntervalMs: 1 });
    try {
      await bg.onDownloadCreated({ id: 123, url: 'https://example.com/archive.zip', filename: 'archive.zip' });
    } finally {
      resetTiming(bg);
    }

    assert.equal(env.statusCalls() > 0, true, 'the budget must be spent polling, not skipped');
    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
    assert.equal(env.events.includes('resume:123'), true, `the download must be resumed: ${JSON.stringify(env.events)}`);
    assert.equal(env.notifications.length, 1);
  });

  test('background capture: a failing status call never erases before the budget ends', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['zip'],
      domain_denylist: [],
      capture_confirm: false,
    };
    const env = setupFakeBrowser(settings, {}, { failingMethods: ['media.status'] });
    const bg = await bootBackground();

    bg.setCaptureTiming({ ownershipTimeoutMs: 6, pollIntervalMs: 1 });
    try {
      await bg.onDownloadCreated({ id: 124, url: 'https://example.com/archive.zip', filename: 'archive.zip' });
    } finally {
      resetTiming(bg);
    }

    assert.deepEqual(env.canceled, []);
    assert.deepEqual(env.erased, []);
    assert.equal(env.events.includes('resume:124'), true);
    assert.equal(statusEventIndices(env.events).length > 0, true, 'the budget is spent polling');
  });
}
