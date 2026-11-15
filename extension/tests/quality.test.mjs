import assert from 'node:assert/strict';
import { loadShared } from './helpers.mjs';

function videoFormat(id, height) {
  return { format_id: id, height: height, ext: 'mp4', kind: 'video', label: height + 'p' };
}

function audioFormat(id) {
  return { format_id: id, ext: 'm4a', kind: 'audio' };
}

const PROBE = {
  title: 'Demo',
  formats: [
    videoFormat('137', 1080),
    videoFormat('248', 1080),
    videoFormat('22', 720),
    videoFormat('18b', 720),
    videoFormat('135', 480),
    videoFormat('134', 360),
    videoFormat('160', 240),
  ],
  audio: [audioFormat('251'), audioFormat('140')],
  playlist: { count: 0, entries: [] },
};

export default function suite({ test }) {
  test('groupQualities dedupes by height and keeps the fixed ladder order', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    const grouped = P.groupQualities(PROBE);
    assert.deepEqual(
      grouped.map((entry) => entry.quality),
      ['1080', '720', '360', 'audio'],
    );
    // The first (best) candidate for a height survives the dedupe.
    assert.equal(grouped[0].formatId, '137');
    assert.equal(grouped[1].formatId, '22');
    assert.equal(grouped[0].label, '1080p');
    assert.equal(grouped[0].mode, 'video');
    assert.equal(grouped[0].height, 1080);
    assert.equal(grouped[3].mode, 'audio');
    assert.equal(grouped[3].height, null);
    assert.equal(grouped[3].formatId, '251');
  });

  test('groupQualities caps the list at the documented five options', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    const grouped = P.groupQualities({
      formats: [
        videoFormat('313', 2160),
        videoFormat('137', 1080),
        videoFormat('22', 720),
        videoFormat('134', 360),
        videoFormat('135', 480),
        videoFormat('160', 240),
        videoFormat('17', 144),
      ],
      audio: [audioFormat('140')],
    });
    assert.deepEqual(
      grouped.map((entry) => entry.quality),
      ['2160', '1080', '720', '360', 'audio'],
    );
    assert.equal(grouped.length, 5);
    assert.equal(grouped[0].label, '4K');
  });

  test('groupQualities accepts a raw array, drops audio without a track and ignores junk', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.deepEqual(P.groupQualities([videoFormat('22', 720)]).map((entry) => entry.quality), [
      '720',
    ]);
    assert.deepEqual(P.groupQualities([]), []);
    assert.deepEqual(P.groupQualities(null), []);
    assert.deepEqual(P.groupQualities([{ format_id: 'x' }, null, 42]), []);
    // Only heights from the ladder are ever offered, in descending order.
    const shuffled = P.groupQualities([
      videoFormat('134', 360),
      videoFormat('22', 720),
      videoFormat('137', 1080),
    ]);
    assert.deepEqual(
      shuffled.map((entry) => entry.quality),
      ['1080', '720', '360'],
    );
  });

  test('groupQualities prefers the ladder shipped by the daemon', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    const grouped = P.groupQualities({
      formats: [videoFormat('22', 720), videoFormat('137', 1080)],
      audio: [audioFormat('251')],
      qualities: [
        { quality: '1080', label: '1080p', height: 1080, format_id: '399' },
        { quality: '720', label: '720p', height: 720, format_id: '22' },
        { quality: 'audio', label: 'audio', height: null, format_id: '251' },
      ],
    });
    assert.deepEqual(
      grouped.map((entry) => entry.quality),
      ['1080', '720', 'audio'],
    );
    // The daemon chose the muxed/merged candidate, so its id wins over the raw list.
    assert.equal(grouped[0].formatId, '399');
  });

  test('defaultQuality always returns 720 when it is available', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    assert.equal(P.defaultQuality(P.groupQualities(PROBE)), '720');
    assert.equal(
      P.defaultQuality([
        { quality: '2160', height: 2160 },
        { quality: '720', height: 720 },
      ]),
      '720',
    );
    assert.equal(P.DEFAULT_QUALITY_HEIGHT, 720);
    assert.deepEqual(P.QUALITY_HEIGHTS, [2160, 1080, 720, 360]);
  });

  test('defaultQuality falls back to the closest height and then to audio', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    // 360 is closer to 720 than 2160 is.
    assert.equal(
      P.defaultQuality([
        { quality: '2160', height: 2160 },
        { quality: '360', height: 360 },
      ]),
      '360',
    );
    // A single high option is still closer than nothing.
    assert.equal(P.defaultQuality([{ quality: '1080', height: 1080 }]), '1080');
    assert.equal(P.defaultQuality([{ quality: '2160', height: 2160 }]), '2160');
    assert.equal(P.defaultQuality([{ quality: 'audio', height: null }]), 'audio');
    assert.equal(P.defaultQuality([]), '');
    assert.equal(P.defaultQuality(null), '');
  });

  // YouTube ships real heights, not the nominal ladder: `FBhSudzU_ck` probes
  // 1184/888/592/394/296/196/128 and the daemon ladder only carries audio. Grouping
  // by exact height found no video at all and fell back to audio-only.
  test('groupQualities tiers real heights instead of matching them exactly', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    const probe = {
      title: 'Mix',
      formats: [
        videoFormat('399', 1184),
        videoFormat('398', 888),
        videoFormat('397', 592),
        videoFormat('18', 394),
        videoFormat('136', 296),
        videoFormat('135', 196),
        videoFormat('160', 128),
      ],
      audio: [audioFormat('251')],
      // The daemon grouped nothing but audio for this video.
      qualities: [{ quality: 'audio', label: 'audio', height: null, format_id: '251' }],
      playlist: { count: 0, entries: [] },
    };
    const grouped = P.groupQualities(probe);
    assert.deepEqual(
      grouped.map((entry) => entry.quality),
      ['1080', '720', '360', 'audio'],
    );
    // The real height and the chosen format id survive, so the request stays valid.
    const video = grouped.filter((entry) => entry.mode === 'video');
    assert.deepEqual(
      video.map((entry) => entry.height),
      [1184, 888, 592],
    );
    assert.deepEqual(
      video.map((entry) => entry.formatId),
      ['399', '398', '397'],
    );
    assert.deepEqual(
      video.map((entry) => entry.label),
      ['1080p', '720p', '360p'],
    );
    // 720 is the tier, not the real height.
    assert.equal(P.defaultQuality(grouped), '720');
    assert.equal(grouped[grouped.length - 1].mode, 'audio');
  });

  test('groupQualities tiers near-ladder sizes like 1920x888 and 1280x592', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    const grouped = P.groupQualities({
      formats: [videoFormat('398', 888), videoFormat('397', 592)],
      audio: [audioFormat('251')],
    });
    assert.deepEqual(
      grouped.map((entry) => [entry.quality, entry.height, entry.formatId]),
      [
        ['720', 888, '398'],
        ['360', 592, '397'],
        ['audio', null, '251'],
      ],
    );
    assert.equal(P.defaultQuality(grouped), '720');
    // The first candidate of a tier wins, and a second one never adds a tier.
    const repeated = P.groupQualities({
      formats: [videoFormat('a1', 900), videoFormat('a2', 1080), videoFormat('a3', 1184)],
    });
    assert.deepEqual(
      repeated.map((entry) => entry.quality),
      ['1080'],
    );
    assert.equal(repeated[0].formatId, 'a1');
    assert.equal(repeated[0].height, 900);
  });

  test('the audio-only probe defaults to the audio target', async () => {
    const P = await loadShared('SegmentaProtocol', 'protocol.js');
    const grouped = P.groupQualities({ formats: [], audio: [audioFormat('140')] });
    assert.deepEqual(
      grouped.map((entry) => entry.quality),
      ['audio'],
    );
    assert.equal(P.defaultQuality(grouped), 'audio');
  });
}
