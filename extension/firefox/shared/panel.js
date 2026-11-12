// Segmenta floating button and download panel.
//
// Everything is built as DOM nodes inside a Shadow DOM so the page CSS can never
// reach it and our CSS can never leak into the page. No inline handlers, no HTML
// strings: text coming from the daemon goes through sanitizeText().
(function (global) {
  'use strict';

  const PROTOCOL = global.SegmentaProtocol;
  const HOST_ID = 'segmenta-extension-root';

  const STYLES = [
    ':host { all: initial; }',
    '.segmenta-launch {',
    '  position: fixed; right: 20px; bottom: 20px; z-index: 2147483646;',
    '  display: inline-flex; align-items: center; gap: 8px;',
    '  padding: 10px 16px; border: 0; border-radius: 999px; cursor: pointer;',
    '  font: 600 13px/1.2 system-ui, -apple-system, "Segoe UI", sans-serif;',
    '  color: #ffffff; background: linear-gradient(180deg, #3584e4, #1c5fb0);',
    '  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.35);',
    '}',
    '.segmenta-launch:hover { filter: brightness(1.08); }',
    '.segmenta-launch:focus-visible { outline: 3px solid #ffffff; outline-offset: 2px; }',
    '.segmenta-panel {',
    '  position: fixed; right: 20px; bottom: 72px; z-index: 2147483646;',
    '  width: 320px; max-height: 70vh; overflow-y: auto;',
    '  padding: 14px; border-radius: 14px; color: #f6f6f6;',
    '  background: #1d1d20; border: 1px solid #3a3a40;',
    '  box-shadow: 0 14px 40px rgba(0, 0, 0, 0.5);',
    '  font: 400 13px/1.45 system-ui, -apple-system, "Segoe UI", sans-serif;',
    '}',
    '.segmenta-panel[hidden] { display: none; }',
    '.segmenta-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; }',
    '.segmenta-title { font-weight: 700; font-size: 14px; }',
    '.segmenta-close {',
    '  border: 0; background: transparent; color: #c9c9cf; cursor: pointer;',
    '  font-size: 16px; line-height: 1; padding: 4px 6px; border-radius: 6px;',
    '}',
    '.segmenta-close:hover { background: #2f2f36; color: #ffffff; }',
    '.segmenta-media-title { margin: 8px 0 0; font-size: 12px; color: #b9b9c2; word-break: break-word; }',
    '.segmenta-loading { margin: 10px 0 0; color: #b9b9c2; }',
    '.segmenta-loading[hidden] { display: none; }',
    '.segmenta-error {',
    '  margin: 10px 0 0; padding: 8px 10px; border-radius: 8px;',
    '  background: #40222a; border: 1px solid #7a2f3d; color: #ffd9df;',
    '  font-size: 12px; white-space: pre-wrap; word-break: break-word;',
    '}',
    '.segmenta-error[hidden] { display: none; }',
    'fieldset { margin: 12px 0 0; padding: 0; border: 0; }',
    'legend { padding: 0; margin: 0 0 6px; font-weight: 600; font-size: 12px; color: #d6d6dd; }',
    '.segmenta-option {',
    '  display: flex; align-items: center; gap: 8px; padding: 6px 8px;',
    '  border-radius: 8px; cursor: pointer;',
    '}',
    '.segmenta-option:hover { background: #26262c; }',
    '.segmenta-option input { accent-color: #3584e4; margin: 0; }',
    '.segmenta-option-label { flex: 1 1 auto; word-break: break-word; }',
    '.segmenta-limit-row { display: flex; align-items: center; gap: 8px; margin: 8px 0 0; }',
    '.segmenta-limit-row[hidden] { display: none; }',
    '.segmenta-limit-row input {',
    '  width: 76px; padding: 4px 6px; border-radius: 6px;',
    '  border: 1px solid #4a4a52; background: #121214; color: #f6f6f6;',
    '  font: inherit;',
    '}',
    '.segmenta-hint { display: block; margin: 4px 0 0; font-size: 11px; color: #8f8f9a; }',
    '.segmenta-start {',
    '  margin: 12px 0 0; width: 100%; padding: 9px 12px; border: 0; border-radius: 9px;',
    '  cursor: pointer; font: 600 13px/1.2 system-ui, sans-serif; color: #ffffff;',
    '  background: #3584e4;',
    '}',
    '.segmenta-start:hover { background: #4a93ea; }',
    '.segmenta-start:disabled { background: #3a3a44; color: #9a9aa4; cursor: default; }',
    '.segmenta-progress { margin: 12px 0 0; }',
    '.segmenta-progress[hidden] { display: none; }',
    '.segmenta-bar { height: 6px; border-radius: 999px; background: #2c2c33; overflow: hidden; }',
    '.segmenta-bar-fill { height: 100%; width: 0%; background: #3584e4; transition: width 200ms linear; }',
    '.segmenta-progress-text { margin: 6px 0 0; font-size: 12px; color: #d6d6dd; }',
    '.segmenta-playlist-text { margin: 4px 0 0; font-size: 12px; color: #a9a9b4; }',
    '.segmenta-playlist-text[hidden] { display: none; }',
    '.segmenta-toast {',
    '  margin: 12px 0 0; padding: 8px 10px; border-radius: 8px; font-size: 12px;',
    '  background: #1f3327; border: 1px solid #2f6b45; color: #d8ffe6; word-break: break-all;',
    '}',
    '.segmenta-toast[hidden] { display: none; }',
    '.segmenta-sent { margin: 12px 0 0; }',
    '.segmenta-sent[hidden] { display: none; }',
    '.segmenta-sent-title { margin: 0 0 8px; font-weight: 700; font-size: 13px; color: #d8ffe6; }',
    '.segmenta-sent-actions { display: flex; gap: 8px; }',
    '.segmenta-sent-actions button {',
    '  flex: 1 1 0; padding: 8px 10px; border: 1px solid #3584e4; border-radius: 9px;',
    '  background: #26262c; color: #f6f6f6; cursor: pointer;',
    '  font: 600 12px/1.2 system-ui, sans-serif;',
    '}',
    '.segmenta-sent-actions button:hover { background: #30303a; }',
  ].join('\n');

  function element(document, tag, className, text) {
    const node = document.createElement(tag);
    if (className) {
      node.className = className;
    }
    if (text !== undefined && text !== null) {
      node.textContent = String(text);
    }
    return node;
  }

  function text(t, key, substitutions) {
    const value = typeof t === 'function' ? t(key, substitutions) : '';
    return typeof value === 'string' ? value : '';
  }

  function sanitize(value, max) {
    if (PROTOCOL && typeof PROTOCOL.sanitizeText === 'function') {
      return PROTOCOL.sanitizeText(value, max);
    }
    return typeof value === 'string' ? value.slice(0, max || 200) : '';
  }

  function formatBytes(bytes) {
    const numeric = Number(bytes);
    if (!Number.isFinite(numeric) || numeric < 0) {
      return '';
    }
    const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
    let value = numeric;
    let index = 0;
    while (value >= 1024 && index < units.length - 1) {
      value /= 1024;
      index += 1;
    }
    const digits = index === 0 || value >= 100 ? 0 : 1;
    return value.toFixed(digits) + ' ' + units[index];
  }

  function formatEta(seconds) {
    const numeric = Number(seconds);
    if (!Number.isFinite(numeric) || numeric < 0) {
      return '';
    }
    const total = Math.floor(numeric);
    const hours = Math.floor(total / 3600);
    const minutes = Math.floor((total % 3600) / 60);
    const secs = total % 60;
    const pad = (value) => (value < 10 ? '0' + value : String(value));
    return hours > 0 ? hours + ':' + pad(minutes) + ':' + pad(secs) : minutes + ':' + pad(secs);
  }

  function percentOf(status) {
    const direct = Number(status.percent);
    if (Number.isFinite(direct)) {
      return Math.max(0, Math.min(100, direct));
    }
    const total = Number(status.total_bytes);
    const done = Number(status.downloaded_bytes);
    if (Number.isFinite(total) && total > 0 && Number.isFinite(done) && done >= 0) {
      return Math.max(0, Math.min(100, (done / total) * 100));
    }
    return 0;
  }

  function applyStyles(root) {
    const css = STYLES;
    try {
      if (typeof CSSStyleSheet === 'function' && 'adoptedStyleSheets' in root && 'replaceSync' in CSSStyleSheet.prototype) {
        const sheet = new CSSStyleSheet();
        sheet.replaceSync(css);
        root.adoptedStyleSheets = [sheet];
        return;
      }
    } catch (err) {
      // fall through to a style element
    }
    const style = root.ownerDocument.createElement('style');
    style.textContent = css;
    root.appendChild(style);
  }

  function createPanel(options) {
    const config = options && typeof options === 'object' ? options : {};
    const t = config.t;
    const document = global.document;
    if (!document || !document.body) {
      throw new Error('segmenta: createPanel needs a document body');
    }

    const host = document.createElement('div');
    host.id = HOST_ID;
    const root = host.attachShadow({ mode: 'open' });
    applyStyles(root);

    const launcher = element(document, 'button', 'segmenta-launch', null);
    launcher.type = 'button';
    launcher.title = text(t, 'buttonTitle');
    launcher.setAttribute('aria-label', text(t, 'buttonTitle'));
    launcher.appendChild(element(document, 'span', 'segmenta-launch-label', text(t, 'buttonTitle')));

    const panel = element(document, 'section', 'segmenta-panel', null);
    panel.hidden = true;
    panel.setAttribute('role', 'dialog');
    panel.setAttribute('aria-label', text(t, 'panelTitle'));

    const head = element(document, 'header', 'segmenta-head', null);
    head.appendChild(element(document, 'span', 'segmenta-title', text(t, 'panelTitle')));
    const closeButton = element(document, 'button', 'segmenta-close', null);
    closeButton.type = 'button';
    closeButton.title = text(t, 'panelClose');
    closeButton.setAttribute('aria-label', text(t, 'panelClose'));
    closeButton.appendChild(document.createTextNode('x'));
    head.appendChild(closeButton);

    const mediaTitle = element(document, 'p', 'segmenta-media-title', '');
    const loading = element(document, 'p', 'segmenta-loading', text(t, 'panelLoading'));
    const error = element(document, 'p', 'segmenta-error', '');
    error.hidden = true;

    const targets = element(document, 'fieldset', 'segmenta-targets', null);
    targets.appendChild(element(document, 'legend', null, text(t, 'qualityLabel')));
    const formatList = element(document, 'div', 'segmenta-formats', null);

    const limitRow = element(document, 'div', 'segmenta-limit-row', null);
    limitRow.hidden = true;
    const limitInput = element(document, 'input', 'segmenta-limit', null);
    limitInput.type = 'number';
    limitInput.min = '1';
    limitInput.max = '500';
    limitInput.step = '1';
    limitInput.value = String(PROTOCOL ? PROTOCOL.DEFAULT_PLAYLIST_LIMIT : 100);
    limitInput.setAttribute('aria-label', text(t, 'playlistLimitLabel'));
    limitRow.appendChild(limitInput);
    limitRow.appendChild(element(document, 'span', 'segmenta-limit-label', text(t, 'playlistLimitLabel')));
    const hint = element(document, 'small', 'segmenta-hint', text(t, 'playlistLimitHint'));

    const startButton = element(document, 'button', 'segmenta-start', text(t, 'startDownload'));
    startButton.type = 'button';

    const progress = element(document, 'section', 'segmenta-progress', null);
    progress.hidden = true;
    const bar = element(document, 'div', 'segmenta-bar', null);
    const barFill = element(document, 'div', 'segmenta-bar-fill', null);
    bar.appendChild(barFill);
    const progressText = element(document, 'p', 'segmenta-progress-text', '');
    const playlistText = element(document, 'p', 'segmenta-playlist-text', '');
    playlistText.hidden = true;
    progress.appendChild(bar);
    progress.appendChild(progressText);
    progress.appendChild(playlistText);

    const toast = element(document, 'p', 'segmenta-toast', '');
    toast.hidden = true;

    // Handoff state: after a successful add the card body becomes this block, so the
    // format list never stays on screen (S9).
    const sent = element(document, 'section', 'segmenta-sent', null);
    sent.hidden = true;
    const sentTitle = element(document, 'p', 'segmenta-sent-title', text(t, 'sentTitle'));
    const sentActions = element(document, 'div', 'segmenta-sent-actions', null);
    const openTaskButton = element(document, 'button', 'segmenta-open-task', text(t, 'openSegmenta'));
    openTaskButton.type = 'button';
    const openProgressButton = element(document, 'button', 'segmenta-open-progress', text(t, 'viewProgress'));
    openProgressButton.type = 'button';
    sentActions.appendChild(openTaskButton);
    sentActions.appendChild(openProgressButton);
    sent.appendChild(sentTitle);
    sent.appendChild(sentActions);

    targets.appendChild(formatList);
    targets.appendChild(limitRow);
    targets.appendChild(hint);
    panel.appendChild(head);
    panel.appendChild(mediaTitle);
    panel.appendChild(loading);
    panel.appendChild(error);
    panel.appendChild(targets);
    panel.appendChild(startButton);
    panel.appendChild(sent);
    panel.appendChild(progress);
    panel.appendChild(toast);

    root.appendChild(launcher);
    root.appendChild(panel);

    const state = { options: [], selected: null, kind: 'video', count: 0, taskId: '' };

    function clearFormats() {
      while (formatList.firstChild) {
        formatList.removeChild(formatList.firstChild);
      }
      state.options = [];
      state.selected = null;
    }

    function addOption(spec) {
      const wrapper = element(document, 'label', 'segmenta-option', null);
      const input = element(document, 'input', 'segmenta-option-input', null);
      input.type = 'radio';
      input.name = 'segmenta-target';
      input.value = spec.id;
      wrapper.appendChild(input);
      wrapper.appendChild(element(document, 'span', 'segmenta-option-label', spec.label));
      input.addEventListener('change', () => {
        if (input.checked) {
          select(spec.id);
        }
      });
      formatList.appendChild(wrapper);
      state.options.push(spec);
      return input;
    }

    function select(id) {
      const found = state.options.find((option) => option.id === id);
      if (!found) {
        return;
      }
      state.selected = found;
      const needsLimit = found.mode === 'playlist';
      limitRow.hidden = !needsLimit;
      for (const input of formatList.querySelectorAll('input')) {
        input.checked = input.value === id;
      }
    }

    function renderFormats(formats) {
      clearFormats();
      error.hidden = true;
      error.textContent = '';
      toast.hidden = true;
      progress.hidden = true;
      barFill.style.width = '0%';
      progressText.textContent = '';
      playlistText.hidden = true;
      // A new probe brings the format list back and forgets the previous handoff.
      state.taskId = '';
      sent.hidden = true;
      targets.hidden = false;
      startButton.hidden = false;

      if (!formats || typeof formats !== 'object') {
        loading.hidden = false;
        mediaTitle.textContent = '';
        startButton.disabled = true;
        return;
      }

      loading.hidden = true;
      startButton.disabled = false;
      mediaTitle.textContent = sanitize(formats.title, 160);

      const playlist = formats.playlist && typeof formats.playlist === 'object' ? formats.playlist : null;
      const count = playlist && Number.isFinite(Number(playlist.count)) ? Math.floor(Number(playlist.count)) : 0;
      state.count = count;

      // Only the grouped ladder is rendered: deduped by height, capped to
      // 4K/1080p/720p/360p plus audio. No endless raw list (S9).
      const groups = PROTOCOL && typeof PROTOCOL.groupQualities === 'function'
        ? PROTOCOL.groupQualities(formats)
        : [];
      for (const group of groups) {
        const isAudio = group.mode === 'audio';
        addOption({
          id: 'quality:' + group.quality,
          label: isAudio ? text(t, 'audioOnly') || group.label : group.label,
          mode: group.mode,
          height: isAudio ? undefined : group.height || undefined,
          // Video selection is by height only; the audio mode keeps its format id.
          formatId: isAudio ? group.formatId || undefined : undefined,
        });
      }

      const audioEntry = Array.isArray(formats.audio) ? formats.audio[0] || null : null;
      if (!groups || !groups.length) {
        // Nothing grouped: keep the audio-only target reachable on its own.
        addOption({
          id: 'audio',
          label: text(t, 'audioOnly'),
          mode: 'audio',
          formatId: audioEntry ? sanitize(audioEntry.format_id, 64) || undefined : undefined,
        });
      }

      if (playlist && count > 0) {
        addOption({
          id: 'playlist',
          label: text(t, 'playlistAll', [String(count)]),
          mode: 'playlist',
        });
      }

      const streamUrl = typeof formats.stream === 'string' ? formats.stream : '';
      if (streamUrl) {
        addOption({ id: 'stream', label: text(t, 'streamDetected'), mode: 'video', url: streamUrl });
      }

      if (!state.options.length) {
        startButton.disabled = true;
        error.hidden = false;
        error.textContent = text(t, 'errorGeneric');
        return;
      }

      const fallback = state.options.find((option) => option.mode === 'video' && !option.url);
      const preferred = PROTOCOL && typeof PROTOCOL.defaultQuality === 'function'
        ? 'quality:' + PROTOCOL.defaultQuality(groups)
        : '';
      const preferredOption = preferred ? state.options.find((option) => option.id === preferred) : null;
      select(preferredOption ? preferredOption.id : fallback ? fallback.id : state.options[0].id);
    }

    function buildRequest() {
      const selected = state.selected;
      if (!selected) {
        return null;
      }
      const request = { mode: selected.mode };
      if (selected.formatId) {
        request.formatId = selected.formatId;
      }
      if (selected.height) {
        request.height = selected.height;
      }
      if (selected.url) {
        request.url = selected.url;
      }
      if (selected.mode === 'playlist') {
        request.playlistLimit = PROTOCOL
          ? PROTOCOL.clampPlaylistLimit(limitInput.value)
          : Number(limitInput.value);
      }
      return request;
    }

    function updateProgress(status) {
      if (!status || typeof status !== 'object') {
        return;
      }
      const percent = percentOf(status);
      progress.hidden = false;
      barFill.style.width = percent.toFixed(1) + '%';

      const parts = [];
      if (status.status === 'completed') {
        parts.push(text(t, 'statusCompleted'));
      } else {
        parts.push(text(t, 'statusPercent', [String(Math.floor(percent))]));
        const speed = Number(status.speed_bps);
        if (Number.isFinite(speed) && speed > 0) {
          parts.push(text(t, 'statusSpeed', [formatBytes(speed)]));
        }
        const eta = formatEta(status.eta_seconds);
        if (eta) {
          parts.push(text(t, 'statusEta', [eta]));
        }
      }
      progressText.textContent = parts.join(' | ');

      const playlist = status.playlist && typeof status.playlist === 'object' ? status.playlist : null;
      if (playlist && Number.isFinite(Number(playlist.total))) {
        playlistText.hidden = false;
        playlistText.textContent = text(t, 'statusPlaylist', [
          String(Math.floor(Number(playlist.completed) || 0)),
          String(Math.floor(Number(playlist.total))),
        ]);
      } else {
        playlistText.hidden = true;
      }

      if (status.title) {
        mediaTitle.textContent = sanitize(status.title, 160);
      }

      if (status.status === 'completed') {
        const path = sanitize(status.final_path, 400);
        toast.hidden = !path;
        toast.textContent = path ? text(t, 'toastSaved', [path]) : '';
        startButton.disabled = false;
      } else if (status.status === 'failed') {
        const message = sanitize(status.error_message, 300) || sanitize(status.error_code, 80) || text(t, 'errorGeneric');
        error.hidden = false;
        error.textContent = message;
        startButton.disabled = false;
      } else {
        startButton.disabled = true;
      }
    }

    function showError(message) {
      const clean = sanitize(message, 300) || text(t, 'errorGeneric');
      loading.hidden = true;
      error.hidden = false;
      error.textContent = clean;
      startButton.disabled = false;
    }

    function requestOpenTask() {
      const taskId = typeof state.taskId === 'string' ? state.taskId : '';
      if (!taskId || typeof config.onOpenTask !== 'function') {
        return;
      }
      config.onOpenTask(taskId);
    }

    /**
     * Replaces the card body with the "sent to Segmenta" state: the download already
     * exists, so the format list is cleared and only the two task actions remain.
     */
    function showSent(taskId) {
      const id = sanitize(taskId, 64);
      if (!id) {
        return;
      }
      state.taskId = id;
      loading.hidden = true;
      error.hidden = true;
      error.textContent = '';
      clearFormats();
      limitRow.hidden = true;
      targets.hidden = true;
      startButton.hidden = true;
      sentTitle.textContent = text(t, 'sentTitle');
      openTaskButton.textContent = text(t, 'openSegmenta');
      openProgressButton.textContent = text(t, 'viewProgress');
      sent.hidden = false;
      progress.hidden = false;
    }

    openTaskButton.addEventListener('click', requestOpenTask);
    openProgressButton.addEventListener('click', requestOpenTask);

    function open(formats) {
      panel.hidden = false;
      renderFormats(formats);
    }

    function close() {
      panel.hidden = true;
      if (typeof config.onClose === 'function') {
        config.onClose();
      }
    }

    launcher.addEventListener('click', () => {
      if (typeof config.onProbe === 'function') {
        config.onProbe();
      }
    });

    closeButton.addEventListener('click', () => {
      close();
    });

    startButton.addEventListener('click', () => {
      const request = buildRequest();
      if (!request) {
        return;
      }
      startButton.disabled = true;
      progress.hidden = false;
      barFill.style.width = '0%';
      progressText.textContent = text(t, 'statusPreparing');
      toast.hidden = true;
      if (typeof config.onStart === 'function') {
        config.onStart(request);
      }
    });

    document.body.appendChild(host);

    return {
      root: host,
      launcher: launcher,
      panel: panel,
      open: open,
      setProgress: updateProgress,
      setError: showError,
      setSent: showSent,
      close: close,
      isOpen: () => panel.hidden === false,
    };
  }

  const api = { createPanel: createPanel, HOST_ID: HOST_ID };

  global.SegmentaPanel = api;
  if (typeof module !== 'undefined' && module.exports) {
    module.exports = api;
  }
})(typeof globalThis !== 'undefined' ? globalThis : this);
