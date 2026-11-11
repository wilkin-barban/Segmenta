// Segmenta popup: create a media task for a pasted video or manifest URL.
(function (global) {
  'use strict';

  const API = global.SegmentaApi;
  const PROTOCOL = global.SegmentaProtocol;

  function t(key, substitutions) {
    return API.getMessage(key, substitutions);
  }

  function byId(id) {
    return global.document.getElementById(id);
  }

  function setStatus(message, kind) {
    const node = byId('status');
    node.textContent = message;
    node.className = kind === 'error' ? 'status error' : kind === 'ok' ? 'status ok' : 'status';
  }

  function selectedMode() {
    const inputs = global.document.querySelectorAll('input[name="mode"]');
    for (const input of inputs) {
      if (input.checked) {
        return input.value === 'audio' ? 'audio' : 'video';
      }
    }
    return 'video';
  }

  function boot() {
    byId('title').textContent = t('popupTitle');
    byId('url-label').textContent = t('popupUrlLabel');
    byId('url').placeholder = t('popupUrlPlaceholder');
    byId('mode-label').textContent = t('popupModeLabel');
    byId('mode-video').textContent = t('popupModeVideo');
    byId('mode-audio').textContent = t('popupModeAudio');
    byId('start').textContent = t('popupStart');

    byId('start').addEventListener('click', async () => {
      const button = byId('start');
      const raw = byId('url').value;
      const url = typeof raw === 'string' ? raw.trim() : '';
      if (!PROTOCOL.isAllowedUrl(url, { allowStreams: true })) {
        setStatus(t('popupInvalidUrl'), 'error');
        return;
      }
      button.disabled = true;
      setStatus(t('statusPreparing'), '');
      let reply;
      try {
        reply = await API.sendMessage({ type: 'segmenta.add', url: url, mode: selectedMode() });
      } catch (err) {
        button.disabled = false;
        setStatus(t('errorHostUnavailable'), 'error');
        return;
      }
      button.disabled = false;
      if (!reply || reply.ok !== true) {
        const code = reply && reply.error && typeof reply.error.code === 'string' ? reply.error.code : '';
        setStatus(code === 'ERR_INVALID_URL' ? t('popupInvalidUrl') : t('popupError'), 'error');
        return;
      }
      setStatus(t('popupSent'), 'ok');
      byId('url').value = '';
    });
  }

  if (global.document) {
    if (global.document.readyState === 'loading') {
      global.document.addEventListener('DOMContentLoaded', boot, { once: true });
    } else {
      boot();
    }
  }
})(typeof globalThis !== 'undefined' ? globalThis : this);
