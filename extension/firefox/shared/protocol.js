// Segmenta shared protocol helpers (pure: no DOM, no browser APIs).
//
// This file is a classic script on purpose: the very same bytes are loaded as a
// content script, imported by the Chromium service worker through importScripts,
// listed in the Firefox background scripts and imported by the Node tests. It
// publishes its API on globalThis.SegmentaProtocol and never assumes a bundler.
(function (global) {
  'use strict';

  const YOUTUBE_HOST = 'www.youtube.com';
  const YOUTUBE_BASE = 'https://' + YOUTUBE_HOST;
  // Short and mobile YouTube hosts are never accepted: only the canonical
  // watch/playlist pages of the desktop host are in scope.
  const DENIED_HOSTS = ['youtu.be', 'www.youtu.be', 'm.youtube.com'];
  const YOUTUBE_PATHS = ['/watch', '/playlist'];
  const PROBE_KINDS = ['video', 'playlist', 'stream'];
  const ADD_MODES = ['video', 'audio', 'playlist'];
  const FORMAT_ID_PATTERN = /^[A-Za-z0-9._+-]{1,64}$/;
  const TASK_ID_PATTERN = /^[A-Za-z0-9_-]{6,64}$/;
  const MAX_URL_LENGTH = 2048;
  const DEFAULT_PLAYLIST_LIMIT = 100;
  const MAX_PLAYLIST_LIMIT = 500;
  const MAX_HEIGHT = 4320;
  const MAX_TEXT_LENGTH = 200;
  const STREAM_PATTERN = /\.(m3u8|mpd)$/i;
  const HOST_ERROR_CODES = {
    1040: 'ERR_DRM_PROTECTED',
    1041: 'ERR_LIVE_NOT_SUPPORTED',
    1042: 'ERR_MANIFEST_MALFORMED',
    1043: 'ERR_STREAM_UNSUPPORTED',
    1044: 'ERR_MEDIA_BACKEND',
    1045: 'ERR_MEDIA_NOT_PUBLIC',
  };

  let sequence = 0;

  class ProtocolError extends Error {
    constructor(code, message) {
      super(message || code);
      this.name = 'ProtocolError';
      this.code = code;
    }
  }

  function nextId() {
    sequence += 1;
    return sequence;
  }

  function parseUrl(value) {
    if (typeof value !== 'string') {
      return null;
    }
    const raw = value.trim();
    if (!raw || raw.length > MAX_URL_LENGTH) {
      return null;
    }
    let parsed;
    try {
      parsed = new URL(raw);
    } catch (err) {
      return null;
    }
    if (parsed.protocol !== 'https:') {
      return null;
    }
    if (parsed.username || parsed.password) {
      return null;
    }
    return parsed;
  }

  function normalizedHost(parsed) {
    return parsed.hostname.toLowerCase();
  }

  function allowlistSet(allowlist) {
    if (!Array.isArray(allowlist)) {
      return null;
    }
    const set = new Set();
    for (const entry of allowlist) {
      if (typeof entry === 'string' && entry.trim()) {
        set.add(entry.trim().toLowerCase());
      }
    }
    return set;
  }

  /**
   * Accepts canonical YouTube watch/playlist URLs. With allowStreams it also
   * accepts any https URL whose host is in the optional allowlist (or any https
   * host when no allowlist is supplied). Non https schemes are always rejected.
   */
  function isAllowedUrl(value, options) {
    const opts = options || {};
    const parsed = parseUrl(value);
    if (!parsed) {
      return false;
    }
    const host = normalizedHost(parsed);
    if (!host || DENIED_HOSTS.indexOf(host) !== -1) {
      return false;
    }
    if (host === YOUTUBE_HOST) {
      return YOUTUBE_PATHS.indexOf(parsed.pathname) !== -1;
    }
    if (opts.allowStreams !== true) {
      return false;
    }
    const allowed = allowlistSet(opts.allowlist);
    if (!allowed || allowed.size === 0) {
      return true;
    }
    return allowed.has(host);
  }

  function isStreamUrl(value) {
    const parsed = parseUrl(value);
    if (!parsed) {
      return false;
    }
    return STREAM_PATTERN.test(parsed.pathname);
  }

  function watchUrlFrom(href) {
    const parsed = parseUrl(href);
    if (!parsed || normalizedHost(parsed) !== YOUTUBE_HOST || parsed.pathname !== '/watch') {
      return null;
    }
    const videoId = parsed.searchParams.get('v') || '';
    if (!/^[A-Za-z0-9_-]{11}$/.test(videoId)) {
      return null;
    }
    return YOUTUBE_BASE + '/watch?v=' + videoId;
  }

  function playlistIdFrom(href) {
    const parsed = parseUrl(href);
    if (!parsed || normalizedHost(parsed) !== YOUTUBE_HOST) {
      return null;
    }
    if (YOUTUBE_PATHS.indexOf(parsed.pathname) === -1) {
      return null;
    }
    const listId = parsed.searchParams.get('list') || '';
    if (!/^[A-Za-z0-9_-]{10,64}$/.test(listId)) {
      return null;
    }
    return listId;
  }

  function playlistUrlFrom(href) {
    const listId = playlistIdFrom(href);
    return listId ? YOUTUBE_BASE + '/playlist?list=' + listId : null;
  }

  function clampPlaylistLimit(value) {
    const numeric = typeof value === 'string' ? Number(value.trim()) : Number(value);
    if (!Number.isFinite(numeric)) {
      return DEFAULT_PLAYLIST_LIMIT;
    }
    const floored = Math.floor(numeric);
    if (floored < 1) {
      return DEFAULT_PLAYLIST_LIMIT;
    }
    return floored > MAX_PLAYLIST_LIMIT ? MAX_PLAYLIST_LIMIT : floored;
  }

  function clampHeight(value) {
    const numeric = typeof value === 'string' ? Number(value.trim()) : Number(value);
    if (!Number.isFinite(numeric)) {
      return null;
    }
    const floored = Math.floor(numeric);
    if (floored < 1 || floored > MAX_HEIGHT) {
      return null;
    }
    return floored;
  }

  function envelope(method, params, id) {
    const numericId = typeof id === 'number' && Number.isFinite(id) ? id : nextId();
    return { jsonrpc: '2.0', id: numericId, method: method, params: params };
  }

  function requireUrl(value, options) {
    if (!isAllowedUrl(value, options)) {
      throw new ProtocolError('ERR_INVALID_URL', 'url rejected by the extension allowlist');
    }
    return value.trim();
  }

  function buildProbe(input) {
    const request = input && typeof input === 'object' ? input : {};
    const kind = request.kind;
    const url = requireUrl(request.url, {
      allowStreams: kind === 'stream',
      allowlist: request.allowlist,
    });
    if (PROBE_KINDS.indexOf(kind) === -1) {
      throw new ProtocolError('ERR_INVALID_PARAMS', 'probe kind is not supported');
    }
    const params = { url: url, kind: kind };
    if (request.limit !== undefined && request.limit !== null) {
      params.limit = clampPlaylistLimit(request.limit);
    }
    return envelope('media.probe', params, request.id);
  }

  function buildAdd(input) {
    const request = input && typeof input === 'object' ? input : {};
    const url = requireUrl(request.url, { allowStreams: true, allowlist: request.allowlist });
    if (ADD_MODES.indexOf(request.mode) === -1) {
      throw new ProtocolError('ERR_INVALID_PARAMS', 'add mode is not supported');
    }
    const params = { url: url, mode: request.mode };
    if (request.formatId !== undefined && request.formatId !== null && request.formatId !== '') {
      const formatId = String(request.formatId);
      if (!FORMAT_ID_PATTERN.test(formatId)) {
        throw new ProtocolError('ERR_INVALID_PARAMS', 'format id is not acceptable');
      }
      params.format_id = formatId;
    }
    if (request.height !== undefined && request.height !== null && request.height !== '') {
      const height = clampHeight(request.height);
      if (height === null) {
        throw new ProtocolError('ERR_INVALID_PARAMS', 'height is out of range');
      }
      params.height = height;
    }
    if (request.playlistLimit !== undefined && request.playlistLimit !== null) {
      params.playlist_limit = clampPlaylistLimit(request.playlistLimit);
    }
    return envelope('media.add', params, request.id);
  }

  function buildStatus(input) {
    const request = input && typeof input === 'object' ? input : {};
    const taskId = typeof request.taskId === 'string' ? request.taskId.trim() : '';
    if (!TASK_ID_PATTERN.test(taskId)) {
      throw new ProtocolError('ERR_INVALID_PARAMS', 'task id is not acceptable');
    }
    return envelope('media.status', { task_id: taskId }, request.id);
  }

  /**
   * Validates a JSON-RPC 2.0 reply coming from the native host.
   * Returns {ok:true, id, result} or {ok:false, id, error:{code, message}}.
   * Throws ProtocolError when the envelope itself is not trustworthy.
   */
  function validateHostReply(value, expectedId) {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
      throw new ProtocolError('ERR_PROTOCOL_INVALID', 'reply is not a JSON-RPC object');
    }
    if (value.jsonrpc !== '2.0') {
      throw new ProtocolError('ERR_PROTOCOL_INVALID', 'reply jsonrpc field is invalid');
    }
    const id = value.id;
    if (typeof id !== 'number' && typeof id !== 'string') {
      throw new ProtocolError('ERR_PROTOCOL_INVALID', 'reply id must be a number or string');
    }
    if (expectedId !== undefined && expectedId !== null && id !== expectedId) {
      throw new ProtocolError('ERR_PROTOCOL_ID_MISMATCH', 'reply id does not match the request id');
    }
    const hasResult = Object.prototype.hasOwnProperty.call(value, 'result');
    const hasError = Object.prototype.hasOwnProperty.call(value, 'error');
    if (hasResult === hasError) {
      throw new ProtocolError('ERR_PROTOCOL_INVALID', 'reply must carry either result or error');
    }
    if (hasResult) {
      return { ok: true, id: id, result: value.result };
    }
    const error = value.error;
    if (!error || typeof error !== 'object' || Array.isArray(error)) {
      throw new ProtocolError('ERR_PROTOCOL_INVALID', 'reply error must be an object');
    }
    if (typeof error.code !== 'number' || !Number.isFinite(error.code)) {
      throw new ProtocolError('ERR_PROTOCOL_INVALID', 'reply error code must be a number');
    }
    return {
      ok: false,
      id: id,
      error: {
        code: error.code,
        message: typeof error.message === 'string' ? sanitizeText(error.message, 300) : '',
      },
    };
  }

  /**
   * Removes control characters, collapses whitespace and caps the length of any
   * text that ends up rendered in the panel.
   */
  function sanitizeText(value, max) {
    if (typeof value !== 'string') {
      return '';
    }
    const limit = Number.isFinite(Number(max)) && Number(max) > 0 ? Math.floor(Number(max)) : MAX_TEXT_LENGTH;
    let out = '';
    const normalized = typeof value.normalize === 'function' ? value.normalize('NFC') : value;
    for (const character of normalized) {
      const code = character.codePointAt(0);
      if (code === 0x09 || code === 0x0a || code === 0x0d) {
        out += ' ';
        continue;
      }
      if (code < 0x20 || code === 0x7f) {
        continue;
      }
      if (code >= 0x80 && code <= 0x9f) {
        continue;
      }
      if (code === 0x200b || code === 0x200e || code === 0x200f || code === 0xfeff) {
        continue;
      }
      out += character;
    }
    out = out.replace(/\s+/g, ' ').trim();
    if (out.length > limit) {
      out = out.slice(0, limit).trimEnd();
    }
    return out;
  }

  function errorCodeName(code) {
    if (typeof code === 'string' && code) {
      return code;
    }
    if (typeof code === 'number' && HOST_ERROR_CODES[code]) {
      return HOST_ERROR_CODES[code];
    }
    return 'ERR_UNKNOWN';
  }

  const api = {
    YOUTUBE_HOST: YOUTUBE_HOST,
    DEFAULT_PLAYLIST_LIMIT: DEFAULT_PLAYLIST_LIMIT,
    MAX_PLAYLIST_LIMIT: MAX_PLAYLIST_LIMIT,
    PROBE_KINDS: PROBE_KINDS.slice(),
    ADD_MODES: ADD_MODES.slice(),
    HOST_ERROR_CODES: Object.assign({}, HOST_ERROR_CODES),
    ProtocolError: ProtocolError,
    nextId: nextId,
    isAllowedUrl: isAllowedUrl,
    isStreamUrl: isStreamUrl,
    watchUrlFrom: watchUrlFrom,
    playlistIdFrom: playlistIdFrom,
    playlistUrlFrom: playlistUrlFrom,
    clampPlaylistLimit: clampPlaylistLimit,
    clampHeight: clampHeight,
    buildProbe: buildProbe,
    buildAdd: buildAdd,
    buildStatus: buildStatus,
    validateHostReply: validateHostReply,
    sanitizeText: sanitizeText,
    errorCodeName: errorCodeName,
  };

  global.SegmentaProtocol = api;
  if (typeof module !== 'undefined' && module.exports) {
    module.exports = api;
  }
})(typeof globalThis !== 'undefined' ? globalThis : this);
