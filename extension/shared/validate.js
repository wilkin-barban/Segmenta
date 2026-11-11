// Segmenta inbound message validation (pure: no Chrome/Firefox APIs).
//
// The background service worker runs every message through validateInbound()
// before touching the native host. The rules are pure functions so they can be
// tested in Node and reused by both browser targets.
(function (global) {
  'use strict';

  const PROTOCOL = global.SegmentaProtocol;
  if (!PROTOCOL) {
    throw new Error('shared/protocol.js must be loaded before shared/validate.js');
  }

  const MAX_MESSAGE_BYTES = 65536;
  const MESSAGE_TYPES = ['segmenta.probe', 'segmenta.add', 'segmenta.status'];
  const MESSAGE_MODES = ['video', 'audio', 'playlist'];
  const PROBE_KINDS = ['video', 'playlist', 'stream'];
  const FORMAT_ID_PATTERN = /^[A-Za-z0-9._+-]{1,64}$/;
  const TASK_ID_PATTERN = /^[A-Za-z0-9_-]{6,64}$/;
  const MAX_HEIGHT = 4320;

  function fail(code, message) {
    return { ok: false, error: { code: code, message: message } };
  }

  function ok(value) {
    return { ok: true, value: value };
  }

  function utf8Length(text) {
    if (typeof TextEncoder === 'function') {
      return new TextEncoder().encode(text).length;
    }
    return unescape(encodeURIComponent(text)).length;
  }

  /**
   * A message is only trusted when it comes from this extension. Content script
   * messages keep id === extension id and carry the page url; popup messages
   * carry the extension origin.
   */
  function validateSender(sender, extensionId) {
    if (!sender || typeof sender !== 'object' || Array.isArray(sender)) {
      return fail('ERR_INVALID_SENDER', 'message has no sender');
    }
    if (typeof extensionId !== 'string' || !extensionId) {
      return fail('ERR_INVALID_SENDER', 'extension id is not available');
    }
    if (sender.id !== extensionId) {
      return fail('ERR_INVALID_SENDER', 'sender id does not match this extension');
    }
    const originField = typeof sender.origin === 'string' ? sender.origin : '';
    const urlField = typeof sender.url === 'string' ? sender.url : '';
    const anyOrigin = originField || urlField;
    if (!anyOrigin) {
      return fail('ERR_INVALID_SENDER', 'sender origin is missing');
    }
    if (originField.indexOf('chrome-extension://' + extensionId + '/') === 0) {
      return ok(true);
    }
    if (urlField.indexOf('chrome-extension://' + extensionId + '/') === 0) {
      return ok(true);
    }
    if (originField.indexOf('moz-extension://') === 0 || urlField.indexOf('moz-extension://') === 0) {
      return ok(true);
    }
    // Content scripts report the page url; only allowed pages may talk to us.
    if (PROTOCOL.isAllowedUrl(urlField || anyOrigin)) {
      return ok(true);
    }
    return fail('ERR_INVALID_SENDER', 'sender origin is not allowed');
  }

  function validateProbe(message, options) {
    if (PROBE_KINDS.indexOf(message.kind) === -1) {
      return fail('ERR_INVALID_PARAMS', 'probe kind is not supported');
    }
    const allowStreams = options.allowStreams === true || message.kind === 'stream';
    const url = typeof message.url === 'string' ? message.url.trim() : '';
    if (!PROTOCOL.isAllowedUrl(url, { allowStreams: allowStreams, allowlist: options.allowlist })) {
      return fail('ERR_INVALID_URL', 'probe url is not allowed');
    }
    const value = { type: 'segmenta.probe', url: url, kind: message.kind };
    if (message.limit !== undefined && message.limit !== null) {
      value.limit = PROTOCOL.clampPlaylistLimit(message.limit);
    }
    return ok(value);
  }

  function validateAdd(message, options) {
    if (MESSAGE_MODES.indexOf(message.mode) === -1) {
      return fail('ERR_INVALID_PARAMS', 'add mode is not supported');
    }
    const url = typeof message.url === 'string' ? message.url.trim() : '';
    if (!PROTOCOL.isAllowedUrl(url, { allowStreams: true, allowlist: options.allowlist })) {
      return fail('ERR_INVALID_URL', 'add url is not allowed');
    }
    const value = { type: 'segmenta.add', url: url, mode: message.mode };
    if (message.formatId !== undefined && message.formatId !== null && message.formatId !== '') {
      const formatId = String(message.formatId);
      if (!FORMAT_ID_PATTERN.test(formatId)) {
        return fail('ERR_INVALID_PARAMS', 'format id is not acceptable');
      }
      value.formatId = formatId;
    }
    if (message.height !== undefined && message.height !== null && message.height !== '') {
      const height = Number(message.height);
      if (!Number.isInteger(height) || height < 1 || height > MAX_HEIGHT) {
        return fail('ERR_INVALID_PARAMS', 'height is out of range');
      }
      value.height = height;
    }
    if (message.playlistLimit !== undefined && message.playlistLimit !== null) {
      value.playlistLimit = PROTOCOL.clampPlaylistLimit(message.playlistLimit);
    }
    return ok(value);
  }

  function validateStatus(message) {
    const taskId = typeof message.taskId === 'string' ? message.taskId.trim() : '';
    if (!TASK_ID_PATTERN.test(taskId)) {
      return fail('ERR_INVALID_PARAMS', 'task id is not acceptable');
    }
    return ok({ type: 'segmenta.status', taskId: taskId });
  }

  function validateMessage(message, options) {
    const opts = options && typeof options === 'object' ? options : {};
    if (!message || typeof message !== 'object' || Array.isArray(message)) {
      return fail('ERR_INVALID_MESSAGE', 'message must be an object');
    }
    let serialized;
    try {
      serialized = JSON.stringify(message);
    } catch (err) {
      return fail('ERR_INVALID_MESSAGE', 'message is not serializable');
    }
    if (typeof serialized !== 'string') {
      return fail('ERR_INVALID_MESSAGE', 'message is not serializable');
    }
    if (utf8Length(serialized) > MAX_MESSAGE_BYTES) {
      return fail('ERR_MESSAGE_TOO_LARGE', 'message exceeds the size limit');
    }
    if (typeof message.type !== 'string' || MESSAGE_TYPES.indexOf(message.type) === -1) {
      return fail('ERR_INVALID_TYPE', 'message type is not allowed');
    }
    switch (message.type) {
      case 'segmenta.probe':
        return validateProbe(message, opts);
      case 'segmenta.add':
        return validateAdd(message, opts);
      case 'segmenta.status':
        return validateStatus(message);
      default:
        return fail('ERR_INVALID_TYPE', 'message type is not allowed');
    }
  }

  function validateInbound(message, sender, options) {
    const opts = options && typeof options === 'object' ? options : {};
    const senderCheck = validateSender(sender, opts.extensionId);
    if (!senderCheck.ok) {
      return senderCheck;
    }
    return validateMessage(message, opts);
  }

  const api = {
    MAX_MESSAGE_BYTES: MAX_MESSAGE_BYTES,
    MESSAGE_TYPES: MESSAGE_TYPES.slice(),
    MESSAGE_MODES: MESSAGE_MODES.slice(),
    validateSender: validateSender,
    validateMessage: validateMessage,
    validateInbound: validateInbound,
  };

  global.SegmentaValidate = api;
  if (typeof module !== 'undefined' && module.exports) {
    module.exports = api;
  }
})(typeof globalThis !== 'undefined' ? globalThis : this);
