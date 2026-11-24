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

  /** Whether a task in `status` can be paused from the popup. */
  function canPause(status) {
    return ['pending', 'probing', 'downloading', 'retrying', 'verifying', 'finalizing'].indexOf(status) !== -1;
  }

  function progressText(task) {
    if (!task || typeof task !== 'object') {
      return '';
    }
    if (task.status === 'completed') {
      return t('downloadsCompleted');
    }
    if (task.status === 'paused') {
      return t('downloadsPaused');
    }
    const total = Number(task.total_bytes);
    const done = Number(task.downloaded_bytes);
    if (!Number.isFinite(total) || !Number.isFinite(done) || total <= 0) {
      return task.status || '';
    }
    const percent = Math.min(100, Math.round((done / total) * 100));
    return percent + '%';
  }

  async function refreshDownloads() {
    const list = byId('downloads-list');
    if (!list) {
      return;
    }
    while (list.firstChild) {
      list.removeChild(list.firstChild);
    }
    let reply;
    try {
      reply = await API.sendMessage({ type: 'segmenta.list', filter: 'all', limit: 10 });
    } catch (err) {
      const item = global.document.createElement('li');
      item.textContent = t('downloadsError');
      list.appendChild(item);
      return;
    }
    const tasks = reply && reply.ok === true && Array.isArray(reply.result)
      ? reply.result
      : [];
    if (tasks.length === 0) {
      const item = global.document.createElement('li');
      item.textContent = t('downloadsEmpty');
      list.appendChild(item);
      return;
    }
    for (const task of tasks) {
      list.appendChild(downloadRow(task));
    }
  }

  /** One row: name, progress and the actions its status allows. */
  function downloadRow(task) {
    const item = global.document.createElement('li');
    const name = global.document.createElement('span');
    name.className = 'name';
    name.textContent = task.filename || task.url || '';
    name.title = task.url || '';
    const meta = global.document.createElement('span');
    meta.className = 'meta';
    meta.textContent = progressText(task);
    item.appendChild(name);
    item.appendChild(meta);
    const actions = canPause(task.status)
      ? [['pause', 'actionPause'], ['cancel', 'actionCancel']]
      : task.status === 'paused'
        ? [['resume', 'actionResume'], ['cancel', 'actionCancel']]
        : [];
    for (const [action, label] of actions) {
      const button = global.document.createElement('button');
      button.type = 'button';
      button.textContent = t(label);
      button.addEventListener('click', () => act(task.task_id, action, button));
      item.appendChild(button);
    }
    return item;
  }

  async function act(taskId, action, button) {
    button.disabled = true;
    try {
      await API.sendMessage({ type: 'segmenta.action', action: action, taskId: taskId });
    } catch (err) {
      // The refresh below is the source of truth; nothing else to report here.
    }
    await refreshDownloads();
  }

  function boot() {
    byId('title').textContent = t('popupTitle');
    byId('url-label').textContent = t('popupUrlLabel');
    byId('url').placeholder = t('popupUrlPlaceholder');
    byId('mode-label').textContent = t('popupModeLabel');
    byId('mode-video').textContent = t('popupModeVideo');
    byId('mode-audio').textContent = t('popupModeAudio');
    byId('start').textContent = t('popupStart');
    byId('downloads-title').textContent = t('downloadsTitle');
    byId('downloads-refresh').textContent = t('downloadsRefresh');
    byId('downloads-refresh').addEventListener('click', refreshDownloads);
    refreshDownloads();

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
