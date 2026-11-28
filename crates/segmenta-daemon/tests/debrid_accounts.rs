//! Debrid accounts and the resolution seam: the daemon, not the dialog, decides who resolves.
//!
//! The HTTP layer is injected, which is what makes this hermetic: the seam under test is the
//! priority order, the fallback and the `direct` flag, not `reqwest` itself.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use segmenta_core::debrid::{DebridError, DebridHttp, DebridRequest};
use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::RpcHandler;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::secrets::{FileSecretStore, SecretStore};
use segmenta_daemon::store::Store;
use serde_json::{json, Value};

/// Answers by provider host, and records the order in which providers were asked.
struct Mock {
    /// host → (status, body)
    answers: Mutex<std::collections::HashMap<String, (u16, String)>>,
    asked: Mutex<Vec<String>>,
    calls: AtomicUsize,
}

impl Mock {
    fn new(answers: &[(&str, u16, &str)]) -> Arc<Mock> {
        Arc::new(Mock {
            answers: Mutex::new(
                answers
                    .iter()
                    .map(|(host, status, body)| (host.to_string(), (*status, body.to_string())))
                    .collect(),
            ),
            asked: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        })
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl DebridHttp for Mock {
    async fn send(&self, request: &DebridRequest) -> Result<(u16, String), DebridError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let host = request
            .url
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or_default()
            .to_string();
        self.asked.lock().unwrap().push(host.clone());
        Ok(self
            .answers
            .lock()
            .unwrap()
            .get(&host)
            .cloned()
            .unwrap_or((404, String::new())))
    }
}

struct Harness {
    manager: Arc<Manager>,
    store: Arc<Store>,
    secrets: Arc<dyn SecretStore>,
    _dir: tempfile::TempDir,
}

fn harness(mock: Arc<Mock>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("segmenta.db")).unwrap());
    let secrets: Arc<dyn SecretStore> =
        Arc::new(FileSecretStore::open(&dir.path().join("secrets.json")).unwrap());
    let (shutdown, _rx) = tokio::sync::watch::channel(false);
    let manager = Manager::new(
        Arc::clone(&store),
        Arc::clone(&secrets),
        Arc::new(EventBus::new()),
        shutdown,
    )
    .with_debrid_http(mock);
    Harness {
        manager: Arc::new(manager),
        store,
        secrets,
        _dir: dir,
    }
}

async fn call(manager: &Arc<Manager>, method: &str, params: Value) -> Result<Value, (i64, String)> {
    manager
        .call(method, Some(params))
        .await
        .map_err(|err| (err.code, err.message))
}

#[tokio::test]
async fn the_highest_priority_account_resolves_the_link() {
    let mock = Mock::new(&[(
        "api.real-debrid.com",
        200,
        r#"{"download":"https://cdn.example/f.zip"}"#,
    )]);
    let h = harness(Arc::clone(&mock));
    // Two accounts of the same provider: the second has better priority.
    call(
        &h.manager,
        "debrid.account.add",
        json!({ "provider": "realdebrid", "label": "slow", "priority": 200, "token": "t2", "id": "b" }),
    )
    .await
    .unwrap();
    call(
        &h.manager,
        "debrid.account.add",
        json!({ "provider": "realdebrid", "label": "fast", "priority": 10, "token": "t1", "id": "a" }),
    )
    .await
    .unwrap();

    let resolved = call(
        &h.manager,
        "debrid.resolve",
        json!({ "link": "https://hoster.example/f.zip" }),
    )
    .await
    .unwrap();
    assert_eq!(resolved["link"], "https://cdn.example/f.zip");
    assert_eq!(resolved["provider"], "realdebrid");
    // Exactly one account was asked: the best one answered.
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    let listed = call(&h.manager, "debrid.accounts", json!({}))
        .await
        .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 2);
    // The token never leaves the keyring.
    assert!(!listed.to_string().contains("\"t1\""));
    assert!(h
        .secrets
        .lookup("debrid:a")
        .unwrap()
        .as_deref()
        .is_some_and(|token| token == "t1"));
}

#[tokio::test]
async fn a_failing_account_falls_back_to_the_next_one() {
    let mock = Mock::new(&[(
        "api.alldebrid.com",
        200,
        r#"{"status":"success","data":{"link":"https://cdn.example/second.zip"}}"#,
    )]);
    let h = harness(Arc::clone(&mock));
    // The preferred account is a provider that fails; the next one can serve.
    call(
        &h.manager,
        "debrid.account.add",
        json!({ "provider": "realdebrid", "priority": 10, "token": "t1", "id": "a" }),
    )
    .await
    .unwrap();
    call(
        &h.manager,
        "debrid.account.add",
        json!({ "provider": "alldebrid", "priority": 20, "token": "t2", "id": "b" }),
    )
    .await
    .unwrap();
    let resolved = call(
        &h.manager,
        "debrid.resolve",
        json!({ "link": "https://hoster.example/f.zip" }),
    )
    .await
    .unwrap();
    assert_eq!(resolved["link"], "https://cdn.example/second.zip");
    assert_eq!(resolved["provider"], "alldebrid");
    assert_eq!(
        mock.asked(),
        vec!["api.real-debrid.com", "api.alldebrid.com"]
    );
}

#[tokio::test]
async fn direct_bypasses_every_account() {
    let mock = Mock::new(&[]);
    let h = harness(Arc::clone(&mock));
    call(
        &h.manager,
        "debrid.account.add",
        json!({ "provider": "realdebrid", "token": "t1" }),
    )
    .await
    .unwrap();
    let resolved = call(
        &h.manager,
        "debrid.resolve",
        json!({ "link": "https://hoster.example/f.zip", "direct": true }),
    )
    .await
    .unwrap();
    assert_eq!(resolved["link"], "https://hoster.example/f.zip");
    assert_eq!(resolved["resolved"], false);
    assert_eq!(
        mock.calls.load(Ordering::SeqCst),
        0,
        "direct must not call out"
    );
}

#[tokio::test]
async fn removing_an_account_deletes_its_token_and_leaves_the_others() {
    let h = harness(Mock::new(&[]));
    for id in ["a", "b"] {
        call(
            &h.manager,
            "debrid.account.add",
            json!({ "provider": "realdebrid", "token": format!("token-{id}"), "id": id }),
        )
        .await
        .unwrap();
    }
    call(&h.manager, "debrid.account.remove", json!({ "id": "a" }))
        .await
        .unwrap();
    assert!(h.secrets.lookup("debrid:a").unwrap().is_none());
    assert!(h.secrets.lookup("debrid:b").unwrap().is_some());
    assert_eq!(h.store.debrid_accounts().unwrap().len(), 1);
}

#[tokio::test]
async fn without_accounts_the_legacy_provider_alias_still_works() {
    let mock = Mock::new(&[(
        "api.real-debrid.com",
        200,
        r#"{"download":"https://cdn.example/legacy.zip"}"#,
    )]);
    let h = harness(Arc::clone(&mock));
    h.secrets
        .store("debrid:realdebrid", "legacy-token")
        .unwrap();
    call(
        &h.manager,
        "settings.update",
        json!({ "debrid_provider": "realdebrid" }),
    )
    .await
    .unwrap();
    let resolved = call(
        &h.manager,
        "debrid.resolve",
        json!({ "link": "https://hoster.example/f.zip" }),
    )
    .await
    .unwrap();
    assert_eq!(resolved["link"], "https://cdn.example/legacy.zip");
}
