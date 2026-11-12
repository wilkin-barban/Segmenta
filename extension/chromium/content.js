// Segmenta content script (ISOLATED world, youtube.com only).
//
// Injects the floating button + Shadow DOM panel, asks the background for
// media.probe after an explicit click, sends media.add and polls media.status
// every 1.5 s until the task finishes. The stream sniffer talks to document.js
// through a private MessageChannel port, never through a wildcard message.
(function (global) {
  'use strict';

  const API = global.SegmentaApi;
  const PROTOCOL = global.SegmentaProtocol;
  const PANEL = global.SegmentaPanel;

  const POLL_INTERVAL_MS = 1500;
  const POLL_MAX_ATTEMPTS = 2400;
  const SNIFFER_CHANNEL = 'segmenta:sniffer:v1';

  const ERROR_KEYS = {
    ERR_DRM_PROTECTED: 'errorDrmProtected',
    ERR_LIVE_NOT_SUPPORTED: 'errorLiveNotSupported',
    ERR_MANIFEST_MALFORMED: 'errorManifestMalformed',
    ERR_STREAM_UNSUPPORTED: 'errorStreamUnsupported',
    ERR_MEDIA_BACKEND: 'errorMediaBackend',
    ERR_MEDIA_NOT_PUBLIC: 'errorNotPublic',
    ERR_INVALID_URL: 'errorInvalidUrl',
    ERR_INVALID_MESSAGE: 'errorInvalidMessage',
    ERR_INVALID_SENDER: 'errorInvalidMessage',
    ERR_INVALID_TYPE: 'errorInvalidMessage',
    ERR_INVALID_PARAMS: 'errorInvalidMessage',
    ERR_MESSAGE_TOO_LARGE: 'errorInvalidMessage',
    ERR_HOST_PROTOCOL: 'errorGeneric',
    ERR_HOST_TIMEOUT: 'errorTimeout',
    ERR_HOST_UNAVAILABLE: 'errorHostUnavailable',
    ERR_HOST_ERROR: 'errorGeneric',
    ERR_UNKNOWN: 'errorUnknown',
  };

  let panel = null;
  let pollTimer = null;
  let lastStream = null;

  function t(key, substitutions) {
    return API.getMessage(key, substitutions);
  }

  function translateError(error) {
    const code = error && typeof error.code === 'string' ? error.code : 'ERR_UNKNOWN';
    const key = ERROR_KEYS[code] || 'errorGeneric';
    const localized = t(key);
    return localized || t('errorGeneric');
  }

  function currentRoute() {
    const href = global.location.href;
    if (global.location.pathname === '/playlist') {
      const playlistUrl = PROTOCOL.playlistUrlFrom(href);
      return playlistUrl ? { kind: 'playlist', url: playlistUrl } : null;
    }
    const watchUrl = PROTOCOL.watchUrlFrom(href);
    if (watchUrl) {
      return { kind: 'video', url: watchUrl };
    }
    return null;
  }

  function stopPolling() {
    if (pollTimer !== null) {
      clearTimeout(pollTimer);
      pollTimer = null;
    }
  }

  async function sendProbe(target) {
    const payload = { type: 'segmenta.probe', url: target.url, kind: target.kind };
    if (target.kind === 'playlist') {
      payload.limit = PROTOCOL.DEFAULT_PLAYLIST_LIMIT;
    }
    let reply;
    try {
      reply = await API.sendMessage(payload);
    } catch (err) {
      panel.setError(t('errorHostUnavailable'));
      return;
    }
    if (!reply || reply.ok !== true) {
      panel.setError(translateError(reply && reply.error));
      return;
    }
    const data = reply.data && typeof reply.data === 'object' ? Object.assign({}, reply.data) : {};
    if (lastStream) {
      data.stream = lastStream;
    }
    panel.open(data);
  }

  function onProbe() {
    const target = currentRoute();
    panel.open(null);
    if (target) {
      sendProbe(target);
    }
  }

  function pollStatus(taskId, attempt) {
    if (attempt > POLL_MAX_ATTEMPTS) {
      stopPolling();
      panel.setError(t('errorTimeout'));
      return;
    }
    pollTimer = setTimeout(async () => {
      pollTimer = null;
      let reply;
      try {
        reply = await API.sendMessage({ type: 'segmenta.status', taskId: taskId });
      } catch (err) {
        stopPolling();
        panel.setError(t('errorHostUnavailable'));
        return;
      }
      if (!reply || reply.ok !== true) {
        stopPolling();
        panel.setError(translateError(reply && reply.error));
        return;
      }
      const status = reply.data && typeof reply.data === 'object' ? reply.data : {};
      panel.setProgress(status);
      const state = typeof status.status === 'string' ? status.status : '';
      if (state === 'completed' || state === 'failed' || state === 'cancelled') {
        stopPolling();
        return;
      }
      pollStatus(taskId, attempt + 1);
    }, POLL_INTERVAL_MS);
  }

  async function onStart(request) {
    const target = currentRoute();
    if (!target) {
      panel.setError(t('errorInvalidUrl'));
      return;
    }
    let url = request && request.url ? request.url : target.url;
    if ((!request || !request.url) && request && request.mode === 'playlist') {
      url = PROTOCOL.playlistUrlFrom(global.location.href) || target.url;
    }
    const payload = { type: 'segmenta.add', url: url, mode: request.mode };
    if (request.formatId) {
      payload.formatId = request.formatId;
    }
    if (request.height) {
      payload.height = request.height;
    }
    if (request.playlistLimit) {
      payload.playlistLimit = request.playlistLimit;
    }
    let reply;
    try {
      reply = await API.sendMessage(payload);
    } catch (err) {
      panel.setError(t('errorHostUnavailable'));
      return;
    }
    if (!reply || reply.ok !== true) {
      panel.setError(translateError(reply && reply.error));
      return;
    }
    const data = reply.data && typeof reply.data === 'object' ? reply.data : {};
    if (typeof data.task_id !== 'string' || !data.task_id) {
      panel.setError(t('errorGeneric'));
      return;
    }
    // The task exists: swap the card body for the handoff state, then follow it.
    panel.setSent(data.task_id);
    stopPolling();
    pollStatus(data.task_id, 0);
  }

  /**
   * Brings Segmenta forward focused on an existing task. The background turns this
   * into a single `system.open` call, so the task is never added twice (S9).
   */
  async function onOpenTask(taskId) {
    if (typeof taskId !== 'string' || !taskId) {
      return;
    }
    let reply;
    try {
      reply = await API.sendMessage({ type: 'segmenta.open', taskId: taskId });
    } catch (err) {
      panel.setError(t('errorHostUnavailable'));
      return;
    }
    if (!reply || reply.ok !== true) {
      panel.setError(translateError(reply && reply.error));
    }
  }

  function startSniffer() {
    if (typeof MessageChannel !== 'function') {
      return;
    }
    try {
      const channel = new MessageChannel();
      channel.port1.onmessage = (event) => {
        const data = event && event.data ? event.data : null;
        if (!data || data.channel !== SNIFFER_CHANNEL || data.type !== 'segmenta:media') {
          return;
        }
        if (typeof data.url !== 'string' || !PROTOCOL.isStreamUrl(data.url)) {
          return;
        }
        lastStream = data.url;
      };
      global.postMessage({ channel: SNIFFER_CHANNEL, type: 'segmenta:handshake' }, global.location.origin, [
        channel.port2,
      ]);
    } catch (err) {
      lastStream = null;
    }
  }

  function mount() {
    if (panel || !global.document || !global.document.body) {
      return;
    }
    panel = PANEL.createPanel({
      t: t,
      onProbe: onProbe,
      onStart: onStart,
      onOpenTask: onOpenTask,
      onClose: stopPolling,
    });
  }

  function sync() {
    const route = currentRoute();
    stopPolling();
    if (!route) {
      if (panel) {
        panel.close();
        panel.root.style.display = 'none';
      }
      return;
    }
    mount();
    if (panel) {
      panel.root.style.display = '';
    }
  }

  function boot() {
    startSniffer();
    sync();
    global.document.addEventListener('yt-navigate-finish', sync, true);
    global.addEventListener('popstate', sync, true);
  }

  if (global.document && global.document.body) {
    boot();
  } else if (global.document) {
    global.document.addEventListener('DOMContentLoaded', boot, { once: true });
  }
})(typeof globalThis !== 'undefined' ? globalThis : this);
