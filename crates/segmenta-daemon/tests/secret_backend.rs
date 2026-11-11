//! Secret backend selection and migration. Specs S1-S3.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use segmenta_daemon::secrets::{
    open_with, FileSecretStore, KeyringSecretStore, SecretError, SecretStore,
};

/// Write a fake `secret-tool` that mimics libsecret: `--version` fails with exit 2,
/// `lookup` exits 1 when there is no match, `store`/`clear` keep state in `state/`.
fn fake_secret_tool(dir: &Path, state: &Path) -> std::path::PathBuf {
    let script = dir.join("secret-tool");
    let body = format!(
        r#"#!/bin/sh
state="{state}"
cmd="$1"; shift || true
case "$cmd" in
  --version) exit 2 ;;
  lookup)
    key=""
    while [ $# -gt 0 ]; do
      case "$1" in key) key=$(printf %s "$2" | tr "/" "_"); shift 2 ;; *) shift ;; esac
    done
    if [ -f "$state/$key" ]; then cat "$state/$key"; exit 0; fi
    exit 1 ;;
  store)
    key=""
    while [ $# -gt 0 ]; do
      case "$1" in key) key=$(printf %s "$2" | tr "/" "_"); shift 2 ;; *) shift ;; esac
    done
    mkdir -p "$state"
    cat > "$state/$key"
    exit 0 ;;
  clear)
    key=""
    while [ $# -gt 0 ]; do
      case "$1" in key) key=$(printf %s "$2" | tr "/" "_"); shift 2 ;; *) shift ;; esac
    done
    rm -f "$state/$key"
    exit 0 ;;
esac
exit 1
"#,
        state = state.display()
    );
    std::fs::write(&script, body).unwrap();
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).unwrap();
    script
}

#[test]
fn keyring_is_detected_without_the_version_flag() {
    // `secret-tool --version` is not a libsecret command: it exits 2. Detection must
    // rely on `lookup` returning 0 or 1 instead.
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let tool = fake_secret_tool(dir.path(), &state);
    assert!(KeyringSecretStore::probe_tool(&tool));
}

#[test]
fn detection_fails_when_the_tool_does_not_exist() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!KeyringSecretStore::probe_tool(&dir.path().join("nope")));
}

#[test]
fn the_keyring_backend_stores_and_reads_through_the_tool() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let tool = fake_secret_tool(dir.path(), &state);
    let store = KeyringSecretStore::with_tool(tool);
    assert_eq!(store.backend(), "keyring");
    store.store("alias-1", "Bearer secret").unwrap();
    assert_eq!(
        store.lookup("alias-1").unwrap().as_deref(),
        Some("Bearer secret")
    );
    store.delete("alias-1").unwrap();
    assert_eq!(store.lookup("alias-1").unwrap(), None);
}

#[test]
fn a_legacy_secret_file_is_imported_into_the_keyring_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let tool = fake_secret_tool(dir.path(), &state);
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let legacy = data.join("secrets.json");
    let secrets = FileSecretStore::open(&legacy).unwrap();
    secrets.store("download/abc", "Bearer legacy").unwrap();
    assert!(legacy.exists());

    let store = open_with(&data, Some(KeyringSecretStore::with_tool(tool))).unwrap();
    assert_eq!(store.backend(), "keyring");
    assert_eq!(
        store.lookup("download/abc").unwrap().as_deref(),
        Some("Bearer legacy"),
        "the legacy alias must be readable from the keyring"
    );
    assert!(
        !legacy.exists(),
        "the plaintext file must be removed after import"
    );
}

#[test]
fn without_a_tool_the_backend_is_unavailable_and_refuses_to_write() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let store = open_with(&data, None).unwrap();
    assert_eq!(store.backend(), "unavailable");
    let err = store.store("alias", "value").unwrap_err();
    assert!(matches!(err, SecretError::Unavailable(_)));
    assert!(
        !data.join("secrets.json").exists(),
        "no plaintext fallback may be created"
    );
}

#[test]
fn a_legacy_file_survives_when_the_keyring_is_not_available() {
    // No silent data loss: the file is left alone, not used, and not deleted.
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let legacy = data.join("secrets.json");
    FileSecretStore::open(&legacy)
        .unwrap()
        .store("download/old", "Bearer old")
        .unwrap();
    let store = open_with(&data, None).unwrap();
    assert_eq!(store.backend(), "unavailable");
    assert_eq!(store.lookup("download/old").unwrap(), None);
    assert!(legacy.exists());
}
