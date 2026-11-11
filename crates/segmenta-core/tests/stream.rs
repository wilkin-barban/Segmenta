//! Public HLS/DASH parsing and merge planning (spec S9).
//!
//! Every fixture is an in-memory string; merge plans use `tempfile`. No network and
//! no ffmpeg execution: only the argument vector and the concat plan are asserted.

use std::path::Path;

use segmenta_core::stream::dash::{parse_mpd, segments_for, select_representation};
use segmenta_core::stream::hls::{parse_master, parse_media_playlist, select_variant};
use segmenta_core::stream::merge::{ffconcat, ffconcat_with_init, ffmpeg_args};
use segmenta_core::stream::StreamError;
use url::Url;

fn url(raw: &str) -> Url {
    Url::parse(raw).expect("fixture base URL")
}

/// A `file` line of the ffconcat format: single quoted, with `'` escaped as `'\''`.
fn concat_line(path: &Path) -> String {
    let escaped = path.to_string_lossy().replace('\'', "'\\''");
    format!("file '{escaped}'")
}

const MASTER_BASE: &str = "https://cdn.example.com/media/master.m3u8";

const MASTER: &str = "#EXTM3U
#EXT-X-VERSION:3
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Espanol\",LANGUAGE=\"es\",DEFAULT=YES,AUTOSELECT=YES,URI=\"audio/es.m3u8\"
#EXT-X-STREAM-INF:BANDWIDTH=800000,AVERAGE-BANDWIDTH=700000,RESOLUTION=640x360,CODECS=\"avc1.42e01e,mp4a.40.2\"
360p/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2400000,RESOLUTION=1280x720,CODECS=\"avc1.4d401f,mp4a.40.2\"
720p/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=4800000,RESOLUTION=1920x1080,CODECS=\"avc1.640028,mp4a.40.2\"
/top/1080p/index.m3u8
";

const MEDIA: &str = "#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:6
#EXT-X-MAP:URI=\"init.mp4\"
#EXT-X-MEDIA-SEQUENCE:0
#EXTINF:6.006,first
seg0.m4s
#EXTINF:5.972,
seg1.m4s
#EXT-X-DISCONTINUITY
#EXTINF:6.006,
seg2.m4s
#EXT-X-ENDLIST
";

const MPD_TEMPLATE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- public static manifest -->
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="static" mediaPresentationDuration="PT30S" minBufferTime="PT2S">
  <BaseURL>https://cdn.example.com/dash/</BaseURL>
  <Period id="p0">
    <AdaptationSet contentType="video" mimeType="video/mp4" segmentAlignment="true">
      <SegmentTemplate timescale="1" duration="10" startNumber="1" media="v_$RepresentationID$_$Number$.m4s" initialization="v_$RepresentationID$_init.mp4"/>
      <Representation id="v360" bandwidth="800000" width="640" height="360" codecs="avc1.42e01e"/>
      <Representation id="v480" bandwidth="1600000" width="854" height="480" codecs="avc1.4d401e"/>
      <Representation id="v1080" bandwidth="5200000" width="1920" height="1080" codecs="avc1.640028"/>
    </AdaptationSet>
    <AdaptationSet contentType="audio" mimeType="audio/mp4">
      <SegmentTemplate timescale="1" duration="10" startNumber="0" media="a_$Number$.m4s" initialization="a_init.mp4"/>
      <Representation id="a1" bandwidth="128000" codecs="mp4a.40.2"/>
    </AdaptationSet>
  </Period>
</MPD>
"#;

const MPD_TIMELINE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<MPD type="static" mediaPresentationDuration="PT12S">
  <BaseURL>https://cdn.example.com/tl/</BaseURL>
  <Period id="p0">
    <AdaptationSet mimeType="video/mp4" codecs="avc1.4d401f">
      <SegmentTemplate timescale="1000" initialization="init.mp4" media="seg-$Time$.m4s">
        <SegmentTimeline>
          <S t="0" d="4000" r="2"/>
          <S d="2000"/>
        </SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v" bandwidth="1000000" width="1280" height="720"/>
    </AdaptationSet>
  </Period>
</MPD>
"#;

#[test]
fn hls_master_parses_variants_and_audio_renditions() {
    let master = parse_master(MASTER, &url(MASTER_BASE)).expect("master playlist");
    assert_eq!(master.variants.len(), 3);
    assert_eq!(master.audio.len(), 1);

    let first = &master.variants[0];
    assert_eq!(first.bandwidth, Some(800_000));
    assert_eq!(
        first.resolution.map(|r| (r.width, r.height)),
        Some((640, 360))
    );
    assert_eq!(first.codecs.as_deref(), Some("avc1.42e01e,mp4a.40.2"));
    // Relative URI resolved against the playlist URL.
    assert_eq!(
        first.uri.as_str(),
        "https://cdn.example.com/media/360p/index.m3u8"
    );

    // Second variant resolves relatively too.
    assert_eq!(
        master.variants[1].uri.as_str(),
        "https://cdn.example.com/media/720p/index.m3u8"
    );
    // Absolute URI is kept and only re-based by the scheme/host.
    assert_eq!(
        master.variants[2].uri.as_str(),
        "https://cdn.example.com/top/1080p/index.m3u8"
    );

    let audio = &master.audio[0];
    assert_eq!(audio.kind.as_deref(), Some("AUDIO"));
    assert_eq!(audio.group_id.as_deref(), Some("aud"));
    assert_eq!(audio.name.as_deref(), Some("Espanol"));
    assert_eq!(audio.language.as_deref(), Some("es"));
    assert!(audio.is_default);
    assert_eq!(
        audio.uri.as_ref().map(Url::as_str),
        Some("https://cdn.example.com/media/audio/es.m3u8")
    );

    // A master without variants or `#EXTM3U` never panics.
    let empty = parse_master("#EXTM3U\n", &url(MASTER_BASE)).expect("empty master");
    assert!(empty.variants.is_empty());
    assert!(select_variant(&empty, None).is_none());
    assert!(select_variant(&empty, Some(720)).is_none());
    assert!(matches!(
        parse_master("<html>not a playlist</html>", &url(MASTER_BASE)),
        Err(StreamError::Malformed(_))
    ));
}

#[test]
fn hls_master_selects_variant_by_height() {
    let master = parse_master(MASTER, &url(MASTER_BASE)).expect("master playlist");

    let highest = select_variant(&master, None).expect("highest by default");
    assert_eq!(highest.height(), Some(1080));

    let exact = select_variant(&master, Some(720)).expect("exact height");
    assert_eq!(exact.height(), Some(720));
    assert_eq!(
        exact.uri.as_str(),
        "https://cdn.example.com/media/720p/index.m3u8"
    );

    // Smallest height >= requested when there is no exact match.
    let above = select_variant(&master, Some(1000)).expect("smallest above");
    assert_eq!(above.height(), Some(1080));

    // Nothing >= requested: fall back to the highest available.
    let fallback = select_variant(&master, Some(4000)).expect("fallback highest");
    assert_eq!(fallback.height(), Some(1080));

    let below = select_variant(&master, Some(240)).expect("smallest above the floor");
    assert_eq!(below.height(), Some(360));
}

#[test]
fn hls_media_playlist_parses_extinf_map_endlist_and_discontinuity() {
    let media = parse_media_playlist(MEDIA, &url("https://cdn.example.com/media/720p/index.m3u8"))
        .expect("media playlist");

    assert_eq!(
        media.init.as_ref().map(Url::as_str),
        Some("https://cdn.example.com/media/720p/init.mp4")
    );
    assert_eq!(media.segments.len(), 3);
    assert_eq!(
        media.segments[0].uri.as_str(),
        "https://cdn.example.com/media/720p/seg0.m4s"
    );
    assert_eq!(media.segments[0].duration, Some(6.006));
    assert_eq!(media.segments[1].duration, Some(5.972));
    assert!(!media.segments[0].discontinuity);
    assert!(!media.segments[1].discontinuity);
    // The discontinuity tag applies to the following segment.
    assert!(media.segments[2].discontinuity);
    assert_eq!(media.target_duration, Some(6.0));
}

#[test]
fn hls_playlist_without_endlist_is_live() {
    let live = MEDIA.replace("#EXT-X-ENDLIST\n", "");
    assert!(!live.contains("#EXT-X-ENDLIST"));
    assert!(matches!(
        parse_media_playlist(&live, &url(MASTER_BASE)),
        Err(StreamError::LiveNotSupported)
    ));
    assert_eq!(StreamError::LiveNotSupported.code(), 1041);
    assert_eq!(
        StreamError::LiveNotSupported.err_name(),
        "ERR_LIVE_NOT_SUPPORTED"
    );
}

#[test]
fn hls_encrypted_keys_are_drm_but_method_none_is_fine() {
    let aes = MEDIA.replace(
        "#EXT-X-ENDLIST\n",
        "#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x0123\n#EXT-X-ENDLIST\n",
    );
    let err = parse_media_playlist(&aes, &url(MASTER_BASE)).expect_err("AES-128 is rejected");
    assert!(matches!(err, StreamError::DrmProtected));
    assert_eq!(err.code(), 1040);
    assert_eq!(err.err_name(), "ERR_DRM_PROTECTED");

    let sample_aes = MEDIA.replace(
        "#EXT-X-ENDLIST\n",
        "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n#EXT-X-ENDLIST\n",
    );
    assert!(matches!(
        parse_media_playlist(&sample_aes, &url(MASTER_BASE)),
        Err(StreamError::DrmProtected)
    ));

    let clear = MEDIA.replace(
        "#EXT-X-ENDLIST\n",
        "#EXT-X-KEY:METHOD=NONE\n#EXT-X-ENDLIST\n",
    );
    let parsed = parse_media_playlist(&clear, &url(MASTER_BASE)).expect("METHOD=NONE is clear");
    assert_eq!(parsed.segments.len(), 3);
}

#[test]
fn dash_segment_template_duration_produces_numbered_segments() {
    let manifest = parse_mpd(
        MPD_TEMPLATE,
        &url("https://cdn.example.com/dash/manifest.mpd"),
    )
    .expect("static MPD");
    assert_eq!(manifest.periods.len(), 1);
    assert!((manifest.duration_seconds.unwrap_or_default() - 30.0).abs() < 1e-6);
    assert_eq!(manifest.periods[0].adaptations.len(), 2);

    let video = segments_for(&manifest, "v360").expect("template segments");
    assert_eq!(
        video.init.as_ref().map(Url::as_str),
        Some("https://cdn.example.com/dash/v_v360_init.mp4")
    );
    let media: Vec<&str> = video.media.iter().map(Url::as_str).collect();
    assert_eq!(
        media,
        vec![
            "https://cdn.example.com/dash/v_v360_1.m4s",
            "https://cdn.example.com/dash/v_v360_2.m4s",
            "https://cdn.example.com/dash/v_v360_3.m4s",
        ]
    );

    // `startNumber="0"` shifts the first segment number.
    let audio = segments_for(&manifest, "a1").expect("audio segments");
    let audio_media: Vec<&str> = audio.media.iter().map(Url::as_str).collect();
    assert_eq!(
        audio_media,
        vec![
            "https://cdn.example.com/dash/a_0.m4s",
            "https://cdn.example.com/dash/a_1.m4s",
            "https://cdn.example.com/dash/a_2.m4s",
        ]
    );
    assert_eq!(
        audio.init.as_ref().map(Url::as_str),
        Some("https://cdn.example.com/dash/a_init.mp4")
    );

    assert!(matches!(
        segments_for(&manifest, "missing"),
        Err(StreamError::Unsupported(_))
    ));
}

#[test]
fn dash_segment_timeline_expands_repeats() {
    let manifest = parse_mpd(
        MPD_TIMELINE,
        &url("https://cdn.example.com/tl/manifest.mpd"),
    )
    .expect("timeline MPD");
    let plan = segments_for(&manifest, "v").expect("timeline segments");
    assert_eq!(
        plan.init.as_ref().map(Url::as_str),
        Some("https://cdn.example.com/tl/init.mp4")
    );
    let media: Vec<&str> = plan.media.iter().map(Url::as_str).collect();
    // `t="0" d="4000" r="2"` repeats three times; the trailing `S d="2000"` adds one.
    assert_eq!(
        media,
        vec![
            "https://cdn.example.com/tl/seg-0.m4s",
            "https://cdn.example.com/tl/seg-4000.m4s",
            "https://cdn.example.com/tl/seg-8000.m4s",
            "https://cdn.example.com/tl/seg-12000.m4s",
        ]
    );
}

#[test]
fn dash_content_protection_is_drm_and_dynamic_is_live() {
    let drm = MPD_TEMPLATE.replace(
        "<Representation id=\"v360\" bandwidth=\"800000\" width=\"640\" height=\"360\" codecs=\"avc1.42e01e\"/>",
        "<ContentProtection schemeIdUri=\"urn:mpeg:dash:mp4protection:2011\" value=\"cenc\"/>\n      <Representation id=\"v360\" bandwidth=\"800000\" width=\"640\" height=\"360\" codecs=\"avc1.42e01e\"/>",
    );
    let err = parse_mpd(&drm, &url("https://cdn.example.com/dash/manifest.mpd"))
        .expect_err("ContentProtection is DRM");
    assert!(matches!(err, StreamError::DrmProtected));
    assert_eq!(err.code(), 1040);

    let live = MPD_TEMPLATE.replace("type=\"static\"", "type=\"dynamic\"");
    assert!(matches!(
        parse_mpd(&live, &url("https://cdn.example.com/dash/manifest.mpd")),
        Err(StreamError::LiveNotSupported)
    ));

    let truncated = MPD_TEMPLATE.replace("</MPD>", "");
    assert!(matches!(
        parse_mpd(
            &truncated,
            &url("https://cdn.example.com/dash/manifest.mpd")
        ),
        Err(StreamError::Malformed(_))
    ));
    assert_eq!(StreamError::Malformed(String::new()).code(), 1042);
    assert_eq!(StreamError::Unsupported(String::new()).code(), 1043);
    assert_eq!(
        StreamError::Malformed(String::new()).err_name(),
        "ERR_MANIFEST_MALFORMED"
    );
    assert_eq!(
        StreamError::Unsupported(String::new()).err_name(),
        "ERR_STREAM_UNSUPPORTED"
    );
}

#[test]
fn dash_representation_selection_matches_the_hls_rule() {
    let manifest = parse_mpd(
        MPD_TEMPLATE,
        &url("https://cdn.example.com/dash/manifest.mpd"),
    )
    .expect("static MPD");

    assert_eq!(
        select_representation(&manifest, Some(480)).map(|rep| rep.id.as_str()),
        Some("v480")
    );
    // No exact height: the smallest one above wins.
    assert_eq!(
        select_representation(&manifest, Some(400)).map(|rep| rep.id.as_str()),
        Some("v480")
    );
    // Nothing above: the highest wins.
    assert_eq!(
        select_representation(&manifest, Some(4000)).map(|rep| rep.id.as_str()),
        Some("v1080")
    );
    assert_eq!(
        select_representation(&manifest, None).map(|rep| rep.id.as_str()),
        Some("v1080")
    );
    // Audio-only representations are never selected as video.
    assert_ne!(
        select_representation(&manifest, None).map(|rep| rep.id.as_str()),
        Some("a1")
    );
}

#[test]
fn merge_plan_puts_the_init_segment_first_and_escapes_quotes() {
    let dir = tempfile::tempdir().expect("temp dir");
    let init = dir.path().join("init.mp4");
    let first = dir.path().join("seg 0.m4s");
    let second = dir.path().join("it's.m4s");
    for path in [&init, &first, &second] {
        std::fs::write(path, b"x").expect("fixture file");
    }

    let plan = ffconcat_with_init(Some(&init), &[first.clone(), second.clone()]);
    let lines: Vec<&str> = plan.lines().collect();
    assert_eq!(lines[0], "ffconcat version 1.0");
    assert_eq!(lines[1], concat_line(&init));
    assert_eq!(lines[2], concat_line(&first));
    // A single quote inside the path is escaped so it cannot close the value.
    assert_eq!(lines[3], concat_line(&second));
    assert!(
        lines[3].contains("'\\''"),
        "the quote must be escaped: {}",
        lines[3]
    );
    assert_eq!(lines.len(), 4);

    // Without an init segment the fragments keep their order.
    let plain = ffconcat(&[first.clone(), second.clone()]);
    let plain_lines: Vec<&str> = plain.lines().collect();
    assert_eq!(plain_lines[0], "ffconcat version 1.0");
    assert_eq!(plain_lines[1], concat_line(&first));
    assert_eq!(plain_lines[2], concat_line(&second));
    assert_eq!(plain_lines.len(), 3);
    assert_eq!(ffconcat(&[]).trim_end(), "ffconcat version 1.0");
}

#[test]
fn ffmpeg_args_are_separate_arguments_without_a_shell() {
    let concat = Path::new("/tmp/plan.ffconcat");
    let output = Path::new("/tmp/out.mp4");

    assert_eq!(
        ffmpeg_args(concat, output, None),
        vec![
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            "/tmp/plan.ffconcat",
            "-c",
            "copy",
            "/tmp/out.mp4",
        ]
    );

    let audio = Path::new("/tmp/audio.m4a");
    let with_audio = ffmpeg_args(concat, output, Some(audio));
    assert_eq!(
        with_audio,
        vec![
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            "/tmp/plan.ffconcat",
            "-c",
            "copy",
            "-i",
            "/tmp/audio.m4a",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c:a",
            "copy",
            "/tmp/out.mp4",
        ]
    );

    // Absolute order: inputs, then the video mapping, then the audio mapping, then output.
    let position = |needle: &str| with_audio.iter().position(|arg| arg == needle).unwrap();
    assert!(position("-i") < position("0:v:0"));
    assert!(position("0:v:0") < position("1:a:0"));
    assert_eq!(with_audio.last().map(String::as_str), Some("/tmp/out.mp4"));
}

#[test]
fn merge_paths_never_introduce_shell_metacharacters() {
    let dir = tempfile::tempdir().expect("temp dir");
    let hostile = dir.path().join("a b $(touch pwned); `id` 'q' | &&.m4s");
    std::fs::write(&hostile, b"x").expect("fixture file");
    let init = dir.path().join("init segment.mp4");
    std::fs::write(&init, b"x").expect("fixture file");

    let plan = ffconcat_with_init(Some(&init), std::slice::from_ref(&hostile));
    let lines: Vec<&str> = plan.lines().collect();
    assert_eq!(
        lines.len(),
        3,
        "one line per segment, never split by the name"
    );
    assert!(lines[1].starts_with("file '") && lines[1].ends_with('\''));

    let output = dir.path().join("out put.mp4");
    let args = ffmpeg_args(&init, &output, Some(&hostile));
    // The path is handed over as one argv element, byte for byte.
    let concat_arg = args
        .iter()
        .position(|arg| arg == &init.to_string_lossy().to_string())
        .expect("concat path is a single argument");
    assert_eq!(args[concat_arg - 1], "-i");
    let audio_arg = args
        .iter()
        .position(|arg| arg == &hostile.to_string_lossy().to_string())
        .expect("audio path is a single argument");
    assert_eq!(args[audio_arg - 1], "-i");
    assert_eq!(args.last(), Some(&output.to_string_lossy().to_string()));
    // No shell wrapper and no composite command line argument. `-c copy` is ffmpeg's
    // own codec flag, so it is not in this list.
    for forbidden in ["sh", "bash", "-exec", "&&", "||", ";", "|", "start", "cmd"] {
        assert!(
            !args.iter().any(|arg| arg == forbidden),
            "argument `{forbidden}` must never be produced"
        );
    }
}

#[test]
fn stream_error_codes_are_stable() {
    for (error, code, name) in [
        (StreamError::DrmProtected, 1040, "ERR_DRM_PROTECTED"),
        (
            StreamError::LiveNotSupported,
            1041,
            "ERR_LIVE_NOT_SUPPORTED",
        ),
        (
            StreamError::Malformed(String::new()),
            1042,
            "ERR_MANIFEST_MALFORMED",
        ),
        (
            StreamError::Unsupported(String::new()),
            1043,
            "ERR_STREAM_UNSUPPORTED",
        ),
    ] {
        assert_eq!(error.code(), code);
        assert_eq!(error.err_name(), name);
        assert!(!error.to_string().is_empty());
    }
}
