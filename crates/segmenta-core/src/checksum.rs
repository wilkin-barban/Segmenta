//! Optional integrity verification with SHA-256 (OpenSpec 06 §5.2).

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{CoreError, Result};

const BLOCK: usize = 1024 * 1024;

/// Hex SHA-256 of an in-memory buffer.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Hex SHA-256 of a file, read in 1 MiB blocks off the async runtime.
pub async fn sha256_file(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    let digest = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; BLOCK];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hex::encode(hasher.finalize()))
    })
    .await
    .map_err(|err| CoreError::Io { kind: std::io::ErrorKind::Other, message: err.to_string() })??;
    Ok(digest)
}
