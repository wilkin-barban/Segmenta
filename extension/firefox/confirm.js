// Segmenta capture confirmation window (S10).
//
// The background paused a browser download and opened this page with the pending
// download in the query string. Accepting sends one validated decision message; the
// background only cancels and erases the browser copy after the native host
// accepted the task. Rejecting (or closing the window) resumes the download, so the
// file is never lost.
//
// ASCII source on purpose: every user-visible string comes from _locales.
(function (global) {
  'use strict';

  const API = global.SegmentaApi;
  const PROTOCOL = global.SegmentaProtocol;

  const CLOSE_DELAY_MS = 1500;

  function t(key, substitutions) {
    if (!API || typeof API.getMessage !== 'function') {
      return '';
    }
    const text = substitutions === undefined ? API.getMessage(key) : API.getMessage(key, substitutions);
    return typeof text === 'string' ? text : '';
  }

  function byId(id) {
    return global.document ? global.document.getElementById(id) : null;
  }

  function setStatus(text, kind) {
    const node = byId('status');
    if (!node) {
      return;
    }
    node.textContent = text;
    node.className = kind === 'error' ? 'status error' : kind === 'ok' ? 'status ok' : 'status';
  }

  function queryParams() {
    try {
      return new URLSearchParams(global.location.search);
    } catch (err) {
      return null;
    }
  }

  function fileNameOf(url, filename) {
    if (typeof filename === 'string' && filename.trim()) {
      return filename.trim();
    }
    try {
      const parsed = new URL(url);
      const segments = parsed.pathname.split('/').filter(Boolean);
      const last = segments.length > 0 ? segments[segments.length - 1] : '';
      return last || parsed.hostname;
    } catch (err) {
      return '';
    }
  }

  function hostOf(url) {
    // A magnet link has no http host, only the tracker-less urn.
    if (typeof url === 'string' && url.toLowerCase().startsWith('magnet:')) {
      return t('confirmHostMagnet');
    }
    try {
      return new URL(url).hostname;
    } catch (err) {
      return '';
    }
  }

  function closeSoon() {
    global.setTimeout(() => {
      try {
        global.close();
      } catch (err) {}
    }, CLOSE_DELAY_MS);
  }

  function disableButtons() {
    for (const id of ['accept', 'reject']) {
      const node = byId(id);
      if (node) {
        node.disabled = true;
      }
    }
  }

  function renderQuery() {
    const query = queryParams();
    const rawId = query ? query.get('download') : null;
    const downloadId = rawId === null ? NaN : Number(rawId);
    const url = query && query.get('url') ? query.get('url') : '';
    const filename = query && query.get('filename') ? query.get('filename') : '';
    const referrer = query && query.get('referrer') ? query.get('referrer') : '';
    // Only the daemon reports the effective folder; an empty value stays pending.
    const folder = query && query.get('folder') ? query.get('folder') : '';

    const title = byId('title');
    if (title) title.textContent = t('confirmTitle');
    const intro = byId('intro');
    if (intro) intro.textContent = t('confirmIntro');
    const nameLabel = byId('name-label');
    if (nameLabel) nameLabel.textContent = t('confirmNameLabel');
    const hostLabel = byId('host-label');
    if (hostLabel) hostLabel.textContent = t('confirmHostLabel');
    const folderLabel = byId('folder-label');
    if (folderLabel) folderLabel.textContent = t('confirmFolderLabel');
    const name = byId('name');
    if (name) name.textContent = fileNameOf(url, filename) || t('confirmUnknownName');
    const host = byId('host');
    if (host) host.textContent = hostOf(url) || t('confirmUnknownHost');
    const folderNode = byId('folder');
    if (folderNode) folderNode.textContent = folder || t('confirmFolderPending');
    const accept = byId('accept');
    if (accept) accept.textContent = t('confirmAccept');
    const reject = byId('reject');
    if (reject) reject.textContent = t('confirmReject');

    return {
      downloadId: downloadId,
      url: url,
      filename: filename,
      referrer: referrer,
      folder: folder,
    };
  }

  /**
   * Folder where the accepted task landed. `final_path` is a file, so the whole path
   * is never shown as a directory: only its parent is (same rule as the background).
   */
  function acceptedFolder(data, fallback) {
    if (data && typeof data === 'object') {
      if (typeof data.destination_dir === 'string' && data.destination_dir.trim()) {
        return data.destination_dir.trim();
      }
      const parent = PROTOCOL && typeof PROTOCOL.parentDirectory === 'function'
        ? PROTOCOL.parentDirectory(data.final_path)
        : '';
      if (parent) {
        return parent;
      }
    }
    return fallback;
  }

  function sendDecision(pending, accepted) {
    const message = {
      type: 'segmenta.capture.decide',
      downloadId: pending.downloadId,
      decision: accepted ? 'accept' : 'reject',
      url: pending.url,
    };
    if (pending.filename) message.filename = pending.filename;
    if (pending.referrer) message.referrer = pending.referrer;
    return API.sendMessage(message);
  }

  function onAccepted(pending, reply) {
    if (!reply || reply.ok !== true) {
      setStatus(t('confirmFailed'), 'error');
      closeSoon();
      return;
    }
    if (reply.data && reply.data.accepted === false) {
      setStatus(t('confirmRejected'));
      closeSoon();
      return;
    }
    const folder = acceptedFolder(reply.data, pending.folder);
    if (folder) {
      setStatus(t('confirmAccepted', [folder]), 'ok');
    } else {
      setStatus(t('confirmAcceptedGeneric'), 'ok');
    }
    closeSoon();
  }

  function onRejected() {
    setStatus(t('confirmRejected'));
    closeSoon();
  }

  function boot() {
    const pending = renderQuery();
    if (!Number.isInteger(pending.downloadId) || pending.downloadId < 0 || !pending.url) {
      setStatus(t('confirmInvalid'), 'error');
      disableButtons();
      return;
    }

    const accept = byId('accept');
    const reject = byId('reject');
    if (accept) {
      accept.disabled = false;
      accept.addEventListener('click', () => {
        disableButtons();
        setStatus(t('statusPreparing'));
        sendDecision(pending, true).then(
          (reply) => onAccepted(pending, reply),
          () => {
            setStatus(t('confirmFailed'), 'error');
            closeSoon();
          },
        );
      });
    }
    if (reject) {
      reject.disabled = false;
      reject.addEventListener('click', () => {
        disableButtons();
        sendDecision(pending, false).then(onRejected, onRejected);
      });
    }
  }

  if (global.document) {
    if (global.document.readyState === 'loading') {
      global.document.addEventListener('DOMContentLoaded', boot, { once: true });
    } else {
      boot();
    }
  }

  // Exported for the Node tests; the page itself never reads these directly.
  global.SegmentaConfirm = {
    fileNameOf: fileNameOf,
    hostOf: hostOf,
    renderQuery: renderQuery,
    sendDecision: sendDecision,
    acceptedFolder: acceptedFolder,
    boot: boot,
  };
})(typeof globalThis !== 'undefined' ? globalThis : this);
