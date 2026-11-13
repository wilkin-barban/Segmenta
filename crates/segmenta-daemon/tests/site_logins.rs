//! Site logins by exact origin (L1, OpenSpec 12 §6).
//!
//! The previous attempt at this unit was reverted because its own test caught a leak:
//! with a login for one origin, a task on the same host but another port still received
//! `Authorization: Basic`. These tests are that test, kept.

use std::path::Path;
use std::sync::Arc;

use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::RpcHandler;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::secrets::{FileSecretStore, SecretStore};
use segmenta_daemon::store::Store;
use serde_json::{json, Value};
use tokio::sync::watch;

struct Harness {
    manager: Arc<Manager>,
    store: Arc<Store>,
}

fn harness(dir: &Path) -> Harness {
    let store = Arc::new(Store::open(&dir.join("segmenta.db")).unwrap());
    // A real store on disk: the happy path needs a keyring, and the file store is the
    // same trait with the same semantics for this contract.
    let secrets: Arc<dyn SecretStore> =
        Arc::new(FileSecretStore::open(&dir.join("secrets.json")).unwrap());
    let bus = Arc::new(EventBus::new());
    let (shutdown, _rx) = watch::channel(false);
    let manager = Arc::new(Manager::new(Arc::clone(&store), secrets, bus, shutdown));
    Harness { manager, store }
}

async fn call(manager: &Arc<Manager>, method: &str, params: Value) -> Result<Value, (i64, String)> {
    manager
        .call(method, Some(params))
        .await
        .map_err(|err| (err.code, err.message))
}

const ORIGIN: &str = "https://files.example";

#[tokio::test]
async fn the_exact_origin_gets_basic_auth_and_no_neighbour_does() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    call(
        &h.manager,
        "site_login.set",
        json!({ "origin": ORIGIN, "username": "ana", "password": "s3cr3t" }),
    )
    .await
    .expect("the login is stored");

    let header = h
        .manager
        .site_login_header("https://files.example/path/file.bin")
        .expect("the exact origin carries the credentials");
    assert_eq!(header.to_str().unwrap(), "Basic YW5hOnMzY3IzdA==");

    // The leak: same host, another port is another origin.
    assert!(
        h.manager
            .site_login_header("https://files.example:8443/file.bin")
            .is_none(),
        "a different port must not receive the credentials"
    );
    // Another scheme is another origin.
    assert!(
        h.manager
            .site_login_header("http://files.example/file.bin")
            .is_none(),
        "a different scheme must not receive the credentials"
    );
    // Another host, obviously.
    assert!(
        h.manager
            .site_login_header("https://other.example/file.bin")
            .is_none(),
        "a different host must not receive the credentials"
    );
}

#[tokio::test]
async fn listing_never_returns_the_password() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    call(
        &h.manager,
        "site_login.set",
        json!({ "origin": ORIGIN, "username": "ana", "password": "s3cr3t" }),
    )
    .await
    .unwrap();

    let listed = call(&h.manager, "site_login.list", json!({}))
        .await
        .unwrap();
    let rows = listed.as_array().expect("list is an array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["origin"], ORIGIN);
    assert_eq!(rows[0]["username"], "ana");
    assert!(
        rows[0].get("password").is_none(),
        "the list must not carry a password field: {listed}"
    );
    let raw = serde_json::to_string(&listed).unwrap();
    assert!(!raw.contains("s3cr3t"), "the password leaked: {raw}");
}

#[tokio::test]
async fn a_legacy_row_is_listed_but_never_injects() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    // A row as 0.1.2 wrote it: host only, no origin.
    h.store
        .upsert_site_login(&segmenta_daemon::store::SiteLoginRow {
            domain: "files.example".to_string(),
            username: "ana".to_string(),
            alias: "site/files.example".to_string(),
            origin: String::new(),
        })
        .unwrap();

    let listed = call(&h.manager, "site_login.list", json!({}))
        .await
        .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1, "the row stays visible");
    assert!(
        h.manager
            .site_login_header("https://files.example/file.bin")
            .is_none(),
        "a legacy row without an origin never sends credentials"
    );
}

#[tokio::test]
async fn an_origin_with_a_path_or_credentials_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    for bad in [
        "https://files.example/private",
        "https://user:pass@files.example",
        "ftp://files.example",
    ] {
        let error = call(
            &h.manager,
            "site_login.set",
            json!({ "origin": bad, "username": "ana", "password": "s3cr3t" }),
        )
        .await
        .expect_err(bad);
        assert_eq!(error.0, -32602, "{bad} was not rejected");
    }
    assert_eq!(
        call(&h.manager, "site_login.list", json!({}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0,
        "a rejected origin stores nothing"
    );
}

#[tokio::test]
async fn removing_an_origin_removes_the_login_and_its_secret() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    call(
        &h.manager,
        "site_login.set",
        json!({ "origin": ORIGIN, "username": "ana", "password": "s3cr3t" }),
    )
    .await
    .unwrap();
    let removed = call(&h.manager, "site_login.remove", json!({ "origin": ORIGIN }))
        .await
        .unwrap();
    assert_eq!(removed["removed"], true);
    assert!(h
        .manager
        .site_login_header("https://files.example/x")
        .is_none());
}
