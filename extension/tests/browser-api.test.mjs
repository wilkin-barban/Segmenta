import assert from 'node:assert/strict';
import { loadShared } from './helpers.mjs';

const EXTENSION_ID = 'abcdefghijklmnopabcdefghijklmnop';

function installFakeChrome() {
  const calls = [];
  globalThis.chrome = {
    runtime: {
      id: EXTENSION_ID,
      lastError: undefined,
      sendMessage(message, callback) {
        calls.push(['sendMessage', message]);
        callback({ ok: true, echo: message });
      },
      sendNativeMessage(host, message, callback) {
        calls.push(['sendNativeMessage', host, message]);
        callback({ jsonrpc: '2.0', id: message.id, result: { ok: true } });
      },
    },
    i18n: {
      getMessage(key, substitutions) {
        if (key === 'missingKey') {
          return '';
        }
        const suffix = Array.isArray(substitutions) ? substitutions.join('|') : '';
        return suffix ? `${key}:${suffix}` : key;
      },
    },
  };
  return calls;
}

export default function suite({ test }) {
  test('browser-api resolves chrome and publishes the host name', async () => {
    const calls = installFakeChrome();
    const api = await loadShared('SegmentaApi', 'browser-api.js');
    assert.equal(api.api, globalThis.chrome);
    assert.equal(globalThis.segmentaApi, globalThis.chrome);
    assert.equal(api.hostName, 'io.github.wilkin.segmenta');
    assert.equal(typeof api.sendMessage, 'function');
    assert.equal(typeof api.sendNative, 'function');
    assert.equal(typeof api.getMessage, 'function');
    assert.equal(calls.length, 0);
  });

  test('sendNative waits for an asynchronous callback with declared arity 2', async () => {
    const calls = installFakeChrome();
    // chrome.runtime.sendNativeMessage(host, message, callback) declares two named
    // parameters and answers through the third argument, asynchronously.
    globalThis.chrome.runtime.sendNativeMessage = (host, message, callback) => {
      calls.push(['sendNativeMessage', host, message]);
      setTimeout(() => callback({ jsonrpc: '2.0', id: message.id, result: { ok: true } }), 5);
    };
    const api = await loadShared('SegmentaApi', 'browser-api.js');
    const reply = await api.sendNative({ jsonrpc: '2.0', id: 9, method: 'media.probe', params: {} });
    assert.equal(reply && reply.result && reply.result.ok, true, 'the reply must not be undefined');
    assert.equal(reply.id, 9);
    assert.equal(calls.length, 1);
    assert.equal(calls[0][1], 'io.github.wilkin.segmenta');
  });

  test('sendMessage unwraps callback-style chrome.runtime.sendMessage', async () => {
    const calls = installFakeChrome();
    const api = await loadShared('SegmentaApi', 'browser-api.js');
    const reply = await api.sendMessage({ type: 'segmenta.probe', url: 'https://www.youtube.com/watch?v=x' });
    assert.deepEqual(reply.echo.type, 'segmenta.probe');
    assert.equal(calls[0][0], 'sendMessage');
  });

  test('sendNative posts to the fixed native host name', async () => {
    const calls = installFakeChrome();
    const api = await loadShared('SegmentaApi', 'browser-api.js');
    const reply = await api.sendNative({ jsonrpc: '2.0', id: 1, method: 'media.status', params: {} });
    assert.equal(reply.result.ok, true);
    assert.equal(calls[0][0], 'sendNativeMessage');
    assert.equal(calls[0][1], 'io.github.wilkin.segmenta');
  });

  test('sendNative accepts promise-style browser APIs', async () => {
    globalThis.chrome = {
      runtime: {
        id: EXTENSION_ID,
        sendMessage: async (message) => ({ ok: true, echo: message }),
        sendNativeMessage: async (host, message) => ({ jsonrpc: '2.0', id: message.id, result: { host } }),
      },
      i18n: { getMessage: () => 'txt' },
    };
    const api = await loadShared('SegmentaApi', 'browser-api.js');
    const reply = await api.sendNative({ jsonrpc: '2.0', id: 2, method: 'media.status', params: {} });
    assert.equal(reply.result.host, 'io.github.wilkin.segmenta');

    globalThis.browser = { runtime: { id: EXTENSION_ID } };
    assert.equal(typeof api.sendMessage, 'function');
    delete globalThis.browser;
  });

  test('getMessage substitutes and tolerates missing keys', async () => {
    installFakeChrome();
    const api = await loadShared('SegmentaApi', 'browser-api.js');
    assert.equal(api.getMessage('panelTitle'), 'panelTitle');
    assert.equal(api.getMessage('toastSaved', ['/home/x']), 'toastSaved:/home/x');
    assert.equal(api.getMessage('missingKey'), '');
  });
}
