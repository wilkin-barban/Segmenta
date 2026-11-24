//! Gallery import through `gallery-dl`.
//!
//! Contract: T6 of `odd/tasks/segmenta-v0.5.0-ecosistema.md`.
//!
//! # Execution
//!
//! The binary is discovered on [`GALLERY_PATH_ENV`] when it is set and non-empty, and on
//! `PATH` otherwise — the same shape the antivirus scan uses. It runs through
//! [`crate::postprocess::run_process`]: argv only, no shell, a deadline and a bounded
//! capture, so a stuck or noisy extractor cannot take the daemon down.
//!
//! # Results
//!
//! `gallery-dl` writes the files itself; the module walks the destination afterwards and
//! returns what appeared, bounded by [`MAX_FILES`]. The caller turns those files into
//! completed tasks under one package, which is exactly the shape the GUI and the popup
//! already know.
//!
//! # Degradation
//!
//! A missing binary is [`GalleryRun::Unavailable`], not an error: the caller reports it and
//! the rest of the daemon keeps working. A failed run keeps its exit code, and a run that
//! produced nothing is [`GalleryRun::Empty`], so an empty gallery is never reported as a
//! success.

use crate::postprocess::{run_process, DEFAULT_CAPTURE_LIMIT};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Environment override for the search directory, used by the tests.
pub const GALLERY_PATH_ENV: &str = "SEGMENTA_GALLERY_PATH";

/// Binary name of the extractor.
pub const BINARY_NAME: &str = "gallery-dl";

/// Deadline of one gallery run, in seconds.
/// Deadline of one gallery run, in seconds.
///
/// Kept below the 120 s read timeout the CLI and the GUI use for an IPC call, so a slow
/// gallery fails with a clear code instead of leaving the client waiting.
pub const DEFAULT_TIMEOUT_SECS: u64 = 110;

/// Most files one import registers.
pub const MAX_FILES: usize = 2000;

/// What one gallery import produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GalleryRun {
    /// The extractor is not installed; the import does nothing and says so.
    Unavailable,
    /// The extractor ran and wrote these files, in the order they were found.
    Extracted(Vec<PathBuf>),
    /// The extractor ran and wrote nothing.
    Empty,
    /// The extractor failed; the message is bounded and never holds a token.
    Failed {
        /// Exit code, when the process produced one.
        exit: Option<i32>,
        /// Bounded record of the reason.
        message: String,
    },
}

impl GalleryRun {
    /// Stable code the caller maps to an error for the user.
    pub fn code(&self) -> Option<&'static str> {
        match self {
            GalleryRun::Unavailable => Some("ERR_GALLERY_UNAVAILABLE"),
            GalleryRun::Empty => Some("ERR_GALLERY_EMPTY"),
            GalleryRun::Failed { .. } => Some("ERR_GALLERY_FAILED"),
            GalleryRun::Extracted(_) => None,
        }
    }
}

/// Directories scanned for the binary.
pub fn search_path() -> Vec<PathBuf> {
    let raw = match std::env::var_os(GALLERY_PATH_ENV) {
        Some(value) if !value.is_empty() => value,
        _ => std::env::var_os("PATH").unwrap_or_default(),
    };
    std::env::split_paths(&raw).collect()
}

/// First `gallery-dl` found in `dirs`.
pub fn detect(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(BINARY_NAME))
        .find(|binary| is_executable(binary))
}

/// Run the extractor for `url`, writing into `destination`.
///
/// `search` is the list of directories scanned for the binary, so a test passes a
/// temporary directory holding a stub.
pub fn run(url: &str, destination: &Path, search: &[PathBuf], timeout: Duration) -> GalleryRun {
    if !is_http_url(url) {
        return GalleryRun::Failed {
            exit: None,
            message: "the gallery url must be http or https".to_string(),
        };
    }
    let Some(binary) = detect(search) else {
        return GalleryRun::Unavailable;
    };
    let before: std::collections::HashSet<PathBuf> = collect(destination).into_iter().collect();
    let argv = vec![
        binary.to_string_lossy().into_owned(),
        "--dest".to_string(),
        destination.to_string_lossy().into_owned(),
        url.to_string(),
    ];
    let outcome = run_process(&argv, timeout, DEFAULT_CAPTURE_LIMIT, "gallery-dl");
    if !outcome.exit.succeeded() {
        return GalleryRun::Failed {
            exit: outcome.exit.code(),
            message: outcome.message,
        };
    }
    let files: Vec<PathBuf> = collect(destination)
        .into_iter()
        .filter(|path| !before.contains(path))
        .collect();
    if files.is_empty() {
        return GalleryRun::Empty;
    }
    GalleryRun::Extracted(files)
}

/// Regular files under `root`, bounded by [`MAX_FILES`].
fn collect(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                files.push(path);
                if files.len() == MAX_FILES {
                    return files;
                }
            }
        }
    }
    files.sort();
    files
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn is_http_url(url: &str) -> bool {
    let url = url.trim();
    url.len() <= 4096 && (url.starts_with("http://") || url.starts_with("https://"))
}
