//! Torrent download orchestration using `librqbit`.

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
        let length = meta.info.data.length.unwrap_or(0);

        Ok(json!({
            "name": name,
            "info_hash": "",
            "total_bytes": length,
            "files": [{
                "index": 0,
                "path": name.clone(),
                "length": length,
                "selected": true,
            }],
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
