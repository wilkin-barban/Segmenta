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
  const storageData = { segmenta_settings: settings, segmenta_settings_version: '0.1.1' };
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
      sendNativeMessage(host, message, callback) {
        nativeCalls.push({ host, message });
        events.push('native:' + message.method);
        if (failing.indexOf(message.method) !== -1) {
          callback({ jsonrpc: '2.0', id: message.id, error: { code: -32601, message: 'method not found' } });
          return;
        }
        const canned = nativeReplies[message.method];
        const reply = canned
          ? Object.assign({}, canned, { id: message.id })
          : {
            jsonrpc: '2.0',
            id: message.id,
            result: message.method === 'system.open'
              ? { opened: true }
              : { task_id: defaultTaskId(), status: 'downloading' },
          };
        callback(reply);
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
    downloads: {
      onCreated: {
        listeners: [],
        addListener(fn) { this.listeners.push(fn); },
        removeListener(fn) { this.listeners = this.listeners.filter(l => l !== fn); },
      },
      cancel(id, cb) {
        canceled.push(id);
        if (cb) cb();
      },
      erase(query, cb) {
        erased.push(query);
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

  return { canceled, erased, nativeCalls, notifications, events, storageData };
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
    // A captured file is added and then focused: no other host traffic is allowed.
    assert.deepEqual(env.events, ['native:download.add', 'notify', 'native:system.open']);
    assert.deepEqual(env.nativeCalls.map((call) => call.message.method), ['download.add', 'system.open']);
    assert.equal(env.nativeCalls[0].message.params.url, 'https://example.com/archive.zip');
    assert.deepEqual(env.nativeCalls[1].message.params, { task_id: 'task-000001' });
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
    assert.deepEqual(env.nativeCalls.map((call) => call.message.method), ['download.add', 'system.open']);
  });

  test('background capture: torrent routed to torrent.add', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['zip'],
      domain_denylist: [],
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
    assert.deepEqual(env.nativeCalls.map((call) => call.message.method), [
      'torrent.add',
      'system.open',
      'torrent.add',
      'system.open',
    ]);
    assert.deepEqual(env.nativeCalls[1].message.params, { task_id: 'task-000001' });
    assert.deepEqual(env.nativeCalls[3].message.params, { task_id: 'task-000002' });
  });

  test('background capture: r0* and r1* prefix match', async () => {
    const settings = {
      capture_enabled: true,
      capture_torrents: true,
      capture_magnet: true,
      capture_extensions: ['r0*', 'r1*'],
      domain_denylist: [],
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
    assert.deepEqual(env.nativeCalls.map((call) => call.message.method), [
      'download.add',
      'system.open',
      'download.add',
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
}
