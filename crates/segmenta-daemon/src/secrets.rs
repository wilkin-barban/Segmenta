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
pub struct KeyringSecretStore {
    tool: PathBuf,
}

impl KeyringSecretStore {
    /// Use a specific `secret-tool` binary.
    pub fn with_tool(tool: PathBuf) -> KeyringSecretStore {
        KeyringSecretStore { tool }
    }

    /// Detect the binary from `SEGMENTA_SECRET_TOOL` or `PATH`.
    ///
    /// `secret-tool` has no `--version` flag (libsecret 0.21 exits 2 for it), so
    /// availability is probed with a `lookup` that is allowed to answer "not found".
    pub fn detect() -> Option<KeyringSecretStore> {
        let tool = std::env::var_os("SEGMENTA_SECRET_TOOL")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_SECRET_TOOL));
        KeyringSecretStore::probe_tool(&tool).then(|| KeyringSecretStore::with_tool(tool))
    }

    /// Whether `tool` behaves like a usable libsecret CLI.
    pub fn probe_tool(tool: &Path) -> bool {
        let output = Command::new(tool)
            .args(["lookup", "service", "segmenta", "key", PROBE_ALIAS])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match output {
            // 0: a secret exists; 1: no secret — both prove the tool works.
            Ok(output) => matches!(output.status.code(), Some(0) | Some(1)),
            Err(_) => false,
        }
    }
}

/// Binary looked up in `PATH` when the environment does not override it.
const DEFAULT_SECRET_TOOL: &str = "secret-tool";
/// Alias used only to probe the backend; it is never written.
const PROBE_ALIAS: &str = "__segmenta_probe__";

/// Backend used when no Secret Service is reachable.
///
/// It never writes a plaintext fallback: storing a secret fails with
/// [`SecretError::Unavailable`] and the caller must pause or reject the task with
/// `ERR_KEYRING_UNAVAILABLE` (OpenSpec 08).
pub struct UnavailableSecretStore;

impl SecretStore for UnavailableSecretStore {
    fn backend(&self) -> &'static str {
        "unavailable"
    }

    fn store(&self, _alias: &str, _secret: &str) -> Result<(), SecretError> {
        Err(SecretError::Unavailable(
            "no Secret Service: install libsecret-tools and unlock the keyring".to_string(),
        ))
    }

    fn lookup(&self, _alias: &str) -> Result<Option<String>, SecretError> {
        Ok(None)
    }

    fn delete(&self, _alias: &str) -> Result<(), SecretError> {
        Ok(())
    }
}

impl SecretStore for KeyringSecretStore {
    fn backend(&self) -> &'static str {
        "keyring"
    }

    fn store(&self, alias: &str, secret: &str) -> Result<(), SecretError> {
        let mut child = Command::new(&self.tool)
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
        let output = Command::new(&self.tool)
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
        let _ = Command::new(&self.tool)
            .args(["clear", "service", "segmenta", "key", alias])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(())
    }
}

/// Pick the best available backend for this session: keyring or nothing.
///
/// The plaintext file backend is never selected automatically. If the 0.0.1 fallback
/// file exists and the keyring works, its entries are imported and the file removed.
pub fn open_default(data_dir: &Path) -> Result<Box<dyn SecretStore>, SecretError> {
    open_with(data_dir, KeyringSecretStore::detect())
}

/// Same as [`open_default`] with an explicit keyring choice (tests and diagnostics).
pub fn open_with(
    data_dir: &Path,
    keyring: Option<KeyringSecretStore>,
) -> Result<Box<dyn SecretStore>, SecretError> {
    match keyring {
        Some(keyring) => {
            migrate_legacy_file(&keyring, data_dir);
            Ok(Box::new(keyring))
        }
        None => Ok(Box::new(UnavailableSecretStore)),
    }
}

/// Move every alias of the 0.0.1 `secrets.json` into the keyring, then delete the file.
///
/// A partial failure leaves the file untouched: losing a secret silently is worse than
/// asking the user to unlock the keyring and retry.
fn migrate_legacy_file(keyring: &KeyringSecretStore, data_dir: &Path) {
    let path = data_dir.join("secrets.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    let Ok(entries) = serde_json::from_slice::<BTreeMap<String, String>>(&bytes) else {
        eprintln!(
            "segmenta-daemon: {} is not a legacy secret file; leaving it untouched",
            path.display()
        );
        return;
    };
    if entries.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    for (alias, secret) in &entries {
        if let Err(err) = keyring.store(alias, secret) {
            eprintln!(
                "segmenta-daemon: could not import {alias} into the keyring ({err}); keeping {} for a later attempt",
                path.display()
            );
            return;
        }
    }
    match std::fs::remove_file(&path) {
        Ok(()) => eprintln!(
            "segmenta-daemon: imported {} secret(s) from {} into the keyring",
            entries.len(),
            path.display()
        ),
        Err(err) => eprintln!(
            "segmenta-daemon: migrated {} secret(s) but could not remove {}: {err}",
            entries.len(),
            path.display()
        ),
    }
}

fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
}
