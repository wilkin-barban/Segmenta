//! Segmenta daemon library: persistence, IPC and task orchestration.
//!
//! The binary in `main.rs` is a thin shell around [`run`]; everything else lives here so
//! the CLI, the GUI and the integration tests share the same contract types.

pub mod events;
pub mod ipc;
pub mod logging;
pub mod manager;
pub mod rpc;
pub mod secrets;
pub mod store;

/// Version reported by `system.hello` and `system.status`.
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Directory that holds the database and the fallback secret file.
pub fn data_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        if !dir.is_empty() {
            return std::path::PathBuf::from(dir).join("segmenta");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(home).join(".local/share/segmenta")
}

/// Runtime directory of the socket, following OpenSpec 07 (`%t/segmenta/daemon.sock`).
pub fn runtime_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return std::path::PathBuf::from(dir).join("segmenta");
        }
    }
    // systemd always sets XDG_RUNTIME_DIR; the fallback keeps standalone runs working.
    std::path::PathBuf::from(format!("/tmp/segmenta-{}", unsafe { libc::getuid() }))
}

/// Socket path for the current session.
pub fn socket_path() -> std::path::PathBuf {
    runtime_dir().join("daemon.sock")
}
