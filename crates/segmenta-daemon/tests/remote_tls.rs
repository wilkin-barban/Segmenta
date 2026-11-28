//! Remote TLS material: generated once, reused, and identifiable by its fingerprint.

use segmenta_daemon::remote_tls::RemoteTls;
use std::os::unix::fs::PermissionsExt;

fn cert_name() -> String {
    format!("remote-{}.pem", "cert")
}

fn key_name() -> String {
    format!("remote-{}.pem", "key")
}

#[test]
fn the_pair_is_generated_once_and_reused_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let first = RemoteTls::ensure(dir.path()).unwrap();
    assert!(first.cert_pem().contains("BEGIN CERTIFICATE"));
    assert!(first.key_pem().contains("PRIVATE KEY"));
    assert_eq!(first.fingerprint().split(':').count(), 32);

    // A second start must reuse the pair: a new certificate would break every paired phone.
    let second = RemoteTls::ensure(dir.path()).unwrap();
    assert_eq!(first.fingerprint(), second.fingerprint());
    assert_eq!(first.cert_pem(), second.cert_pem());
}

#[test]
fn the_private_key_is_only_readable_by_its_owner() {
    let dir = tempfile::tempdir().unwrap();
    let tls = RemoteTls::ensure(dir.path()).unwrap();
    for name in [cert_name(), key_name()] {
        let mode = std::fs::metadata(dir.path().join(&name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "{name} is {mode:o}");
    }
    assert_eq!(tls.path(), dir.path());
}

#[test]
fn a_world_readable_key_is_tightened_even_when_the_pair_is_reused() {
    // `OpenOptions::mode` only applies at creation: replacing or reusing a file that was
    // already 0644 used to leave the private key readable by every user on the machine.
    let dir = tempfile::tempdir().unwrap();
    let tls = RemoteTls::ensure(dir.path()).unwrap();
    let key = dir.path().join(key_name());
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();

    // Reusing the existing pair must repair the permissions, not honour them.
    RemoteTls::ensure(dir.path()).unwrap();
    let mode = std::fs::metadata(&key).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "reused key left at {mode:o}");
    assert!(tls.cert_pem().contains("BEGIN CERTIFICATE"));

    // And replacement (corrupt pair) must end at 0600 as well.
    std::fs::write(&key, "garbage").unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    RemoteTls::ensure(dir.path()).unwrap();
    let mode = std::fs::metadata(&key).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "replaced key left at {mode:o}");
}

#[test]
fn an_unreadable_pair_is_replaced_instead_of_failing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(cert_name()), "garbage").unwrap();
    std::fs::write(dir.path().join(key_name()), "garbage").unwrap();
    let tls = RemoteTls::ensure(dir.path()).unwrap();
    assert!(tls.cert_pem().contains("BEGIN CERTIFICATE"));
    let stored = std::fs::read_to_string(dir.path().join(cert_name())).unwrap();
    assert!(stored.contains("BEGIN CERTIFICATE"));
}

#[test]
fn an_old_pair_is_renewed_and_the_fingerprint_changes_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let first = RemoteTls::ensure(dir.path()).unwrap();
    // Age the certificate past the renewal window: one year on disk, thirty days of margin.
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(340 * 24 * 60 * 60);
    let file = std::fs::File::options()
        .write(true)
        .open(dir.path().join(cert_name()))
        .unwrap();
    file.set_modified(old).unwrap();
    drop(file);
    let renewed = RemoteTls::ensure(dir.path()).unwrap();
    assert_ne!(
        first.fingerprint(),
        renewed.fingerprint(),
        "a renewal must be visible to the user"
    );
}

#[test]
fn the_debug_never_shows_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let tls = RemoteTls::ensure(dir.path()).unwrap();
    let printed = format!("{tls:?}");
    assert!(printed.contains("fingerprint"));
    assert!(!printed.contains("PRIVATE KEY"), "{printed}");
    let head = &tls.key_pem()[..40.min(tls.key_pem().len())];
    assert!(!printed.contains(head));
}

#[test]
fn the_status_reports_https_a_fingerprint_and_the_device_count() {
    use segmenta_daemon::events::EventBus;
    use segmenta_daemon::ipc::RpcHandler;
    use segmenta_daemon::manager::Manager;
    use segmenta_daemon::secrets::FileSecretStore;
    use segmenta_daemon::store::Store;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    // The status must not depend on `~/.local/share/segmenta`: it reads what the daemon
    // would serve, which is the same material the CLI prints for the user to compare.
    let previous = std::env::var("XDG_DATA_HOME").ok();
    std::env::set_var("XDG_DATA_HOME", dir.path());
    let store = Arc::new(Store::open(&dir.path().join("segmenta.db")).unwrap());
    let secrets: Arc<dyn segmenta_daemon::secrets::SecretStore> =
        Arc::new(FileSecretStore::open(&dir.path().join("secrets.json")).unwrap());
    let (shutdown, _rx) = tokio::sync::watch::channel(false);
    let manager = Arc::new(Manager::new(
        store,
        secrets,
        Arc::new(EventBus::new()),
        shutdown,
    ));
    let status = tokio::runtime::Runtime::new().unwrap().block_on(async {
        manager
            .call(
                "settings.update",
                Some(serde_json::json!({ "remote_enabled": true })),
            )
            .await
            .unwrap();
        manager
            .call("remote.status", Some(serde_json::json!({})))
            .await
            .unwrap()
    });
    match previous {
        Some(value) => std::env::set_var("XDG_DATA_HOME", value),
        None => std::env::remove_var("XDG_DATA_HOME"),
    }
    assert_eq!(status["scheme"], "https", "{status}");
    assert_eq!(status["devices"], 0, "{status}");
    let fingerprint = status["fingerprint"].as_str().unwrap_or_default();
    assert_eq!(fingerprint.split(':').count(), 32, "{status}");
}
