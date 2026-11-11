//! Secret storage.
//!
//! SQLite only ever stores an alias (`download_secrets.keyring_secret_id`). The secret
//! itself lives in the Secret Service when `secret-tool` is available, and otherwise in
//! a `0600` file next to the database — never in the MIME-decoded database, never in a
//! log, never in an event payload.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// Errors of the secret layer.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// The backend could not be used.
    #[error("secret backend unavailable: {0}")]
    Unavailable(String),
    /// Filesystem failure.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Where secrets live.
pub trait SecretStore: Send + Sync {
    /// Human name of the backend, shown by `system.status`.
    fn backend(&self) -> &'static str;
    /// Store or replace the secret behind `alias`.
    fn store(&self, alias: &str, secret: &str) -> Result<(), SecretError>;
    /// Read it back.
    fn lookup(&self, alias: &str) -> Result<Option<String>, SecretError>;
    /// Delete it; missing aliases are not an error.
    fn delete(&self, alias: &str) -> Result<(), SecretError>;
}

/// Secrets in a `0600` JSON file, used when no Secret Service is reachable.
pub struct FileSecretStore {
    path: PathBuf,
    entries: Mutex<BTreeMap<String, String>>,
}

impl FileSecretStore {
    /// Open (creating if needed) the fallback file.
    pub fn open(path: &Path) -> Result<FileSecretStore, SecretError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            set_mode(parent, 0o700)?;
        }
        let entries = match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(err) => return Err(SecretError::Io(err)),
        };
        let store = FileSecretStore {
            path: path.to_path_buf(),
            entries: Mutex::new(entries),
        };
        if !store.path.exists() {
            store.flush()?;
        }
        set_mode(&store.path, 0o600)?;
        Ok(store)
    }

    fn flush(&self) -> Result<(), SecretError> {
        let entries = self.entries.lock().unwrap_or_else(|err| err.into_inner());
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)?;
        file.write_all(serde_json::to_vec(&*entries).unwrap_or_default().as_slice())?;
        file.sync_all()?;
        Ok(())
    }
}

impl SecretStore for FileSecretStore {
    fn backend(&self) -> &'static str {
        "file"
    }

    fn store(&self, alias: &str, secret: &str) -> Result<(), SecretError> {
        self.entries
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(alias.to_string(), secret.to_string());
        self.flush()
    }

    fn lookup(&self, alias: &str) -> Result<Option<String>, SecretError> {
        Ok(self
            .entries
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(alias)
            .cloned())
    }

    fn delete(&self, alias: &str) -> Result<(), SecretError> {
        self.entries
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(alias);
        self.flush()
    }
}

/// Secrets in the Secret Service through `secret-tool` (libsecret CLI).
pub struct KeyringSecretStore;

impl KeyringSecretStore {
    /// Whether `secret-tool` exists in `PATH`.
    pub fn available() -> bool {
        Command::new("secret-tool")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

impl SecretStore for KeyringSecretStore {
    fn backend(&self) -> &'static str {
        "keyring"
    }

    fn store(&self, alias: &str, secret: &str) -> Result<(), SecretError> {
        let mut child = Command::new("secret-tool")
            .args([
                "store",
                "--label",
                "Segmenta download secret",
                "service",
                "segmenta",
                "key",
                alias,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| SecretError::Unavailable(err.to_string()))?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(secret.as_bytes())
                .map_err(|err| SecretError::Unavailable(err.to_string()))?;
        }
        let status = child
            .wait()
            .map_err(|err| SecretError::Unavailable(err.to_string()))?;
        if !status.success() {
            return Err(SecretError::Unavailable(
                "secret-tool store failed".to_string(),
            ));
        }
        Ok(())
    }

    fn lookup(&self, alias: &str) -> Result<Option<String>, SecretError> {
        let output = Command::new("secret-tool")
            .args(["lookup", "service", "segmenta", "key", alias])
            .output()
            .map_err(|err| SecretError::Unavailable(err.to_string()))?;
        if !output.status.success() {
            return Ok(None);
        }
        let secret = String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string();
        Ok(if secret.is_empty() {
            None
        } else {
            Some(secret)
        })
    }

    fn delete(&self, alias: &str) -> Result<(), SecretError> {
        let _ = Command::new("secret-tool")
            .args(["clear", "service", "segmenta", "key", alias])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(())
    }
}

/// Pick the best available backend for this session.
pub fn open_default(data_dir: &Path) -> Result<Box<dyn SecretStore>, SecretError> {
    if KeyringSecretStore::available() {
        return Ok(Box::new(KeyringSecretStore));
    }
    Ok(Box::new(FileSecretStore::open(
        &data_dir.join("secrets.json"),
    )?))
}

fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
}
