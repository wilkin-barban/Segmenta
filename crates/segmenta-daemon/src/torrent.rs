//! Torrent download orchestration using `librqbit`.
//!
//! [`probe_torrent`] reads the metainfo and reports what the daemon persists:
//!
//! - Multi-file metainfo (`info.files`) reports `total_bytes` as the sum of the file
//!   lengths (never `info.length`, which BEP 3 forbids alongside `files`), `name` from
//!   `info.name`, and one `files` entry per `info.files` entry with `index` contiguous
//!   from 0, the sanitized relative `path`, its `length` and `selected`. Nothing tracks
//!   a per-file selection yet, so `selected` is always `true`.
//! - Single-file metainfo (`info.length`, no `files`) keeps its 0.1.x shape:
//!   `total_bytes` is `info.length` and `files` carries exactly one entry (`index` 0,
//!   `path` from `info.name`, its `length`, `selected: true`). The array is empty only
//!   when there is no file to report (magnets, and a degenerate `info.files: []`), so a
//!   decoded single-file `.torrent` always reports exactly one entry. That is the single
//!   decision for the whole module.
//! - Every path component goes through [`segmenta_core::filename::sanitize_filename`],
//!   which drops directory prefixes (so `../escape.bin`, `/etc/passwd` and
//!   `dir\..\evil.dll` collapse to plain file names) and removes control characters.
//!   A component with nothing usable left (`..`, `.`, empty) rejects the metainfo with
//!   the same invalid-params error used for undecodable bencode instead of being
//!   repaired into an escaping path. `name` is still reported raw (lossy UTF-8) with
//!   its original meaning: [`sanitize_torrent_path`] is what guards the filesystem
//!   write.
//! - The declared `pieces` budget has to cover the summed length exactly (one 20-byte
//!   hash per `piece length` slice, the last one possibly partial). A metainfo whose
//!   hashes disagree with the lengths is rejected with that same invalid-params error
//!   rather than silently reporting a truncated total.

use crate::rpc::RpcError;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn sanitize_torrent_path(base_dir: &Path, raw_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(raw_path);
    if path.is_absolute() {
        return Err(format!("absolute path rejected: {raw_path}"));
    }
    let mut resolved = base_dir.to_path_buf();
    for component in path.components() {
        match component {
            std::path::Component::Normal(s) => {
                let s_str = s.to_string_lossy();
                if s_str == ".." || s_str == "." {
                    return Err(format!("directory traversal rejected: {raw_path}"));
                }
                if let Some(safe_comp) = segmenta_core::filename::sanitize_filename(&s_str) {
                    resolved.push(safe_comp);
                } else {
                    return Err(format!("invalid path component: {s_str}"));
                }
            }
            std::path::Component::ParentDir | std::path::Component::CurDir => {
                return Err(format!("directory traversal rejected: {raw_path}"));
            }
            _ => {
                return Err(format!("invalid component in path: {raw_path}"));
            }
        }
    }
    if !resolved.starts_with(base_dir) {
        return Err(format!("path escapes base directory: {raw_path}"));
    }
    Ok(resolved)
}

/// One file reported by the probe.
struct ProbeFile {
    index: usize,
    path: String,
    length: u64,
    selected: bool,
}

impl ProbeFile {
    fn to_json(&self) -> Value {
        json!({
            "index": self.index,
            "path": self.path,
            "length": self.length,
            "selected": self.selected,
        })
    }
}

/// Sanitize one `info.files[].path` list into a relative path.
///
/// Components keep their order and are joined with `/`. A component that sanitizes to
/// nothing is hostile input (a bare `..`, `.` or an empty string) and rejects the
/// metainfo: there is no name to invent that would stay inside the download directory.
fn sanitize_path_components(components: &[&[u8]]) -> Result<String, RpcError> {
    if components.is_empty() {
        return Err(RpcError::invalid_params(
            "invalid .torrent: a file entry has no path components",
        ));
    }
    let mut parts = Vec::with_capacity(components.len());
    for component in components {
        let raw = String::from_utf8_lossy(component);
        let Some(safe) = segmenta_core::filename::sanitize_filename(&raw) else {
            return Err(RpcError::invalid_params(format!(
                "invalid .torrent: unusable path component `{raw}`"
            )));
        };
        parts.push(safe);
    }
    Ok(parts.join("/"))
}

/// Check that `info.pieces` declares one 20-byte hash per `piece_length` slice of
/// `total`, the last piece possibly partial.
fn validate_piece_budget(total: u64, piece_length: u32, pieces_len: usize) -> Result<(), RpcError> {
    if piece_length == 0 {
        return Err(RpcError::invalid_params(
            "invalid .torrent: info.piece length is zero",
        ));
    }
    if pieces_len % 20 != 0 {
        return Err(RpcError::invalid_params(format!(
            "invalid .torrent: info.pieces is {pieces_len} bytes, not a whole number of 20-byte piece hashes"
        )));
    }
    let expected = total.div_ceil(u64::from(piece_length));
    let declared = (pieces_len / 20) as u64;
    if declared != expected {
        return Err(RpcError::invalid_params(format!(
            "invalid .torrent: {declared} piece hashes declared, {expected} needed for {total} bytes"
        )));
    }
    Ok(())
}

/// Turn a decoded `info` dict into the reported total and file list.
fn probe_metainfo(
    info: &librqbit::TorrentMetaV1Info<&[u8]>,
    name: &str,
) -> Result<(u64, Vec<ProbeFile>), RpcError> {
    if let Some(files) = info.files.as_ref() {
        let mut total = 0u64;
        let mut entries = Vec::with_capacity(files.len());
        for (index, file) in files.iter().enumerate() {
            total = total.checked_add(file.length).ok_or_else(|| {
                RpcError::invalid_params("invalid .torrent: file lengths overflow u64")
            })?;
            entries.push(ProbeFile {
                index,
                path: sanitize_path_components(&file.path)?,
                length: file.length,
                selected: true,
            });
        }
        validate_piece_budget(total, info.piece_length, info.pieces.len())?;
        return Ok((total, entries));
    }

    let length = info.length.unwrap_or(0);
    if info.length.is_some() {
        validate_piece_budget(length, info.piece_length, info.pieces.len())?;
    }
    let entries = vec![ProbeFile {
        index: 0,
        path: sanitize_path_components(&[name.as_bytes()])?,
        length,
        selected: true,
    }];
    Ok((length, entries))
}

pub async fn probe_torrent(
    client: &reqwest::Client,
    url_or_magnet: &str,
) -> Result<Value, RpcError> {
    if url_or_magnet.starts_with("magnet:") {
        let magnet = librqbit::Magnet::parse(url_or_magnet)
            .map_err(|err| RpcError::invalid_params(format!("invalid magnet URL: {err}")))?;
        let name = magnet
            .name
            .clone()
            .unwrap_or_else(|| "magnet_download".to_string());
        Ok(json!({
            "name": name,
            "info_hash": "",
            "total_bytes": null,
            "files": [],
            "source_type": "magnet",
        }))
    } else {
        let parsed_url = url::Url::parse(url_or_magnet)
            .map_err(|err| RpcError::invalid_params(format!("invalid torrent URL: {err}")))?;
        let response = client
            .get(parsed_url.as_str())
            .send()
            .await
            .map_err(|err| RpcError::new(1004, "ERR_NETWORK", err.to_string()))?;
        let bytes = response
            .bytes()
            .await
            .map_err(|err| RpcError::new(1004, "ERR_NETWORK", err.to_string()))?;
        let meta = librqbit_bencode::from_bytes::<librqbit::TorrentMetaV1<&[u8]>>(&bytes)
            .map_err(|err| RpcError::invalid_params(format!("invalid .torrent bencode: {err}")))?;
        let name = String::from_utf8_lossy(meta.info.data.name.unwrap_or(b"torrent")).to_string();
        let (total_bytes, files) = probe_metainfo(&meta.info.data, &name)?;
        let files: Vec<Value> = files.iter().map(ProbeFile::to_json).collect();

        Ok(json!({
            "name": name,
            "info_hash": "",
            "total_bytes": total_bytes,
            "files": files,
            "source_type": "torrent",
        }))
    }
}

/// Progress of a running torrent, as reported by `handle.stats()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TorrentProgress {
    /// Bytes already written.
    pub downloaded: u64,
    /// Declared total size.
    pub total: u64,
    /// The torrent finished downloading.
    pub finished: bool,
}

/// One process-wide session: `librqbit` must not be recreated per task or chunk.
static SESSION: tokio::sync::OnceCell<std::sync::Arc<librqbit::Session>> =
    tokio::sync::OnceCell::const_new();

/// Returns the shared session, creating it on first use.
pub async fn shared_session(
    output_dir: &Path,
) -> Result<std::sync::Arc<librqbit::Session>, String> {
    SESSION
        .get_or_try_init(|| async {
            librqbit::Session::new(output_dir.to_path_buf())
                .await
                .map_err(|err| err.to_string())
        })
        .await
        .cloned()
}

/// Adds a magnet or `.torrent` URL and drives it to completion.
///
/// Seeding stays off: the task stops as soon as the payload is complete, and a
/// cancellation deletes the torrent from the session without touching the data.
pub async fn run_torrent(
    session: std::sync::Arc<librqbit::Session>,
    source: String,
    cancel: segmenta_core::engine::Canceller,
    mut on_progress: impl FnMut(TorrentProgress),
) -> Result<(), String> {
    let response = session
        .add_torrent(
            librqbit::AddTorrent::from_url(source),
            Some(librqbit::AddTorrentOptions {
                overwrite: true,
                paused: false,
                ..Default::default()
            }),
        )
        .await
        .map_err(|err| err.to_string())?;
    let (id, handle) = match response {
        librqbit::AddTorrentResponse::Added(id, handle)
        | librqbit::AddTorrentResponse::AlreadyManaged(id, handle) => (id, handle),
        librqbit::AddTorrentResponse::ListOnly(_) => {
            return Err("the torrent was opened in list-only mode".to_string())
        }
    };
    loop {
        if cancel.is_cancelled() {
            let _ = session
                .delete(librqbit::api::TorrentIdOrHash::Id(id), false)
                .await;
            return Err("cancelled".to_string());
        }
        let stats = handle.stats();
        on_progress(TorrentProgress {
            downloaded: stats.progress_bytes,
            total: stats.total_bytes,
            finished: stats.finished,
        });
        if stats.finished {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}
