import assert from 'node:assert/strict';
import { loadShared } from './helpers.mjs';

const WATCH = 'https://www.youtube.com/watch?v=dQw4w9WgXcQ';
const PLAYLIST = 'https://www.youtube.com/playlist?list=PL1234567890abcdef';

export default function suite({ test }) {
  test('exposes the documented protocol API', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    for (const name of [
      'isAllowedUrl',
      'watchUrlFrom',
      'playlistIdFrom',
      'buildProbe',
      'buildAdd',
      'buildStatus',
      'buildSystemOpen',
      'validateHostReply',
      'clampPlaylistLimit',
      'sanitizeText',
    ]) {
      assert.equal(typeof P[name], 'function', `missing protocol.${name}`);
    }
  });

  test('isAllowedUrl accepts youtube watch and playlist only', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.equal(P.isAllowedUrl(WATCH), true);
    assert.equal(P.isAllowedUrl(PLAYLIST), true);
    assert.equal(P.isAllowedUrl('https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL1234567890abcdef'), true);
  });

  test('isAllowedUrl rejects non-youtube schemes, hosts and paths', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    for (const url of [
      'javascript:alert(1)',
      'data:text/html,<b>x</b>',
      'file:///etc/passwd',
      'http://www.youtube.com/watch?v=dQw4w9WgXcQ',
      'https://youtu.be/dQw4w9WgXcQ',
      'https://m.youtube.com/watch?v=dQw4w9WgXcQ',
      'https://www.youtube.com/',
      'https://www.youtube.com/feed/subscriptions',
      'https://www.youtube.com.evil.test/watch?v=dQw4w9WgXcQ',
      'https://user:pass@www.youtube.com/watch?v=dQw4w9WgXcQ',
      'https://example.com/live.m3u8',
      '',
      'not a url',
      null,
      42,
    ]) {
      assert.equal(P.isAllowedUrl(url), false, `should reject ${String(url)}`);
    }
  });

  test('isAllowedUrl allows https streams with an optional host allowlist', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    const stream = 'https://cdn.example.com/hls/master.m3u8';
    assert.equal(P.isAllowedUrl(stream, { allowStreams: true }), true);
    assert.equal(P.isAllowedUrl(stream, { allowStreams: true, allowlist: ['cdn.example.com'] }), true);
    assert.equal(P.isAllowedUrl(stream, { allowStreams: true, allowlist: ['other.example.com'] }), false);
    assert.equal(P.isAllowedUrl(stream, { allowStreams: false }), false);
    assert.equal(P.isAllowedUrl('javascript:alert(1)', { allowStreams: true }), false);
    assert.equal(P.isAllowedUrl('file:///tmp/x.m3u8', { allowStreams: true }), false);
    assert.equal(P.isAllowedUrl('https://youtu.be/dQw4w9WgXcQ', { allowStreams: true }), false);
  });

  test('isStreamUrl detects m3u8 and mpd manifests', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.equal(P.isStreamUrl('https://cdn.example.com/a/master.m3u8?token=1'), true);
    assert.equal(P.isStreamUrl('https://cdn.example.com/a/manifest.MPD'), true);
    assert.equal(P.isStreamUrl('https://cdn.example.com/a/video.mp4'), false);
    assert.equal(P.isStreamUrl('/relative/master.m3u8'), false);
    assert.equal(P.isStreamUrl(null), false);
  });

  test('watchUrlFrom normalizes watch URLs and rejects everything else', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.equal(P.watchUrlFrom(`${WATCH}&list=PL1234567890abcdef&t=10`), WATCH);
    assert.equal(P.watchUrlFrom('https://www.youtube.com/watch?v=short'), null);
    assert.equal(P.watchUrlFrom(PLAYLIST), null);
    assert.equal(P.watchUrlFrom('https://youtu.be/dQw4w9WgXcQ'), null);
    assert.equal(P.watchUrlFrom('https://www.youtube.com/watch'), null);
    assert.equal(P.watchUrlFrom(undefined), null);
  });

  test('playlistIdFrom extracts valid playlist ids only', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.equal(P.playlistIdFrom(PLAYLIST), 'PL1234567890abcdef');
    assert.equal(P.playlistIdFrom(`${WATCH}&list=PL1234567890abcdef`), 'PL1234567890abcdef');
    assert.equal(P.playlistIdFrom(WATCH), null);
    assert.equal(P.playlistIdFrom('https://www.youtube.com/playlist?list=../evil'), null);
    assert.equal(P.playlistIdFrom('https://www.youtube.com/playlist'), null);
  });

  test('playlistUrlFrom rebuilds a canonical playlist url', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.equal(P.playlistUrlFrom(PLAYLIST), PLAYLIST);
    assert.equal(P.playlistUrlFrom(WATCH), null);
  });

  test('clampPlaylistLimit defaults to 100 and caps at 500', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.equal(P.clampPlaylistLimit(undefined), 100);
    assert.equal(P.clampPlaylistLimit(null), 100);
    assert.equal(P.clampPlaylistLimit(0), 100);
    assert.equal(P.clampPlaylistLimit(-5), 100);
    assert.equal(P.clampPlaylistLimit('abc'), 100);
    assert.equal(P.clampPlaylistLimit(0.5), 100);
    assert.equal(P.clampPlaylistLimit(1), 1);
    assert.equal(P.clampPlaylistLimit('250'), 250);
    assert.equal(P.clampPlaylistLimit(12.9), 12);
    assert.equal(P.clampPlaylistLimit(500), 500);
    assert.equal(P.clampPlaylistLimit(501), 500);
    assert.equal(P.clampPlaylistLimit(99999), 500);
    assert.equal(Number.isInteger(P.clampPlaylistLimit(37.2)), true);
  });

  test('buildProbe builds a JSON-RPC 2.0 media.probe envelope', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    const envelope = P.buildProbe({ url: WATCH, kind: 'video', id: 7 });
    assert.deepEqual(envelope, {
      jsonrpc: '2.0',
      id: 7,
      method: 'media.probe',
      params: { url: WATCH, kind: 'video' },
    });

    const generated = P.buildProbe({ url: PLAYLIST, kind: 'playlist', limit: 250 });
    assert.equal(generated.jsonrpc, '2.0');
    assert.equal(typeof generated.id, 'number');
    assert.deepEqual(generated.params, { url: PLAYLIST, kind: 'playlist', limit: 250 });

    const stream = P.buildProbe({ url: 'https://cdn.example.com/master.m3u8', kind: 'stream', id: 9 });
    assert.deepEqual(stream.params, { url: 'https://cdn.example.com/master.m3u8', kind: 'stream' });
  });

  test('buildProbe rejects invalid kinds and urls', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.throws(() => P.buildProbe({ url: WATCH, kind: 'movie' }), /kind/i);
    assert.throws(() => P.buildProbe({ url: 'javascript:alert(1)', kind: 'video' }), /url/i);
    assert.throws(() => P.buildProbe({ url: 'https://www.youtube.com/', kind: 'video' }), /url/i);
    assert.throws(() => P.buildProbe(null), /url/i);
  });

  test('buildAdd builds the media.add envelope with optional fields', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.deepEqual(P.buildAdd({ url: WATCH, mode: 'video', id: 3 }), {
      jsonrpc: '2.0',
      id: 3,
      method: 'media.add',
      params: { url: WATCH, mode: 'video' },
    });
    assert.deepEqual(
      P.buildAdd({
        url: PLAYLIST,
        mode: 'playlist',
        playlistLimit: 501,
        height: 1080,
        id: 4,
      }).params,
      { url: PLAYLIST, mode: 'playlist', height: 1080, playlist_limit: 500 },
    );
    assert.deepEqual(P.buildAdd({ url: WATCH, mode: 'audio', formatId: '251', id: 5 }).params, {
      url: WATCH,
      mode: 'audio',
      format_id: '251',
    });
    assert.deepEqual(P.buildAdd({ url: WATCH, mode: 'video', formatId: '137+140', id: 6 }).params, {
      url: WATCH,
      mode: 'video',
      format_id: '137+140',
    });
  });

  test('buildAdd rejects invalid modes, urls, format ids and heights', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.throws(() => P.buildAdd({ url: WATCH, mode: 'both' }), /mode/i);
    assert.throws(() => P.buildAdd({ url: 'javascript:alert(1)', mode: 'video' }), /url/i);
    assert.throws(() => P.buildAdd({ url: WATCH, mode: 'video', formatId: 'a b;rm -rf' }), /format/i);
    assert.throws(() => P.buildAdd({ url: WATCH, mode: 'video', height: 99999 }), /height/i);
  });

  test('buildStatus builds the media.status envelope', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.deepEqual(P.buildStatus({ taskId: 'task-123456', id: 11 }), {
      jsonrpc: '2.0',
      id: 11,
      method: 'media.status',
      params: { task_id: 'task-123456' },
    });
    assert.throws(() => P.buildStatus({ taskId: '' }), /task/i);
    assert.throws(() => P.buildStatus({}), /task/i);
    assert.throws(() => P.buildStatus({ taskId: 'has space' }), /task/i);
  });

  test('buildSystemOpen builds the system.open envelope', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.deepEqual(P.buildSystemOpen({ taskId: 'task-123456', id: 12 }), {
      jsonrpc: '2.0',
      id: 12,
      method: 'system.open',
      params: { task_id: 'task-123456' },
    });
    const uuid = '123e4567-e89b-12d3-a456-426614174000';
    const generated = P.buildSystemOpen({ taskId: uuid });
    assert.equal(generated.method, 'system.open');
    assert.equal(typeof generated.id, 'number');
    assert.deepEqual(generated.params, { task_id: uuid });
    // Surrounding whitespace is trimmed before the id is validated.
    assert.deepEqual(P.buildSystemOpen({ taskId: '  task-123456  ', id: 13 }).params, {
      task_id: 'task-123456',
    });
  });

  test('buildSystemOpen rejects ids outside the task id pattern', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    for (const taskId of ['', '   ', 'short', 'has space', 'a;rm -rf /', '../etc/passwd', 'x'.repeat(65), null, 42]) {
      assert.throws(
        () => P.buildSystemOpen({ taskId: taskId }),
        (err) => err instanceof P.ProtocolError && err.code === 'ERR_INVALID_PARAMS',
        `should reject ${String(taskId)}`,
      );
    }
    assert.throws(() => P.buildSystemOpen({}), /task/i);
    assert.throws(() => P.buildSystemOpen(null), /task/i);
  });

  test('validateHostReply accepts a matching result reply', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    const verdict = P.validateHostReply(
      { jsonrpc: '2.0', id: 7, result: { kind: 'video', title: 'x' } },
      7,
    );
    assert.equal(verdict.ok, true);
    assert.deepEqual(verdict.result, { kind: 'video', title: 'x' });
  });

  test('validateHostReply accepts a numeric error reply', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    const verdict = P.validateHostReply(
      { jsonrpc: '2.0', id: 8, error: { code: 1040, message: 'DRM' } },
      8,
    );
    assert.equal(verdict.ok, false);
    assert.deepEqual(verdict.error, { code: 1040, message: 'DRM' });
  });

  test('validateHostReply rejects mismatched ids and malformed replies', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.throws(() => P.validateHostReply({ jsonrpc: '2.0', id: 9, result: {} }, 8), /id/i);
    assert.throws(() => P.validateHostReply({ jsonrpc: '2.0', id: 9 }, 9), /result|error/i);
    assert.throws(
      () => P.validateHostReply({ jsonrpc: '2.0', id: 9, result: {}, error: { code: 1, message: 'x' } }, 9),
      /result|error/i,
    );
    assert.throws(
      () => P.validateHostReply({ jsonrpc: '2.0', id: 9, error: { code: 'ERR_DRM', message: 'x' } }, 9),
      /code/i,
    );
    assert.throws(() => P.validateHostReply({ jsonrpc: '1.0', id: 9, result: {} }, 9), /jsonrpc/i);
    assert.throws(() => P.validateHostReply({ result: {} }, 9), /jsonrpc/i);
    assert.throws(() => P.validateHostReply(null, 9), /object|reply/i);
    assert.throws(() => P.validateHostReply('nope', 9), /object|reply/i);
  });

  test('sanitizeText strips control chars and caps length', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.equal(P.sanitizeText('  Hola\n mundo  ', 50), 'Hola mundo');
    assert.equal(P.sanitizeText('a\u0000b\u001fc', 50), 'abc');
    assert.equal(P.sanitizeText('abcdef', 3), 'abc');
    assert.equal(P.sanitizeText(null, 10), '');
    assert.equal(P.sanitizeText(42, 10), '');
    assert.equal(P.sanitizeText('ñandú', 10), 'ñandú');
    assert.equal(P.sanitizeText('x'.repeat(500), 200).length, 200);
  });

  test('errorCodeName maps numeric host codes onto stable names', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.equal(P.errorCodeName(1040), 'ERR_DRM_PROTECTED');
    assert.equal(P.errorCodeName(1044), 'ERR_MEDIA_BACKEND');
    assert.equal(P.errorCodeName('ERR_FRAME_TRUNCATED'), 'ERR_FRAME_TRUNCATED');
    assert.equal(P.errorCodeName(9999), 'ERR_UNKNOWN');
  });
}
