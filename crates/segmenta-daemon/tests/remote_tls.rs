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
