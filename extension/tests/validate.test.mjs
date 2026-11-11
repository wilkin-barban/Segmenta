import assert from 'node:assert/strict';
import { loadShared } from './helpers.mjs';

const EXTENSION_ID = 'abcdefghijklmnopabcdefghijklmnop';
const WATCH = 'https://www.youtube.com/watch?v=dQw4w9WgXcQ';
const PLAYLIST = 'https://www.youtube.com/playlist?list=PL1234567890abcdef';
const CONTENT_SENDER = { id: EXTENSION_ID, url: WATCH, origin: 'https://www.youtube.com' };
const POPUP_SENDER = {
  id: EXTENSION_ID,
  url: `chrome-extension://${EXTENSION_ID}/popup.html`,
  origin: `chrome-extension://${EXTENSION_ID}`,
};

export default function suite({ test }) {
  test('exposes the documented validation API', async () => {
    const V = await loadShared('SegmentaValidate', 'validate.js');
    assert.equal(typeof V.validateSender, 'function');
    assert.equal(typeof V.validateMessage, 'function');
    assert.equal(typeof V.validateInbound, 'function');
    assert.equal(V.MAX_MESSAGE_BYTES, 65536);
    assert.deepEqual([...V.MESSAGE_TYPES].sort(), [
      'segmenta.add',
      'segmenta.capture',
      'segmenta.probe',
      'segmenta.status',
      'segmenta.torrent.add',
    ]);
  });

  test('validateSender accepts this extension and rejected anything else', async () => {
    const V = await loadShared('SegmentaValidate', 'validate.js');
    assert.equal(V.validateSender(CONTENT_SENDER, EXTENSION_ID).ok, true);
    assert.equal(V.validateSender(POPUP_SENDER, EXTENSION_ID).ok, true);

    const wrongId = { ...CONTENT_SENDER, id: 'ponmlkjihgfedcbaponmlkjihgfedcba' };
    assert.equal(V.validateSender(wrongId, EXTENSION_ID).ok, false);
    assert.equal(V.validateSender(wrongId, EXTENSION_ID).error.code, 'ERR_INVALID_SENDER');

    const foreignPage = { id: EXTENSION_ID, url: 'https://evil.test/watch', origin: 'https://evil.test' };
    assert.equal(V.validateSender(foreignPage, EXTENSION_ID).ok, false);
    assert.equal(V.validateSender(foreignPage, EXTENSION_ID).error.code, 'ERR_INVALID_SENDER');

    assert.equal(V.validateSender(null, EXTENSION_ID).ok, false);
    assert.equal(V.validateSender({}, EXTENSION_ID).ok, false);
    assert.equal(V.validateSender(CONTENT_SENDER, '').ok, false);
    assert.equal(V.validateSender({ id: EXTENSION_ID }, EXTENSION_ID).ok, false);
  });

  test('validateMessage rejects unknown and missing types', async () => {
    const V = await loadShared('SegmentaValidate', 'validate.js');
    const options = { extensionId: EXTENSION_ID, allowStreams: true };
    assert.equal(V.validateMessage({ type: 'media.probe' }, options).error.code, 'ERR_INVALID_TYPE');
    assert.equal(V.validateMessage({ type: 'segmenta.eval' }, options).error.code, 'ERR_INVALID_TYPE');
    assert.equal(V.validateMessage({}, options).error.code, 'ERR_INVALID_TYPE');
    assert.equal(V.validateMessage(null, options).error.code, 'ERR_INVALID_MESSAGE');
    assert.equal(V.validateMessage([], options).error.code, 'ERR_INVALID_MESSAGE');
    assert.equal(V.validateMessage('x', options).error.code, 'ERR_INVALID_MESSAGE');
  });

  test('validateMessage rejects oversized payloads', async () => {
    const V = await loadShared('SegmentaValidate', 'validate.js');
    const options = { extensionId: EXTENSION_ID };
    const big = {
      type: 'segmenta.probe',
      url: WATCH,
      kind: 'video',
      padding: 'x'.repeat(70000),
    };
    const verdict = V.validateMessage(big, options);
    assert.equal(verdict.ok, false);
    assert.equal(verdict.error.code, 'ERR_MESSAGE_TOO_LARGE');

    const small = { type: 'segmenta.probe', url: WATCH, kind: 'video' };
    assert.equal(V.validateMessage(small, options).ok, true);
  });

  test('validateMessage normalizes a probe request', async () => {
    const V = await loadShared('SegmentaValidate', 'validate.js');
    const options = { extensionId: EXTENSION_ID };
    const verdict = V.validateMessage({ type: 'segmenta.probe', url: WATCH, kind: 'video' }, options);
    assert.equal(verdict.ok, true);
    assert.deepEqual(verdict.value, { type: 'segmenta.probe', url: WATCH, kind: 'video' });

    const withLimit = V.validateMessage(
      { type: 'segmenta.probe', url: PLAYLIST, kind: 'playlist', limit: 501 },
      options,
    );
    assert.equal(withLimit.value.limit, 500);

    const defaulted = V.validateMessage(
      { type: 'segmenta.probe', url: PLAYLIST, kind: 'playlist', limit: 0 },
      options,
    );
    assert.equal(defaulted.value.limit, 100);
  });

  test('validateMessage rejects unsafe urls and kinds', async () => {
    const V = await loadShared('SegmentaValidate', 'validate.js');
    const options = { extensionId: EXTENSION_ID };
    for (const url of [
      'javascript:alert(1)',
      'data:text/html,x',
      'file:///etc/passwd',
      'https://youtu.be/dQw4w9WgXcQ',
      'https://www.youtube.com/',
    ]) {
      assert.equal(
        V.validateMessage({ type: 'segmenta.probe', url, kind: 'video' }, options).error.code,
        'ERR_INVALID_URL',
        `probe should reject ${url}`,
      );
      assert.equal(
        V.validateMessage({ type: 'segmenta.add', url, mode: 'video' }, options).error.code,
        'ERR_INVALID_URL',
        `add should reject ${url}`,
      );
    }
    assert.equal(
      V.validateMessage({ type: 'segmenta.probe', url: WATCH, kind: 'movie' }, options).error.code,
      'ERR_INVALID_PARAMS',
    );
  });

  test('validateMessage accepts https streams only when enabled', async () => {
    const V = await loadShared('SegmentaValidate', 'validate.js');
    const stream = 'https://cdn.example.com/master.m3u8';
    const enabled = { extensionId: EXTENSION_ID, allowStreams: true };
    const disabled = { extensionId: EXTENSION_ID, allowStreams: false };

    assert.equal(
      V.validateMessage({ type: 'segmenta.probe', url: stream, kind: 'stream' }, enabled).ok,
      true,
    );
    assert.equal(
      V.validateMessage({ type: 'segmenta.probe', url: stream, kind: 'video' }, disabled).error.code,
      'ERR_INVALID_URL',
    );
    assert.equal(
      V.validateMessage(
        { type: 'segmenta.probe', url: stream, kind: 'stream' },
        { extensionId: EXTENSION_ID, allowStreams: true, allowlist: ['other.example.com'] },
      ).error.code,
      'ERR_INVALID_URL',
    );
  });

  test('validateMessage normalizes an add request', async () => {
    const V = await loadShared('SegmentaValidate', 'validate.js');
    const options = { extensionId: EXTENSION_ID };
    const verdict = V.validateMessage(
      {
        type: 'segmenta.add',
        url: PLAYLIST,
        mode: 'playlist',
        formatId: '137+140',
        height: 1080,
        playlistLimit: 999,
      },
      options,
    );
    assert.equal(verdict.ok, true);
    assert.deepEqual(verdict.value, {
      type: 'segmenta.add',
      url: PLAYLIST,
      mode: 'playlist',
      formatId: '137+140',
      height: 1080,
      playlistLimit: 500,
    });

    assert.equal(
      V.validateMessage({ type: 'segmenta.add', url: WATCH, mode: 'both' }, options).error.code,
      'ERR_INVALID_PARAMS',
    );
    assert.equal(
      V.validateMessage({ type: 'segmenta.add', url: WATCH, mode: 'video', formatId: 'rm -rf /' }, options)
        .error.code,
      'ERR_INVALID_PARAMS',
    );
    assert.equal(
      V.validateMessage({ type: 'segmenta.add', url: WATCH, mode: 'video', height: -1 }, options).error.code,
      'ERR_INVALID_PARAMS',
    );
  });

  test('validateMessage normalizes a status request', async () => {
    const V = await loadShared('SegmentaValidate', 'validate.js');
    const options = { extensionId: EXTENSION_ID };
    assert.deepEqual(
      V.validateMessage({ type: 'segmenta.status', taskId: 'task-123456' }, options).value,
      { type: 'segmenta.status', taskId: 'task-123456' },
    );
    assert.equal(
      V.validateMessage({ type: 'segmenta.status', taskId: 'a b' }, options).error.code,
      'ERR_INVALID_PARAMS',
    );
    assert.equal(V.validateMessage({ type: 'segmenta.status' }, options).error.code, 'ERR_INVALID_PARAMS');
  });

  test('validateInbound checks sender before payload', async () => {
    const V = await loadShared('SegmentaValidate', 'validate.js');
    const options = { extensionId: EXTENSION_ID, allowStreams: true };
    const good = V.validateInbound(
      { type: 'segmenta.probe', url: WATCH, kind: 'video' },
      CONTENT_SENDER,
      options,
    );
    assert.equal(good.ok, true);

    const badSender = V.validateInbound(
      { type: 'segmenta.probe', url: WATCH, kind: 'video' },
      { id: 'wrong' },
      options,
    );
    assert.equal(badSender.error.code, 'ERR_INVALID_SENDER');

    const badPayload = V.validateInbound(
      { type: 'segmenta.probe', url: 'javascript:alert(1)', kind: 'video' },
      CONTENT_SENDER,
      options,
    );
    assert.equal(badPayload.error.code, 'ERR_INVALID_URL');
  });

  test('error results are shaped for the message layer', async () => {
    const V = await loadShared('SegmentaValidate', 'validate.js');
    const verdict = V.validateMessage({ type: 'nope' }, { extensionId: EXTENSION_ID });
    assert.equal(verdict.ok, false);
    assert.equal(typeof verdict.error.code, 'string');
    assert.equal(typeof verdict.error.message, 'string');
    assert.equal(verdict.error.message.length > 0, true);
  });
}
