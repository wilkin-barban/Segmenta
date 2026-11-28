//! Pairing and per-device authorization: revoking one phone leaves the others working.

use std::sync::Arc;

use segmenta_daemon::events::EventBus;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::secrets::FileSecretStore;
use segmenta_daemon::store::Store;
use serde_json::json;

struct Harness {
    manager: Arc<Manager>,
    store: Arc<Store>,
    _dir: tempfile::TempDir,
}

fn harness(enabled: bool) -> Harness {
    use segmenta_daemon::ipc::RpcHandler;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("segmenta.db")).unwrap());
    let secrets: Arc<dyn segmenta_daemon::secrets::SecretStore> =
        Arc::new(FileSecretStore::open(&dir.path().join("secrets.json")).unwrap());
    let bus = Arc::new(EventBus::new());
    let (shutdown, _rx) = tokio::sync::watch::channel(false);
    let manager = Arc::new(Manager::new(Arc::clone(&store), secrets, bus, shutdown));
    if enabled {
        let handle = Arc::clone(&manager);
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                handle
                    .call("settings.update", Some(json!({ "remote_enabled": true })))
                    .await
                    .unwrap()
            });
    }
    Harness {
        manager,
        store,
        _dir: dir,
    }
}

#[test]
fn a_paired_device_gets_its_own_token_and_the_others_keep_working() {
    let h = harness(true);
    let code = h.manager.pair_issue("phone");
    let (first_id, first_token) = h.manager.pair_redeem(&code, "").unwrap();
    assert_eq!(h.store.remote_devices().unwrap().len(), 1);
    assert_eq!(
        h.manager.remote_authorize(&first_token).as_deref(),
        Some(first_id.as_str())
    );

    let second_code = h.manager.pair_issue("tablet");
    let (second_id, second_token) = h.manager.pair_redeem(&second_code, "tablet").unwrap();
    assert_eq!(h.store.remote_devices().unwrap().len(), 2);

    // Revoking one device must not disturb the other.
    assert!(h.store.remove_remote_device(&first_id).unwrap());
    assert!(h.manager.remote_authorize(&first_token).is_none());
    assert_eq!(
        h.manager.remote_authorize(&second_token).as_deref(),
        Some(second_id.as_str())
    );
}

#[test]
fn a_code_is_single_use_and_a_wrong_one_is_refused() {
    let h = harness(true);
    let code = h.manager.pair_issue("phone");
    assert!(h.manager.pair_redeem("00000000", "").is_err());
    assert!(h.manager.pair_redeem(&code, "").is_ok());
    assert!(
        h.manager.pair_redeem(&code, "").is_err(),
        "a used code must not pair a second device"
    );
    assert_eq!(h.store.remote_devices().unwrap().len(), 1);
}

#[test]
fn authorization_needs_remote_control_to_be_enabled() {
    let h = harness(false);
    let token = segmenta_daemon::remote::new_token();
    assert!(h.manager.remote_authorize(&token).is_none());
}

#[test]
fn only_the_digest_is_stored_and_the_token_never_is() {
    let h = harness(true);
    let code = h.manager.pair_issue("phone");
    let (_id, token) = h.manager.pair_redeem(&code, "").unwrap();
    let bytes = std::fs::read(h.store.path()).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes).to_string();
    assert!(!text.contains(&token), "the token reached the database");
    assert_eq!(h.store.remote_devices().unwrap()[0].token_digest.len(), 64);
}
