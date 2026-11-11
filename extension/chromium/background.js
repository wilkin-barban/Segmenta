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

  const handledDownloadIds = new Set();

  function trackHandledId(id) {
    if (handledDownloadIds.size >= 100) {
      const first = handledDownloadIds.values().next().value;
      handledDownloadIds.delete(first);
    }
    handledDownloadIds.add(id);
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

  async function getCachedSettings() {
    try {
      if (API.storage && typeof API.storage.get === 'function') {
        const stored = await API.storage.get(['segmenta_settings']);
        if (stored && stored.segmenta_settings) {
          return stored.segmenta_settings;
        }
      }
    } catch (err) {}

    const reply = await callHost(() => PROTOCOL.envelope('settings.get', {}));
    if (reply.ok && reply.data) {
      try {
        if (API.storage && typeof API.storage.set === 'function') {
          await API.storage.set({ segmenta_settings: reply.data });
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

  async function onDownloadCreated(downloadItem) {
    if (!downloadItem || !downloadItem.id || !downloadItem.url) {
      return;
    }
    if (handledDownloadIds.has(downloadItem.id)) {
      return;
    }
    trackHandledId(downloadItem.id);

    const url = downloadItem.url;
    const parsed = PROTOCOL.parseHttpUrl(url);
    const isMagnet = url.startsWith('magnet:');
    if (!parsed && !isMagnet) {
      return;
    }

    if (parsed) {
      const scheme = parsed.protocol;
      if (scheme === 'blob:' || scheme === 'data:' || scheme === 'file:' || scheme === 'chrome-extension:' || scheme === 'moz-extension:') {
        return;
      }
    }

    if (downloadItem.byExtensionId && downloadItem.byExtensionId === extensionId()) {
      return;
    }

    const settings = await getCachedSettings();
    if (!settings) {
      return;
    }

    if (settings.capture_enabled === false) {
      return;
    }

    if (parsed && isHostDenylisted(url, settings.domain_denylist)) {
      return;
    }

    const isTorrentFile = PROTOCOL.parseTorrentUrl(url);
    if (isTorrentFile) {
      if (settings.capture_torrents === false) {
        return;
      }
    } else if (isMagnet) {
      if (settings.capture_magnet === false) {
        return;
      }
    } else {
      const filename = downloadItem.filename || downloadItem.url;
      if (!matchExtension(filename, settings.capture_extensions)) {
        return;
      }
    }

    try {
      if (API.downloads && typeof API.downloads.cancel === 'function') {
        await API.downloads.cancel(downloadItem.id);
      }
      if (API.downloads && typeof API.downloads.erase === 'function') {
        await API.downloads.erase({ id: downloadItem.id });
      }
    } catch (err) {}

    if (isTorrentFile || isMagnet) {
      await callHost(() => PROTOCOL.buildTorrentAdd({ url: url }));
    } else {
      await callHost(() => PROTOCOL.buildDownloadAdd({
        url: url,
        filename: downloadItem.filename,
        referrer: downloadItem.referrer,
      }));
    }
  }

  if (API && API.downloads && API.downloads.onCreated && typeof API.downloads.onCreated.addListener === 'function') {
    API.downloads.onCreated.addListener(onDownloadCreated);
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

  if (API && API.runtime && API.runtime.onInstalled && typeof API.runtime.onInstalled.addListener === 'function') {
    API.runtime.onInstalled.addListener(setupContextMenus);
  } else {
    setupContextMenus();
  }

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
      case 'segmenta.capture':
        return callHost(() =>
          PROTOCOL.buildDownloadAdd({
            url: value.url,
            filename: value.filename,
            referrer: value.referrer,
          }),
        );
      case 'segmenta.torrent.add':
        return callHost(() => PROTOCOL.buildTorrentAdd({ url: value.url }));
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
    onDownloadCreated: onDownloadCreated,
    getCachedSettings: getCachedSettings,
    matchExtension: matchExtension,
    isHostDenylisted: isHostDenylisted,
  };
})(typeof globalThis !== 'undefined' ? globalThis : this);
