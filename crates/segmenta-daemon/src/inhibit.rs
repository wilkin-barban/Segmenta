//! Sleep inhibition while transfers are running.
//!
//! Contract: T4 of `odd/tasks/segmenta-v0.4.0-automatizacion.md`.
//!
//! # Mechanism
//!
//! The daemon does not link a D-Bus binding: `systemd-inhibit` already takes the lock
//! `org.freedesktop.login1` would hand out and keeps it for as long as the process lives.
//! The daemon spawns `systemd-inhibit --what=sleep --mode=block ... sleep infinity` when
//! the first transfer starts and kills it when the last one ends, which releases the lock.
//!
//! # Degradation
//!
//! Every failure is silent: a missing binary, a system without logind, a session that does
//! not allow the lock. Transfers never depend on the inhibitor, and the reply of a failed
//! attempt is only a `tracing` line.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Why the lock is held, as shown by `systemd-inhibit --list`.
const REASON: &str = "Segmenta is downloading";

/// Holds the sleep inhibitor lock while transfers are running.
pub struct SleepInhibitor {
    child: Option<Child>,
    /// Directories scanned for `systemd-inhibit`.
    search: Vec<PathBuf>,
}

impl SleepInhibitor {
    /// A released inhibitor that looks for the helper on `PATH`.
    pub fn new() -> SleepInhibitor {
        SleepInhibitor::with_search(search_path())
    }

    /// A released inhibitor that looks for the helper in `search`.
    pub fn with_search(search: Vec<PathBuf>) -> SleepInhibitor {
        SleepInhibitor {
            child: None,
            search,
        }
    }

    /// Whether the lock is held right now.
    pub fn is_held(&self) -> bool {
        self.child.is_some()
    }

    /// Take the lock when it is not held, with no effect when it already is.
    ///
    /// Returns whether the lock is held after the call.
    pub fn acquire(&mut self) -> bool {
        if self.child.is_some() {
            return true;
        }
        let Some(binary) = detect(&self.search) else {
            return false;
        };
        let spawned = Command::new(binary)
            .arg("--what=sleep")
            .arg("--mode=block")
            .arg(format!("--why={REASON}"))
            .arg("sleep")
            .arg("infinity")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(child) => {
                self.child = Some(child);
                true
            }
            Err(err) => {
                tracing::debug!(error = %err, "sleep inhibition is unavailable");
                false
            }
        }
    }

    /// Release the lock, with no effect when it is not held.
    pub fn release(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Whether the running helper died on its own, so the next tick retries.
    pub fn dropped(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(_)) => {
                    self.child = None;
                    true
                }
                _ => false,
            },
            None => false,
        }
    }
}

impl Default for SleepInhibitor {
    fn default() -> SleepInhibitor {
        SleepInhibitor::new()
    }
}

impl Drop for SleepInhibitor {
    fn drop(&mut self) {
        self.release();
    }
}

/// Directories scanned for `systemd-inhibit`.
fn search_path() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|raw| std::env::split_paths(&raw).collect())
        .unwrap_or_default()
}

/// First `systemd-inhibit` found in `dirs`.
fn detect(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join("systemd-inhibit"))
        .find(|binary| is_executable(binary))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}
