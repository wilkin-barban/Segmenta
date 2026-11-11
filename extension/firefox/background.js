// Segmenta background service worker (Chromium classic MV3 worker, reused by
// Firefox through background.scripts).
//
// Responsibilities:
//   * validate every inbound message (sender, type, url, size);
//   * translate an allowlisted message into one JSON-RPC 2.0 envelope;
//   * correlate the native reply by id with a 10 s timeout;
//   * never forward arbitrary methods, urls or commands to the host.
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
  // HLS/DASH sites are opt-in for the future optional host permission; until the
  // user grants a site, only the fixed allowlist below is accepted.
  const STREAM_ALLOWLIST = [];

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
        return callHost(() =>
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
    handle: handle,
    hostFailure: hostFailure,
  };
})(typeof globalThis !== 'undefined' ? globalThis : this);
