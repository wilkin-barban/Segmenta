//! Optional extraction of a finished archive.
//!
//! Contract: T3 of `odd/tasks/segmenta-v0.4.0-automatizacion.md`.
//!
//! # Why an external tool
//!
//! The daemon does not link an archive library: `7z` already handles zip, 7z, rar and
//! multipart volumes, and `unzip` is the fallback for zip on machines without `7z`. Both
//! are executed through [`crate::postprocess::run_process`]: argv only, no shell, a
//! deadline and bounded capture. When neither binary exists the phase degrades silently:
//! the download keeps its successful state and only a message is recorded.
//!
//! # Containment
//!
//! Extraction is not trusted to the tool. The entries are listed first and the whole
//! archive is rejected when any name is absolute or carries a `..` component. The tool then
//! writes into a staging directory inside the destination; the staging tree is walked and
//! rejected if it holds a symbolic link, and only afterwards are its top-level entries
//! moved into place, refusing to overwrite anything that already exists.
//!
//! # The original
//!
//! The archive is removed only when the caller asked for it **and** the extraction
//! succeeded. Every other path leaves the bytes in place.

use crate::postprocess::{run_process, DEFAULT_CAPTURE_LIMIT};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Deadline of one extraction, in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Extensions treated as archives. Multipart volumes keep one of these suffixes.
const ARCHIVE_EXTENSIONS: &[&str] = &["zip", "7z", "rar", "001"];

/// Counter that keeps two concurrent extractions from sharing a staging directory.
static STAGING: AtomicU64 = AtomicU64::new(0);

/// What happened to one archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extraction {
    /// The file is not an archive: the phase does not apply.
    Skipped,
    /// No `7z` or `unzip` is available; the download keeps its state.
    Unavailable {
        /// Bounded, human-readable record.
        message: String,
    },
    /// The archive was refused before its content reached the destination.
    Rejected {
        /// Bounded, human-readable record.
        message: String,
    },
    /// The archive was extracted into its own directory.
    Succeeded {
        /// Bounded, human-readable record.
        message: String,
    },
    /// The tool ran and failed.
    Failed {
        /// Bounded, human-readable record.
        message: String,
    },
}

impl Extraction {
    /// The recorded message, when the phase left one.
    pub fn message(&self) -> Option<&str> {
        match self {
            Extraction::Skipped => None,
            Extraction::Unavailable { message }
            | Extraction::Rejected { message }
            | Extraction::Succeeded { message }
            | Extraction::Failed { message } => Some(message),
        }
    }

    /// Whether the archive may be deleted: only after a real success.
    pub fn source_removable(&self) -> bool {
        matches!(self, Extraction::Succeeded { .. })
    }
}

/// Whether `path` names an archive this phase can handle.
pub fn is_archive(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .map(|extension| {
            let extension = extension.to_ascii_lowercase();
            ARCHIVE_EXTENSIONS.contains(&extension.as_str())
        })
        .unwrap_or(false)
}

/// The external tool that will be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    SevenZip,
    Unzip,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::SevenZip => "7z",
            Tool::Unzip => "unzip",
        }
    }

    /// Arguments that make the tool list the entry names, one per line.
    fn list_args(self, binary: &Path, archive: &Path) -> Vec<String> {
        let binary = binary.to_string_lossy().into_owned();
        match self {
            Tool::SevenZip => vec![
                binary,
                "l".to_string(),
                "-slt".to_string(),
                archive.to_string_lossy().into_owned(),
            ],
            Tool::Unzip => vec![
                binary,
                "-Z1".to_string(),
                archive.to_string_lossy().into_owned(),
            ],
        }
    }

    /// Arguments that extract the archive into `staging`.
    fn extract_args(
        self,
        binary: &Path,
        archive: &Path,
        staging: &Path,
        password: Option<&str>,
    ) -> Vec<String> {
        let binary = binary.to_string_lossy().into_owned();
        let mut arguments = match self {
            Tool::SevenZip => vec![
                binary,
                "x".to_string(),
                "-y".to_string(),
                format!("-o{}", staging.to_string_lossy()),
                archive.to_string_lossy().into_owned(),
            ],
            Tool::Unzip => vec![
                binary,
                "-o".to_string(),
                "-q".to_string(),
                archive.to_string_lossy().into_owned(),
                "-d".to_string(),
                staging.to_string_lossy().into_owned(),
            ],
        };
        if let Some(password) = password {
            arguments.push(match self {
                Tool::SevenZip => format!("-p{password}"),
                Tool::Unzip => format!("-P{password}"),
            });
        }
        arguments
    }

    /// Whether one line of the listing names an entry, and its name.
    fn entry(self, line: &str) -> Option<&str> {
        match self {
            Tool::SevenZip => line.trim().strip_prefix("Path = "),
            Tool::Unzip => {
                let line = line.trim_end();
                (!line.is_empty()).then_some(line)
            }
        }
    }
}

/// Extract `archive` next to itself, honouring `keep_source`.
///
/// `search` is the list of directories scanned for the tool; the daemon passes its own
/// `engine_search_path`, tests pass a temporary directory holding a stub.
pub fn extract(
    archive: &Path,
    keep_source: bool,
    password: Option<&str>,
    search: &[PathBuf],
) -> Extraction {
    if !is_archive(archive) {
        return Extraction::Skipped;
    }
    let Some((tool, binary)) = detect(search) else {
        return Extraction::Unavailable {
            message: "no archive tool found: install 7z or unzip".to_string(),
        };
    };
    let Some(destination) = archive.parent() else {
        return Extraction::Rejected {
            message: "archive has no parent directory".to_string(),
        };
    };

    let listing = run_process(
        &tool.list_args(&binary, archive),
        Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        DEFAULT_CAPTURE_LIMIT,
        "extract",
    );
    if !listing.exit.succeeded() {
        return Extraction::Failed {
            message: format!("cannot list `{}`: {}", name_of(archive), listing.message),
        };
    }
    if let Some(entry) = unsafe_entry(tool, &listing.stdout) {
        return Extraction::Rejected {
            message: format!("archive entry `{entry}` escapes the destination"),
        };
    }

    let staging = destination.join(format!(
        ".segmenta-extract-{}-{}",
        std::process::id(),
        STAGING.fetch_add(1, Ordering::Relaxed)
    ));
    if std::fs::create_dir(&staging).is_err() {
        return Extraction::Failed {
            message: "cannot create the staging directory".to_string(),
        };
    }

    let extracted = run_process(
        &tool.extract_args(&binary, archive, &staging, password),
        Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        DEFAULT_CAPTURE_LIMIT,
        "extract",
    );
    if !extracted.exit.succeeded() {
        let _ = std::fs::remove_dir_all(&staging);
        return Extraction::Failed {
            message: format!(
                "cannot extract `{}`: {}",
                name_of(archive),
                extracted.message
            ),
        };
    }
    if has_symlink(&staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Extraction::Rejected {
            message: "archive contains a symbolic link".to_string(),
        };
    }

    let moved = publish(&staging, destination);
    let _ = std::fs::remove_dir_all(&staging);
    let published = match moved {
        Ok(count) => count,
        Err(message) => return Extraction::Rejected { message },
    };
    if !keep_source {
        let _ = std::fs::remove_file(archive);
    }
    Extraction::Succeeded {
        message: format!(
            "extracted {published} entr{} from `{}`",
            if published == 1 { "y" } else { "ies" },
            name_of(archive)
        ),
    }
}

/// First tool found in `search`, in the documented preference order, with its binary.
fn detect(search: &[PathBuf]) -> Option<(Tool, PathBuf)> {
    [Tool::SevenZip, Tool::Unzip].into_iter().find_map(|tool| {
        search
            .iter()
            .map(|dir| dir.join(tool.name()))
            .find(|binary| binary.is_file())
            .map(|binary| (tool, binary))
    })
}

/// Entry names that would escape the destination, if any.
fn unsafe_entry(tool: Tool, listing: &str) -> Option<String> {
    entries(tool, listing)
        .into_iter()
        .find(|entry| is_unsafe(entry))
        .map(|entry| entry.to_string())
}

/// The entry names of a listing, without the archive's own metadata.
///
/// `7z l -slt` prints the archive itself as a `Path = ...` record before the `----------`
/// separator, so only the records after it are members. Treating the header as a member
/// would reject every archive opened by its absolute path.
fn entries(tool: Tool, listing: &str) -> Vec<&str> {
    let lines: Vec<&str> = listing.lines().collect();
    let start = match tool {
        Tool::SevenZip => lines
            .iter()
            .position(|line| line.trim() == "----------")
            .map(|index| index + 1)
            .unwrap_or(lines.len()),
        Tool::Unzip => 0,
    };
    lines[start..]
        .iter()
        .filter_map(|line| tool.entry(line))
        .collect()
}

/// An entry escapes when it is absolute or walks upwards.
fn is_unsafe(entry: &str) -> bool {
    if entry.is_empty() || entry.contains('\0') {
        return true;
    }
    let path = Path::new(entry);
    path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
}

/// Whether the staged tree holds a link the destination would follow.
fn has_symlink(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return true;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            return true;
        };
        if metadata.file_type().is_symlink() {
            return true;
        }
        if metadata.is_dir() && has_symlink(&path) {
            return true;
        }
    }
    false
}

/// Move every top-level entry of `staging` into `destination`, never overwriting.
fn publish(staging: &Path, destination: &Path) -> Result<usize, String> {
    let entries = std::fs::read_dir(staging)
        .map_err(|err| format!("cannot read the staging directory: {err}"))?;
    let mut moved = 0;
    for entry in entries.flatten() {
        let target = destination.join(entry.file_name());
        if target.exists() {
            return Err(format!(
                "destination already holds `{}`",
                entry.file_name().to_string_lossy()
            ));
        }
        let path = entry.path();
        if std::fs::rename(&path, &target).is_err() {
            return Err(format!(
                "cannot move `{}` into the destination",
                entry.file_name().to_string_lossy()
            ));
        }
        moved += 1;
    }
    Ok(moved)
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}
