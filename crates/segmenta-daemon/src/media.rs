//! Media backends for 0.1.0: `yt-dlp` for YouTube and the HLS/DASH pipelines.
//!
//! Everything here is process-based but shell-free: arguments are passed as a `Vec`
//! and never interpolated into a string. The probe is pure parsing over `yt-dlp -J`
//! output (testable with a fake binary) and the download runner streams progress lines
//! from the child into the event bus.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// `yt-dlp`/`ffmpeg` missing, failed or outdated.
pub const ERR_MEDIA_BACKEND: i64 = 1044;
/// Private, age-restricted or unavailable content.
pub const ERR_MEDIA_NOT_PUBLIC: i64 = 1045;

/// One selectable format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaFormat {
    /// Backend identifier (`format_id`).
    pub format_id: String,
    /// Human label (`1080p`, `m4a`).
    pub label: String,
    /// Vertical resolution when known.
    pub height: Option<u32>,
    /// Container extension.
    pub ext: String,
    /// `video` or `audio`.
    pub kind: String,
}

/// Heights offered by the grouped quality ladder (S9).
///
/// The panel shows at most these four video options plus one audio-only entry;
/// anything else the backend reports is dropped from the ladder (the raw
/// `formats` list keeps every distinct height for programmatic callers).
pub const QUALITY_HEIGHTS: [u32; 4] = [2160, 1080, 720, 360];

/// Height preselected whenever it is available (S9).
pub const DEFAULT_QUALITY_HEIGHT: u32 = 720;

/// Label for one grouped quality: `4K` for 2160, `{height}p` otherwise.
pub fn quality_label(height: u32) -> String {
    if height == 2160 {
        "4K".to_string()
    } else {
        format!("{height}p")
    }
}

/// One entry of the grouped `qualities` ladder (S9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityOption {
    /// `2160`, `1080`, `720`, `360` or `audio`.
    pub quality: String,
    /// Human label (`4K`, `1080p`, `audio`).
    pub label: String,
    /// Vertical resolution; `None` for the audio-only entry.
    pub height: Option<u32>,
    /// Backend id of the selected candidate: the muxed/DASH winner for a height, the
    /// highest bitrate for audio.
    pub format_id: String,
}

/// Result of `media.probe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeOutcome {
    /// `video`, `playlist` or `stream`.
    pub kind: String,
    /// Title shown by the panel.
    pub title: String,
    /// Duration in seconds when known.
    pub duration_seconds: Option<u64>,
    /// Thumbnail URL when known.
    pub thumbnail: Option<String>,
    /// Video formats, best candidate per height, highest first.
    pub formats: Vec<MediaFormat>,
    /// Audio formats, highest bitrate first.
    pub audio: Vec<MediaFormat>,
    /// Grouped quality ladder: 2160, 1080, 720, 360 and `audio` when available.
    pub qualities: Vec<QualityOption>,
    /// Quality the panel preselects: 720 when available, else the closest height,
    /// else `audio`.
    pub default_quality: String,
    /// Playlist summary.
    pub playlist: PlaylistSummary,
}

/// The deduplicated format lists plus the ladder built from them (S9).
#[derive(Debug, Clone)]
pub struct GroupedProbe {
    /// Video formats with a single best candidate per height, highest first.
    pub formats: Vec<MediaFormat>,
    /// Audio formats, highest bitrate first.
    pub audio: Vec<MediaFormat>,
    /// Ordered quality ladder.
    pub qualities: Vec<QualityOption>,
    /// Preselected quality tag.
    pub default_quality: String,
}

/// Playlist count plus a bounded sample of entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlaylistSummary {
    /// Total public entries.
    pub count: usize,
    /// First entries, capped.
    pub entries: Vec<PlaylistEntry>,
}

/// One playlist entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistEntry {
    /// Entry title.
    pub title: String,
    /// Entry URL.
    pub url: String,
    /// Duration in seconds when known.
    pub duration_seconds: Option<u64>,
}

/// Failure with a stable code for the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaError {
    /// Numeric JSON-RPC code.
    pub code: i64,
    /// `ERR_*` identifier.
    pub name: &'static str,
    /// Human message.
    pub message: String,
}

impl MediaError {
    /// Backend failure.
    pub fn backend(message: impl Into<String>) -> MediaError {
        MediaError {
            code: ERR_MEDIA_BACKEND,
            name: "ERR_MEDIA_BACKEND",
            message: message.into(),
        }
    }

    /// Content that is not public.
    pub fn not_public(message: impl Into<String>) -> MediaError {
        MediaError {
            code: ERR_MEDIA_NOT_PUBLIC,
            name: "ERR_MEDIA_NOT_PUBLIC",
            message: message.into(),
        }
    }

    /// Invalid parameters.
    pub fn invalid(message: impl Into<String>) -> MediaError {
        MediaError {
            code: -32602,
            name: "INVALID_PARAMS",
            message: message.into(),
        }
    }
}

/// Default playlist limit (S7).
pub const DEFAULT_PLAYLIST_LIMIT: usize = 100;
/// Hard playlist limit (S7).
pub const MAX_PLAYLIST_LIMIT: usize = 500;
/// Playlist entries returned by a probe.
const PROBE_ENTRY_SAMPLE: usize = 200;

/// Clamp the playlist limit to the documented 1..=500 range.
pub fn clamp_limit(limit: Option<u32>) -> Result<usize, MediaError> {
    match limit {
        None => Ok(DEFAULT_PLAYLIST_LIMIT),
        Some(0) => Err(MediaError::invalid("playlist limit must be at least 1")),
        Some(value) => Ok((value as usize).min(MAX_PLAYLIST_LIMIT)),
    }
}

/// Whether the URL is an allowed public media URL.
///
/// `allow_local` mirrors the SSRF setting: with it enabled, loopback `http://` fixtures
/// are accepted (tests and explicit user opt-in); public media is https only.
pub fn validate_media_url_with(raw: &str, allow_local: bool) -> Result<url::Url, MediaError> {
    let parsed = url::Url::parse(raw).map_err(|err| MediaError::invalid(err.to_string()))?;
    let loopback = matches!(
        parsed.host_str().unwrap_or_default(),
        "127.0.0.1" | "localhost" | "::1"
    );
    let accepted = match parsed.scheme() {
        "https" => true,
        "http" => allow_local && loopback,
        _ => false,
    };
    if !accepted {
        return Err(MediaError::invalid("media URLs must use https"));
    }
    let host = parsed.host_str().unwrap_or_default();
    if host.is_empty() {
        return Err(MediaError::invalid("media URL has no host"));
    }
    if parsed.username() != "" || parsed.password().is_some() {
        return Err(MediaError::invalid("media URLs must not embed credentials"));
    }
    Ok(parsed)
}

/// https-only variant used by callers without settings access.
pub fn validate_media_url(raw: &str) -> Result<url::Url, MediaError> {
    validate_media_url_with(raw, false)
}

/// Whether the host is a YouTube host.
pub fn is_youtube(host: &str) -> bool {
    host == "www.youtube.com" || host == "youtube.com" || host == "m.youtube.com"
}

/// One parsed backend format with the extra fields only the ranking needs.
struct FormatCandidate {
    format: MediaFormat,
    /// Carries both a video and an audio track in one file.
    muxed: bool,
    /// DASH-style video-only stream.
    dash: bool,
    /// Bits per second used to break ties.
    bitrate: u64,
}

impl FormatCandidate {
    /// Muxed first, then DASH video-only, then anything else.
    fn video_rank(&self) -> u8 {
        if self.muxed {
            0
        } else if self.dash {
            1
        } else {
            2
        }
    }
}

/// Highest declared bitrate of a format, `0` when the backend omits it.
fn candidate_bitrate(format: &Value) -> u64 {
    for key in ["tbr", "abr", "vbr", "bitrate"] {
        if let Some(value) = format.get(key).and_then(Value::as_f64) {
            if value > 0.0 {
                return value.round() as u64;
            }
        }
    }
    0
}

fn classify_formats(info: &Value) -> (Vec<MediaFormat>, Vec<MediaFormat>) {
    let mut video: Vec<FormatCandidate> = Vec::new();
    let mut audio: Vec<FormatCandidate> = Vec::new();
    let Some(entries) = info.get("formats").and_then(Value::as_array) else {
        return (Vec::new(), Vec::new());
    };
    for format in entries {
        let id = format
            .get("format_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let vcodec = format
            .get("vcodec")
            .and_then(Value::as_str)
            .unwrap_or("none");
        let acodec = format
            .get("acodec")
            .and_then(Value::as_str)
            .unwrap_or("none");
        let protocol = format
            .get("protocol")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let ext = format.get("ext").and_then(Value::as_str).unwrap_or("bin");
        let height = format
            .get("height")
            .and_then(Value::as_u64)
            .map(|value| value as u32);
        let has_video = vcodec != "none";
        let has_audio = acodec != "none";
        let item = MediaFormat {
            format_id: id.to_string(),
            label: match (height, has_video) {
                (Some(height), true) => format!("{height}p"),
                _ => ext.to_string(),
            },
            height,
            ext: ext.to_string(),
            kind: if has_video { "video" } else { "audio" }.to_string(),
        };
        let candidate = FormatCandidate {
            format: item,
            muxed: has_video && has_audio,
            // A video-only stream is the DASH branch even when yt-dlp reports an
            // `https` protocol for it.
            dash: protocol.contains("dash") || !has_audio,
            bitrate: candidate_bitrate(format),
        };
        if has_video {
            video.push(candidate);
        } else if has_audio {
            audio.push(candidate);
        }
    }
    // Highest height first; within one height the muxed format wins, then the DASH
    // video-only one, then the higher bitrate.
    video.sort_by(|left, right| {
        right
            .format
            .height
            .cmp(&left.format.height)
            .then_with(|| left.video_rank().cmp(&right.video_rank()))
            .then_with(|| right.bitrate.cmp(&left.bitrate))
            .then_with(|| left.format.format_id.cmp(&right.format.format_id))
    });
    audio.sort_by(|left, right| {
        right
            .bitrate
            .cmp(&left.bitrate)
            .then_with(|| left.format.format_id.cmp(&right.format.format_id))
    });
    (
        video
            .into_iter()
            .map(|candidate| candidate.format)
            .collect(),
        audio
            .into_iter()
            .map(|candidate| candidate.format)
            .collect(),
    )
}

/// Build the grouped ladder from an already ordered format list (S9).
fn quality_ladder(formats: &[MediaFormat], audio: &[MediaFormat]) -> Vec<QualityOption> {
    let mut ladder = Vec::new();
    for height in QUALITY_HEIGHTS {
        let Some(found) = formats.iter().find(|format| format.height == Some(height)) else {
            continue;
        };
        ladder.push(QualityOption {
            quality: height.to_string(),
            label: quality_label(height),
            height: Some(height),
            format_id: found.format_id.clone(),
        });
    }
    if let Some(best) = audio.first() {
        ladder.push(QualityOption {
            quality: "audio".to_string(),
            label: "audio".to_string(),
            height: None,
            format_id: best.format_id.clone(),
        });
    }
    ladder
}

/// `720` when available, else the closest available height, else `audio`.
///
/// Equidistant heights resolve towards the higher quality (the ladder is ordered
/// highest first, so the first minimum wins).
pub fn default_quality(qualities: &[QualityOption]) -> String {
    if let Some(exact) = qualities
        .iter()
        .find(|quality| quality.height == Some(DEFAULT_QUALITY_HEIGHT))
    {
        return exact.quality.clone();
    }
    if let Some(height) = qualities
        .iter()
        .filter_map(|quality| quality.height)
        .min_by_key(|height| height.abs_diff(DEFAULT_QUALITY_HEIGHT))
    {
        return height.to_string();
    }
    qualities
        .iter()
        .find(|quality| quality.quality == "audio")
        .map(|quality| quality.quality.clone())
        .unwrap_or_default()
}

/// Deduplicate the video formats by height and cap the ladder to the fixed set.
///
/// The caller orders the input best first, so the first format seen for a height is
/// the one kept: muxed, then DASH video-only, then the higher bitrate.
pub fn group_probe(formats: Vec<MediaFormat>, audio: Vec<MediaFormat>) -> GroupedProbe {
    let mut seen: Vec<u32> = Vec::new();
    let mut deduped = Vec::with_capacity(formats.len());
    for format in formats {
        if let Some(height) = format.height {
            if seen.contains(&height) {
                continue;
            }
            seen.push(height);
        }
        deduped.push(format);
    }
    let qualities = quality_ladder(&deduped, &audio);
    let default_quality = default_quality(&qualities);
    GroupedProbe {
        formats: deduped,
        audio,
        qualities,
        default_quality,
    }
}

/// Group an already serialized stream probe (HLS/DASH) in place (S9).
///
/// Manifest variants arrive in manifest order and may repeat a height, so the same
/// grouping the YouTube probe uses is applied here; streams carry no separate audio
/// list, so the ladder stays video-only.
pub fn annotate_stream_probe(probe: &mut Value) {
    let formats: Vec<MediaFormat> = probe
        .get("formats")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    let audio: Vec<MediaFormat> = probe
        .get("audio")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    let grouped = group_probe(formats, audio);
    let Some(map) = probe.as_object_mut() else {
        return;
    };
    let formats = serde_json::to_value(&grouped.formats).unwrap_or(Value::Null);
    let qualities = serde_json::to_value(&grouped.qualities).unwrap_or(Value::Null);
    map.insert("formats".to_string(), formats);
    map.insert("qualities".to_string(), qualities);
    map.insert(
        "default_quality".to_string(),
        Value::String(grouped.default_quality),
    );
}

/// Parse the `yt-dlp -J` payload into a [`ProbeOutcome`].
pub fn parse_probe_json(info: &Value, kind: &str) -> Result<ProbeOutcome, MediaError> {
    let title = info
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("media")
        .to_string();
    let duration_seconds = info.get("duration").and_then(Value::as_u64);
    let thumbnail = info
        .get("thumbnail")
        .and_then(Value::as_str)
        .map(str::to_string);
    let (formats, audio) = classify_formats(info);
    let grouped = group_probe(formats, audio);

    let mut playlist = PlaylistSummary::default();
    if let Some(entries) = info.get("entries").and_then(Value::as_array) {
        playlist.count = entries.len();
        for entry in entries.iter().take(PROBE_ENTRY_SAMPLE) {
            playlist.entries.push(PlaylistEntry {
                title: entry
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("entry")
                    .to_string(),
                url: entry
                    .get("webpage_url")
                    .or_else(|| entry.get("url"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                duration_seconds: entry.get("duration").and_then(Value::as_u64),
            });
        }
    }

    let kind = match kind {
        "playlist" => "playlist",
        "stream" => "stream",
        _ => {
            if playlist.count > 0 {
                "playlist"
            } else {
                "video"
            }
        }
    };

    if grouped.formats.is_empty() && grouped.audio.is_empty() && playlist.count == 0 {
        return Err(MediaError::not_public(
            "the media has no downloadable public formats",
        ));
    }

    Ok(ProbeOutcome {
        kind: kind.to_string(),
        title,
        duration_seconds,
        thumbnail,
        formats: grouped.formats,
        audio: grouped.audio,
        qualities: grouped.qualities,
        default_quality: grouped.default_quality,
        playlist,
    })
}

/// Classify a `yt-dlp` failure into a stable media error.
pub fn classify_backend_failure(stderr: &str, status: Option<i32>) -> MediaError {
    let lowered = stderr.to_lowercase();
    if lowered.contains("private")
        || lowered.contains("sign in")
        || lowered.contains("confirm your age")
        || lowered.contains("age-restricted")
        || lowered.contains("members-only")
    {
        return MediaError::not_public("the media is not public");
    }
    if lowered.contains("no such file") || stderr.contains("yt-dlp: not found") {
        return MediaError::backend(
            "yt-dlp is not installed; install it with `sudo apt install yt-dlp`",
        );
    }
    let detail = stderr.lines().last().unwrap_or("yt-dlp failed").trim();
    MediaError::backend(format!("{detail} (exit {})", status.unwrap_or(-1)))
}

/// Locate a tool in `PATH`, honouring `SEGMENTA_<NAME>` overrides (tests use them).
pub fn locate_tool(name: &str) -> Result<PathBuf, MediaError> {
    let override_key = format!("SEGMENTA_{}", name.to_uppercase().replace('-', "_"));
    if let Some(path) = std::env::var_os(&override_key) {
        let path = PathBuf::from(path);
        return Ok(path);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(MediaError::backend(format!(
        "{name} is not installed; install it with `sudo apt install {name}`"
    )))
}

/// Run `yt-dlp -J` and parse the result.
pub async fn probe_with(
    tool: &Path,
    url: &str,
    kind: &str,
    limit: Option<usize>,
) -> Result<ProbeOutcome, MediaError> {
    let mut command = Command::new(tool);
    command.arg("--no-config").arg("--no-warnings");
    // A full `-J` walk fails when any later entry is private or unavailable, so the
    // probe stops at the requested playlist bound (`--playlist-end N`).
    if let Some(limit) = limit.filter(|_| kind == "playlist") {
        command.arg("--playlist-end").arg(limit.to_string());
    }
    let output = command
        .arg("-J")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|err| MediaError::backend(format!("cannot run {}: {err}", tool.display())))?;
    if !output.status.success() {
        return Err(classify_backend_failure(
            &String::from_utf8_lossy(&output.stderr),
            output.status.code(),
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| MediaError::backend(format!("yt-dlp returned invalid JSON: {err}")))?;
    parse_probe_json(&value, kind)
}

/// Turn a backend title into a safe single path component.
pub fn sanitize_title(title: &str) -> String {
    let mut cleaned = String::with_capacity(title.len());
    for ch in title.chars() {
        match ch {
            '/' | '\\' | '\0' => cleaned.push('_'),
            c if c.is_control() => {}
            c => cleaned.push(c),
        }
    }
    let cleaned = cleaned.trim().trim_matches('.').trim();
    let cleaned = if cleaned.is_empty() { "media" } else { cleaned };
    if cleaned.len() <= 120 {
        return cleaned.to_string();
    }
    let mut end = 120;
    while end > 0 && !cleaned.is_char_boundary(end) {
        end -= 1;
    }
    cleaned[..end].to_string()
}

/// Kind of media task stored in `downloads.download_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaTaskKind {
    /// Single YouTube video.
    YoutubeVideo,
    /// Whole public playlist.
    YoutubePlaylist,
    /// Audio only.
    YoutubeAudio,
    /// HLS manifest.
    Hls,
    /// DASH manifest.
    Dash,
}

impl MediaTaskKind {
    /// Value stored in `download_type`.
    pub fn as_str(self) -> &'static str {
        match self {
            MediaTaskKind::YoutubeVideo => "youtube_video",
            MediaTaskKind::YoutubePlaylist => "youtube_playlist",
            MediaTaskKind::YoutubeAudio => "youtube_audio",
            MediaTaskKind::Hls => "hls",
            MediaTaskKind::Dash => "dash",
        }
    }

    /// Parse a stored value.
    pub fn parse(value: &str) -> Option<MediaTaskKind> {
        Some(match value {
            "youtube_video" => MediaTaskKind::YoutubeVideo,
            "youtube_playlist" => MediaTaskKind::YoutubePlaylist,
            "youtube_audio" => MediaTaskKind::YoutubeAudio,
            "hls" => MediaTaskKind::Hls,
            "dash" => MediaTaskKind::Dash,
            _ => return None,
        })
    }

    /// Whether the task downloads a playlist.
    pub fn is_playlist(self) -> bool {
        matches!(self, MediaTaskKind::YoutubePlaylist)
    }
}

/// Arguments for one `yt-dlp` download.
#[derive(Debug, Clone)]
pub struct DownloadPlan {
    /// Task kind.
    pub kind: MediaTaskKind,
    /// Source URL.
    pub url: String,
    /// Output template (`-o`).
    pub output: PathBuf,
    /// Selected `format_id` or selector expression.
    pub format: Option<String>,
    /// Playlist limit.
    pub playlist_limit: usize,
    /// Requested vertical resolution for video mode.
    pub height: Option<u32>,
}

/// Preference for muxed (video+audio) formats.
///
/// YouTube answers 403 to many DASH video-only formats from datacenter IPs, so video
/// mode asks for the best single file that already carries both streams; `18` is the
/// progressive fallback YouTube still serves.
pub fn muxed_selector(height: Option<u32>) -> String {
    // Muxed first; YouTube increasingly ships video-only DASH, so fall back to a
    // video+audio merge before giving up (`398+251` was the only working pair in the
    // live QA). Plain `b[...]` alone fails with "Requested format is not available".
    match height {
        Some(height) => {
            format!("b[height<={height}]/18/bv*[height<={height}]+ba/b[height<={height}]/best")
        }
        None => "b/18/bv*+ba/best".to_string(),
    }
}

/// Minimum `yt-dlp` release known to work against the current YouTube.
pub const MIN_YTDLP_VERSION: &str = "2026.10.01";

/// Parse a `yt-dlp --version` string (`2026.10.26`).
pub fn parse_version(raw: &str) -> Option<(u32, u32, u32)> {
    let mut parts = raw.trim().split('.');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    Some((year, month, day))
}

/// Compare a yt-dlp version against [`MIN_YTDLP_VERSION`].
pub fn version_is_recent(raw: &str) -> bool {
    let (Some(current), Some(minimum)) = (parse_version(raw), parse_version(MIN_YTDLP_VERSION))
    else {
        return true;
    };
    current >= minimum
}

impl DownloadPlan {
    /// Build the argument vector, never a shell string.
    pub fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "--no-config".into(),
            "--newline".into(),
            "--no-part".into(),
            "--no-continue".into(),
            "--progress".into(),
            "--progress-template".into(),
            "download:%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.speed)s|%(progress.eta)s".into(),
            "-o".into(),
            self.output.to_string_lossy().into_owned(),
        ];
        match self.kind {
            MediaTaskKind::YoutubeAudio => {
                args.push("-f".into());
                args.push(
                    self.format
                        .clone()
                        .unwrap_or_else(|| "bestaudio/best".to_string()),
                );
                args.push("--no-playlist".into());
            }
            MediaTaskKind::YoutubeVideo => {
                args.push("-f".into());
                args.push(muxed_selector(self.height));
                args.push("--merge-output-format".into());
                args.push("mp4".into());
                args.push("--no-playlist".into());
            }
            MediaTaskKind::YoutubePlaylist => {
                args.push("-f".into());
                args.push(muxed_selector(self.height));
                args.push("--merge-output-format".into());
                args.push("mp4".into());
                args.push("--yes-playlist".into());
                args.push("--playlist-end".into());
                args.push(self.playlist_limit.to_string());
            }
            MediaTaskKind::Hls | MediaTaskKind::Dash => {
                // Manifests are handled by the native pipeline, not yt-dlp.
            }
        }
        args.push(self.url.clone());
        args
    }
}

/// One parsed progress line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressLine {
    /// Bytes downloaded.
    pub downloaded: u64,
    /// Total bytes when known.
    pub total: Option<u64>,
    /// Instant speed.
    pub speed: u64,
    /// Remaining seconds.
    pub eta: Option<u64>,
}

/// Parse one `download:...` progress line emitted by the template above.
pub fn parse_progress_line(line: &str) -> Option<ProgressLine> {
    // yt-dlp does not always print the literal `download:` prefix: with the
    // `--progress-template` we use, a real 2026.10.26 run emitted bare lines such as
    // `261120|10202210|36387144.43|0`. Requiring the prefix meant no progress at all and a
    // bar frozen at 0 until the task completed. Both shapes are accepted; the pipe in the
    // first field keeps unrelated numeric output out.
    let line = line.trim();
    let payload = line.strip_prefix("download:").unwrap_or(line);
    if !payload.contains('|') {
        return None;
    }
    let mut parts = payload.split('|');
    let downloaded = parts.next()?.trim().parse::<u64>().ok()?;
    let total = parts
        .next()
        .map(str::trim)
        .filter(|value| *value != "NA" && *value != "None")
        .and_then(|value| value.parse::<u64>().ok());
    let speed = parts
        .next()
        .map(str::trim)
        .filter(|value| *value != "NA" && *value != "None")
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| value as u64)
        .unwrap_or(0);
    let eta = parts
        .next()
        .map(str::trim)
        .filter(|value| *value != "NA" && *value != "None")
        .and_then(|value| value.parse::<u64>().ok());
    Some(ProgressLine {
        downloaded,
        total,
        speed,
        eta,
    })
}

/// Stream a `yt-dlp` download, forwarding progress through `on_progress`.
pub async fn run_download(
    tool: &Path,
    plan: &DownloadPlan,
    cancel: segmenta_core::engine::Canceller,
    mut on_progress: impl FnMut(ProgressLine),
) -> Result<(), MediaError> {
    let mut child = Command::new(tool)
        .args(plan.args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| MediaError::backend(format!("cannot run {}: {err}", tool.display())))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| MediaError::backend("no stdout"))?;
    let stderr = child.stderr.take();

    // `--progress-template` lines go to stderr while stdout carries the destination
    // lines, so both streams are consumed and classified here.
    let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<(bool, String)>(256);
    {
        let sink = line_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if sink.send((false, line)).await.is_err() {
                    return;
                }
            }
        });
    }
    if let Some(stderr) = stderr {
        let sink = line_tx;
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if sink.send((true, line)).await.is_err() {
                    return;
                }
            }
        });
    }

    let mut stderr_text = String::new();
    let mut killed = false;
    loop {
        tokio::select! {
            line = line_rx.recv() => {
                match line {
                    Some((is_stderr, line)) => {
                        if let Some(progress) = parse_progress_line(&line) {
                            on_progress(progress);
                        } else if is_stderr && stderr_text.len() < 8192 {
                            stderr_text.push_str(&line);
                            stderr_text.push('\n');
                        }
                    }
                    None => break,
                }
            }
            _ = cancel.cancelled() => {
                let _ = child.start_kill();
                killed = true;
                break;
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|err| MediaError::backend(err.to_string()))?;
    if killed || cancel.is_cancelled() {
        return Err(MediaError {
            code: 1030,
            name: "ERR_CANCELLED",
            message: "cancelled".to_string(),
        });
    }
    if !status.success() {
        return Err(classify_backend_failure(&stderr_text, status.code()));
    }
    Ok(())
}

/// Size of a produced file, or of a playlist directory, in bytes.
pub fn output_size(path: &std::path::Path) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_file() {
        return Some(meta.len()).filter(|bytes| *bytes > 0);
    }
    let mut total = 0u64;
    for entry in std::fs::read_dir(path).ok()?.flatten() {
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                total += meta.len();
            }
        }
    }
    Some(total).filter(|bytes| *bytes > 0)
}

/// Reject a backend release that is too old to speak to YouTube today.
pub fn check_version(tool: &std::path::Path) -> Result<String, MediaError> {
    let output = std::process::Command::new(tool)
        .arg("--version")
        .output()
        .map_err(|err| MediaError::backend(format!("cannot run yt-dlp: {err}")))?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !version_is_recent(&version) {
        return Err(MediaError {
            code: 1044,
            name: "ERR_MEDIA_BACKEND",
            message: format!(
                "yt-dlp {version} is too old for YouTube (minimum {MIN_YTDLP_VERSION}); install a current release, for example in ~/.local/bin"
            ),
        });
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn old_backends_are_rejected_with_an_actionable_error() {
        assert!(version_is_recent("2026.10.26"));
        assert!(!version_is_recent("2026.03.17"));
        let directory = tempfile::tempdir().unwrap();
        let tool = directory.path().join("yt-dlp");
        std::fs::write(&tool, "#!/bin/sh\necho 2026.03.17\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = check_version(&tool).unwrap_err();
        assert_eq!(error.code, 1044);
        assert!(error.message.contains("too old"));
    }

    #[test]
    fn output_size_reads_files_and_playlist_directories() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("a.m4a");
        std::fs::write(&file, b"0123456789").unwrap();
        assert_eq!(output_size(&file), Some(10));
        let playlist = directory.path().join("list");
        std::fs::create_dir_all(&playlist).unwrap();
        std::fs::write(playlist.join("1.mp4"), b"12345").unwrap();
        assert_eq!(output_size(&playlist), Some(5));
        assert_eq!(output_size(&directory.path().join("missing")), None);
    }

    #[test]
    fn qualities_group_by_height_and_cap_the_ladder() {
        let info: Value = serde_json::from_str(
            r#"{"title":"x","formats":[
                {"format_id":"313","height":2160,"ext":"webm","vcodec":"vp9","acodec":"none","tbr":9000},
                {"format_id":"137","height":1080,"ext":"mp4","vcodec":"avc1","acodec":"none","tbr":4500},
                {"format_id":"22","height":720,"ext":"mp4","vcodec":"avc1","acodec":"mp4a","tbr":1500},
                {"format_id":"18","height":360,"ext":"mp4","vcodec":"avc1","acodec":"mp4a","tbr":600},
                {"format_id":"135","height":480,"ext":"mp4","vcodec":"avc1","acodec":"none","tbr":700},
                {"format_id":"140","ext":"m4a","vcodec":"none","acodec":"mp4a","abr":128}]}"#,
        )
        .unwrap();
        let probe = parse_probe_json(&info, "video").unwrap();
        let tags: Vec<&str> = probe
            .qualities
            .iter()
            .map(|entry| entry.quality.as_str())
            .collect();
        // 480 is a real format but not part of the documented ladder.
        assert_eq!(tags, ["2160", "1080", "720", "360", "audio"]);
        assert_eq!(probe.qualities[0].label, "4K");
        assert_eq!(probe.qualities[1].label, "1080p");
        assert_eq!(probe.qualities[4].label, "audio");
        assert_eq!(probe.qualities[4].format_id, "140");
        assert_eq!(probe.default_quality, "720");
    }

    #[test]
    fn default_quality_falls_back_to_the_closest_height_or_audio() {
        let option = |quality: &str, height: Option<u32>| QualityOption {
            quality: quality.to_string(),
            label: quality.to_string(),
            height,
            format_id: quality.to_string(),
        };
        assert_eq!(default_quality(&[]), "");
        assert_eq!(
            default_quality(&[option("1080", Some(1080)), option("720", Some(720))]),
            "720"
        );
        assert_eq!(
            default_quality(&[option("2160", Some(2160)), option("360", Some(360))]),
            "360"
        );
        assert_eq!(
            default_quality(&[option("1080", Some(1080)), option("audio", None)]),
            "1080"
        );
        assert_eq!(default_quality(&[option("audio", None)]), "audio");
    }

    #[test]
    fn video_selector_prefers_muxed_formats() {
        // Live QA: `b[height<=720]` and `18` no longer exist for every video, so the
        // chain must reach a merged video+audio pair before `best`.
        assert_eq!(
            muxed_selector(Some(720)),
            "b[height<=720]/18/bv*[height<=720]+ba/b[height<=720]/best"
        );
        assert_eq!(muxed_selector(None), "b/18/bv*+ba/best");
        let plan = DownloadPlan {
            kind: MediaTaskKind::YoutubeVideo,
            url: "https://www.youtube.com/watch?v=x".to_string(),
            output: std::path::PathBuf::from("/tmp/x.mp4"),
            format: Some("298".to_string()),
            playlist_limit: 100,
            height: Some(720),
        };
        let args = plan.args();
        let index = args.iter().position(|arg| arg == "-f").expect("-f");
        assert_eq!(args[index + 1], muxed_selector(Some(720)));
        assert!(!args.iter().any(|arg| arg == "298"), "{args:?}");
    }

    #[test]
    fn progress_lines_parse_from_the_template() {
        let line = parse_progress_line("download:4096|8192|512000|6").expect("progress");
        assert_eq!(line.downloaded, 4096);
        assert_eq!(line.total, Some(8192));
        assert_eq!(line.speed, 512_000);
        assert_eq!(line.eta, Some(6));
        assert!(parse_progress_line("[download] 10%").is_none());
    }

    /// yt-dlp 2026.10.26 printed the payload without the literal prefix; the bar never
    /// moved because the parser required `download:` and returned None for every line.
    #[test]
    fn progress_lines_parse_without_the_literal_prefix() {
        let line = parse_progress_line("261120|10202210|36387144.43|0").expect("progress");
        assert_eq!(line.downloaded, 261_120);
        assert_eq!(line.total, Some(10_202_210));
        assert_eq!(line.speed, 36_387_144);
        assert_eq!(line.eta, Some(0));

        // Trailing whitespace, the real log shape, still parses.
        let padded = parse_progress_line("544768|10202210|41943040.0|0\n").expect("progress");
        assert_eq!(padded.downloaded, 544_768);

        // Unrelated numeric output is not a progress line.
        assert!(parse_progress_line("2026").is_none());
        assert!(parse_progress_line("[youtube] Extracting URL").is_none());
    }
}
