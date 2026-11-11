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
