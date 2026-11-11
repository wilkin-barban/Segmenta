//! HLS/DASH pipelines for 0.1.0 (spec S9 wiring).
//!
//! Parsing lives in `segmenta_core::stream`; this module fetches the manifests,
//! resolves the selected variant into an ordered segment list, downloads the fragments
//! and merges them with ffmpeg using separated arguments (never a shell).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::json;
use url::Url;

use segmenta_core::stream::{dash, hls, merge, StreamError};

use crate::media::{MediaError, MediaFormat};

/// One resolved stream: init segment (optional) plus ordered media fragments.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamSelection {
    /// `hls` or `dash`.
    pub kind: &'static str,
    /// Selected representation height, when known.
    pub height: Option<u32>,
    /// Initialization segment.
    pub init: Option<Url>,
    /// Media fragments in order.
    pub segments: Vec<Url>,
}

fn map_error(error: StreamError) -> MediaError {
    MediaError {
        code: error.code(),
        name: error.err_name(),
        message: error.to_string(),
    }
}

/// Fetch a manifest as text.
pub async fn fetch_text(client: &reqwest::Client, url: &Url) -> Result<String, MediaError> {
    let response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|err| MediaError::backend(format!("cannot fetch {url}: {err}")))?;
    if !response.status().is_success() {
        return Err(MediaError::backend(format!(
            "{url} answered {}",
            response.status()
        )));
    }
    response
        .text()
        .await
        .map_err(|err| MediaError::backend(format!("cannot read {url}: {err}")))
}

fn is_hls(url: &Url, body: &str) -> bool {
    body.trim_start().starts_with("#EXTM3U") || url.path().ends_with(".m3u8")
}

/// Probe an HLS or DASH manifest and describe its variants/representations.
pub async fn probe(client: &reqwest::Client, url: &Url) -> Result<serde_json::Value, MediaError> {
    let body = fetch_text(client, url).await?;
    let title = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or("stream")
        .to_string();
    if is_hls(url, &body) {
        let master = hls::parse_master(&body, url).map_err(map_error)?;
        let formats: Vec<MediaFormat> = if master.variants.is_empty() {
            vec![MediaFormat {
                format_id: "0".to_string(),
                label: "source".to_string(),
                height: None,
                ext: "mp4".to_string(),
                kind: "video".to_string(),
            }]
        } else {
            master
                .variants
                .iter()
                .enumerate()
                .map(|(index, variant)| MediaFormat {
                    format_id: index.to_string(),
                    label: variant
                        .height()
                        .map(|height| format!("{height}p"))
                        .unwrap_or_else(|| {
                            variant
                                .bandwidth
                                .map(|bandwidth| format!("{} kbps", bandwidth / 1000))
                                .unwrap_or_else(|| format!("variant {index}"))
                        }),
                    height: variant.height(),
                    ext: "mp4".to_string(),
                    kind: "video".to_string(),
                })
                .collect()
        };
        return Ok(json!({
            "kind": "stream",
            "container": "hls",
            "title": title,
            "duration_seconds": null,
            "thumbnail": null,
            "formats": formats,
            "audio": [],
            "playlist": { "count": 0, "entries": [] },
        }));
    }
    if body.contains("<MPD") {
        let manifest = dash::parse_mpd(&body, url).map_err(map_error)?;
        let representations: Vec<&dash::Representation> = manifest
            .periods
            .iter()
            .flat_map(|period| &period.adaptations)
            .filter(|adaptation| adaptation.kind == dash::MediaKind::Video)
            .flat_map(|adaptation| &adaptation.representations)
            .collect();
        let formats: Vec<MediaFormat> = representations
            .iter()
            .enumerate()
            .map(|(_index, representation)| MediaFormat {
                format_id: representation.id.clone(),
                label: representation
                    .height
                    .map(|height| format!("{height}p"))
                    .or_else(|| {
                        representation
                            .bandwidth
                            .map(|bps| format!("{} kbps", bps / 1000))
                    })
                    .unwrap_or_else(|| representation.id.clone()),
                height: representation.height,
                ext: "mp4".to_string(),
                kind: "video".to_string(),
            })
            .collect();
        return Ok(json!({
            "kind": "stream",
            "container": "dash",
            "title": title,
            "duration_seconds": manifest.duration_seconds,
            "thumbnail": null,
            "formats": formats,
            "audio": [],
            "playlist": { "count": 0, "entries": [] },
        }));
    }
    Err(MediaError {
        code: 1043,
        name: "ERR_STREAM_UNSUPPORTED",
        message: "the URL is not a public HLS or DASH manifest".to_string(),
    })
}

/// Resolve the manifest into the fragment list for the requested height.
pub async fn resolve(
    client: &reqwest::Client,
    url: &Url,
    height: Option<u32>,
) -> Result<StreamSelection, MediaError> {
    let body = fetch_text(client, url).await?;
    if is_hls(url, &body) {
        let master = hls::parse_master(&body, url).map_err(map_error)?;
        let media_url = if master.variants.is_empty() {
            url.clone()
        } else {
            hls::select_variant(&master, height)
                .map(|variant| variant.uri.clone())
                .unwrap_or_else(|| url.clone())
        };
        let media_body = fetch_text(client, &media_url).await?;
        let media = hls::parse_media_playlist(&media_body, &media_url).map_err(map_error)?;
        if media.segments.is_empty() {
            return Err(map_error(StreamError::Malformed(
                "the media playlist has no segments".to_string(),
            )));
        }
        return Ok(StreamSelection {
            kind: "hls",
            height: hls::select_variant(&master, height).and_then(|variant| variant.height()),
            init: media.init,
            segments: media
                .segments
                .into_iter()
                .map(|segment| segment.uri)
                .collect(),
        });
    }
    if body.contains("<MPD") {
        let manifest = dash::parse_mpd(&body, url).map_err(map_error)?;
        let representation = dash::select_representation(&manifest, height)
            .ok_or_else(|| map_error(StreamError::Unsupported("no representation".to_string())))?;
        let segments = dash::segments_for(&manifest, &representation.id).map_err(map_error)?;
        return Ok(StreamSelection {
            kind: "dash",
            height: representation.height,
            init: segments.init,
            segments: segments.media,
        });
    }
    Err(MediaError {
        code: 1043,
        name: "ERR_STREAM_UNSUPPORTED",
        message: "the URL is not a public HLS or DASH manifest".to_string(),
    })
}

fn fragment_name(index: usize, url: &Url) -> String {
    let extension = url
        .path()
        .rsplit('.')
        .next()
        .filter(|ext| ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin");
    format!("seg-{index:05}.{extension}")
}

/// Download every fragment, concatenate and remux with ffmpeg (`-c copy`).
pub async fn download(
    client: &reqwest::Client,
    ffmpeg: &Path,
    selection: &StreamSelection,
    output: &Path,
    cancel: segmenta_core::engine::Canceller,
    mut on_progress: impl FnMut(u64),
) -> Result<(), MediaError> {
    let directory = output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".segmenta-stream-{}",
            uuid::Uuid::new_v4().simple()
        ));
    std::fs::create_dir_all(&directory)
        .map_err(|err| MediaError::backend(format!("cannot create scratch: {err}")))?;

    let mut files: Vec<PathBuf> = Vec::new();
    let mut downloaded = 0u64;
    let mut fetch = |target: PathBuf, url: Url| {
        let client = client.clone();
        async move {
            let response = client
                .get(url.clone())
                .send()
                .await
                .map_err(|err| MediaError::backend(format!("cannot fetch {url}: {err}")))?;
            if !response.status().is_success() {
                return Err(MediaError::backend(format!(
                    "{url} answered {}",
                    response.status()
                )));
            }
            let bytes = response
                .bytes()
                .await
                .map_err(|err| MediaError::backend(format!("cannot read {url}: {err}")))?;
            std::fs::write(&target, &bytes).map_err(|err| {
                MediaError::backend(format!("cannot write {}: {err}", target.display()))
            })?;
            Ok::<u64, MediaError>(bytes.len() as u64)
        }
    };

    if let Some(init) = &selection.init {
        let target = directory.join("init.mp4");
        downloaded += fetch(target.clone(), init.clone()).await?;
        files.push(target);
        on_progress(downloaded);
    }
    for (index, segment) in selection.segments.iter().enumerate() {
        if cancel.is_cancelled() {
            let _ = std::fs::remove_dir_all(&directory);
            return Err(MediaError {
                code: 1030,
                name: "ERR_CANCELLED",
                message: "cancelled".to_string(),
            });
        }
        let target = directory.join(fragment_name(index, segment));
        downloaded += fetch(target.clone(), segment.clone()).await?;
        files.push(target);
        on_progress(downloaded);
    }

    let concat = merge::ffconcat(&files);
    let concat_path = directory.join("concat.txt");
    std::fs::write(&concat_path, concat)
        .map_err(|err| MediaError::backend(format!("cannot write concat: {err}")))?;
    let args = merge::ffmpeg_args(&concat_path, output, None);
    let status = tokio::process::Command::new(ffmpeg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|err| MediaError::backend(format!("cannot run ffmpeg: {err}")))?;
    if !status.success() {
        let _ = std::fs::remove_dir_all(&directory);
        return Err(MediaError {
            code: 1044,
            name: "ERR_MEDIA_BACKEND",
            message: format!("ffmpeg could not remux the stream (exit {status})"),
        });
    }
    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}

/// Safe file stem for a manifest URL (no query string, truncated).
pub fn stream_stem(url: &Url) -> String {
    let raw = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or("stream");
    let stem = raw.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(raw);
    crate::media::sanitize_title(stem)
}
