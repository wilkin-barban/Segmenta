import assert from 'node:assert/strict';
import { loadShared } from './helpers.mjs';

const EXTENSION_ID = 'abcdefghijklmnopabcdefghijklmnop';

function setupFakeBrowser(settings, nativeReplies = {}) {
  const canceled = [];
  const erased = [];
  const nativeCalls = [];
  const storageData = { segmenta_settings: settings };

  globalThis.chrome = {
    runtime: {
      id: EXTENSION_ID,
      lastError: undefined,
      sendNativeMessage(host, message, callback) {
        nativeCalls.push({ host, message });
        const reply = nativeReplies[message.method] || { jsonrpc: '2.0', id: message.id, result: { ok: true } };
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

  return { canceled, erased, nativeCalls, storageData };
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
    assert.equal(env.nativeCalls.length, 1);
    assert.equal(env.nativeCalls[0].message.method, 'download.add');
    assert.equal(env.nativeCalls[0].message.params.url, 'https://example.com/archive.zip');
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
    assert.equal(env.nativeCalls.length, 1);
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
    assert.equal(env.nativeCalls.length, 2);
    assert.equal(env.nativeCalls[0].message.method, 'torrent.add');
    assert.equal(env.nativeCalls[1].message.method, 'torrent.add');
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
    assert.equal(env.nativeCalls.length, 2);
  });
}
