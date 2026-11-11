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
    /// Video formats, best first.
    pub formats: Vec<MediaFormat>,
    /// Audio formats, best first.
    pub audio: Vec<MediaFormat>,
    /// Playlist summary.
    pub playlist: PlaylistSummary,
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
pub fn validate_media_url(raw: &str) -> Result<url::Url, MediaError> {
    let parsed = url::Url::parse(raw).map_err(|err| MediaError::invalid(err.to_string()))?;
    if parsed.scheme() != "https" {
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

/// Whether the host is a YouTube host.
pub fn is_youtube(host: &str) -> bool {
    host == "www.youtube.com" || host == "youtube.com" || host == "m.youtube.com"
}

fn classify_formats(info: &Value) -> (Vec<MediaFormat>, Vec<MediaFormat>) {
    let mut video = Vec::new();
    let mut audio = Vec::new();
    let Some(entries) = info.get("formats").and_then(Value::as_array) else {
        return (video, audio);
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
        if has_video {
            video.push(item);
        } else if has_audio {
            audio.push(item);
        }
    }
    video.sort_by_key(|format| std::cmp::Reverse(format.height));
    audio.sort_by(|left, right| right.label.cmp(&left.label));
    (video, audio)
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

    if formats.is_empty() && audio.is_empty() && playlist.count == 0 {
        return Err(MediaError::not_public(
            "the media has no downloadable public formats",
        ));
    }

    Ok(ProbeOutcome {
        kind: kind.to_string(),
        title,
        duration_seconds,
        thumbnail,
        formats,
        audio,
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
pub async fn probe_with(tool: &Path, url: &str, kind: &str) -> Result<ProbeOutcome, MediaError> {
    let output = Command::new(tool)
        .arg("--no-config")
        .arg("--no-warnings")
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
}

impl DownloadPlan {
    /// Build the argument vector, never a shell string.
    pub fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "--no-config".into(),
            "--newline".into(),
            "--no-part".into(),
            "--continue".into(),
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
                args.push(self.format.clone().unwrap_or_else(|| "best".to_string()));
                args.push("--merge-output-format".into());
                args.push("mp4".into());
                args.push("--no-playlist".into());
            }
            MediaTaskKind::YoutubePlaylist => {
                args.push("-f".into());
                args.push(self.format.clone().unwrap_or_else(|| "best".to_string()));
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
    let payload = line.strip_prefix("download:")?;
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
    let mut reader = BufReader::new(stdout).lines();
    let mut err_task = None;
    if let Some(stderr) = stderr {
        err_task = Some(tokio::spawn(async move {
            let mut buffer = String::new();
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                buffer.push_str(&line);
                buffer.push('\n');
                if buffer.len() > 8192 {
                    break;
                }
            }
            buffer
        }));
    }

    let mut killed = false;
    loop {
        tokio::select! {
            line = reader.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if let Some(progress) = parse_progress_line(&line) {
                            on_progress(progress);
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
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
    let stderr_text = match err_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
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
