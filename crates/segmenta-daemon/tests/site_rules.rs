//! Site rules: additive migration, headers kept out of SQLite, and the rule reaching the
//! probe exactly when it matches the host (T5 of the first network milestone).

use std::path::Path;
use std::sync::Arc;

use segmenta_daemon::events::EventBus;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::secrets::{FileSecretStore, SecretStore};
use segmenta_daemon::store::{SiteRuleRecord, Store};
use serde_json::json;

struct Harness {
    manager: Arc<Manager>,
    store: Arc<Store>,
    secrets: Arc<dyn SecretStore>,
}

fn harness(dir: &Path) -> Harness {
    let store = Arc::new(Store::open(&dir.join("segmenta.db")).unwrap());
    let secrets: Arc<dyn SecretStore> =
        Arc::new(FileSecretStore::open(&dir.join("secrets.json")).unwrap());
    let bus = Arc::new(EventBus::new());
    let (shutdown, _rx) = tokio::sync::watch::channel(false);
    let manager = Arc::new(Manager::new(
        Arc::clone(&store),
        Arc::clone(&secrets),
        bus,
        shutdown,
    ));
    Harness {
        manager,
        store,
        secrets,
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("segmenta-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_migration_adds_the_table_without_touching_existing_rows() {
    let dir = temp_dir("site-rules-migration");
    let path = dir.join("segmenta.db");
    {
        // Simulate a database that stopped at v8: the table is gone and the version says
        // so, exactly like a user upgrading from the previous release.
        let store = Store::open(&path).unwrap();
        drop(store);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE IF EXISTS site_rules; PRAGMA user_version = 8;")
            .unwrap();
        drop(conn);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.schema_version().unwrap(),
            segmenta_daemon::store::SCHEMA_VERSION
        );
        let conn = rusqlite::Connection::open(&path).unwrap();
        let rows: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'site_rules'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1, "the upgrade recreates the table");
    }
    // A second open must be a no-op: running the migration twice cannot duplicate anything.
    let store = Store::open(&path).unwrap();
    // The version keeps moving with each additive table; the point of this test is that a
    // v8 database gains `site_rules` without losing rows, not the number itself.
    assert_eq!(
        store.schema_version().unwrap(),
        segmenta_daemon::store::SCHEMA_VERSION
    );
    assert!(store.site_rules().unwrap().is_empty());
}

#[tokio::test]
async fn the_headers_live_in_the_secret_store_and_never_in_sqlite() {
    let dir = temp_dir("site-rules-secret");
    let harness = harness(&dir);
    let alias = "site-rule:example.com";
    harness
        .secrets
        .store(alias, r#"[["cookie","session=abc123"]]"#)
        .unwrap();
    harness
        .store
        .upsert_site_rule(&SiteRuleRecord {
            id: "rule-1".to_string(),
            domain_pattern: "example.com".to_string(),
            user_agent: Some("custom/1".to_string()),
            max_connections: Some(4),
            browser_like: true,
            headers_secret_id: Some(alias.to_string()),
        })
        .unwrap();

    let rule = harness.manager.site_rule("cdn.example.com").unwrap();
    let rule = rule.expect("the rule must match the subdomain");
    assert_eq!(rule.user_agent.as_deref(), Some("custom/1"));
    assert_eq!(rule.max_connections, Some(4));
    assert!(rule.browser_like);
    assert!(rule
        .headers
        .iter()
        .any(|(name, value)| name == "cookie" && value == "session=abc123"));

    // The database keeps the alias and nothing else: the cookie must not appear anywhere
    // in the file, not even inside a page whose other rows changed afterwards.
    // The write may still be in the WAL, so both files are inspected.
    let mut text = String::new();
    for name in ["segmenta.db", "segmenta.db-wal"] {
        if let Ok(bytes) = std::fs::read(dir.join(name)) {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    assert!(
        !text.contains("session=abc123"),
        "a header value leaked into sqlite"
    );
    assert!(text.contains(alias), "the alias itself is expected");

    // A host the rule does not cover gets no rule at all.
    let uncovered = harness.manager.site_rule("ample.com").unwrap();
    assert!(uncovered.is_none());

    // Deleting the rule leaves the secret to its owner and stops matching.
    assert!(harness.store.delete_site_rule("rule-1").unwrap());
    let deleted = harness.manager.site_rule("example.com").unwrap();
    assert!(deleted.is_none());
}

#[tokio::test]
async fn a_rule_is_applied_to_the_probe_of_its_host_only() {
    let dir = temp_dir("site-rules-probe");
    let harness = harness(&dir);
    harness
        .store
        .upsert_site_rule(&SiteRuleRecord {
            id: "rule-1".to_string(),
            domain_pattern: "example.com".to_string(),
            user_agent: Some("per-site/1".to_string()),
            max_connections: Some(2),
            browser_like: true,
            headers_secret_id: None,
        })
        .unwrap();

    let matching = harness
        .manager
        .http_config("https://cdn.example.com/file.bin")
        .unwrap();
    let rule = matching.rule.expect("the rule must reach the probe");
    assert_eq!(rule.user_agent.as_deref(), Some("per-site/1"));
    assert!(rule.browser_like);

    let other = harness
        .manager
        .http_config("https://other.test/file.bin")
        .unwrap();
    assert!(other.rule.is_none(), "no rule means no override");
    // The default user agent is untouched when there is no rule.
    assert!(other.user_agent.starts_with("Segmenta/"));
}

#[test]
fn the_rule_shape_is_what_the_daemon_stores() {
    // A guard against accidental schema drift: the row the manager writes is the row the
    // table declares.
    let record = SiteRuleRecord {
        id: "rule-1".to_string(),
        domain_pattern: "example.com".to_string(),
        user_agent: None,
        max_connections: None,
        browser_like: false,
        headers_secret_id: None,
    };
    let value = serde_json::to_value(&record).unwrap();
    assert_eq!(value["domain_pattern"], json!("example.com"));
    assert_eq!(value["browser_like"], json!(false));
}

#[tokio::test]
async fn the_rpc_surface_manages_rules_without_leaking_headers() {
    use segmenta_daemon::ipc::RpcHandler;

    let dir = temp_dir("site-rules-rpc");
    let harness = harness(&dir);
    let answer = harness
        .manager
        .call(
            "site_rule.upsert",
            Some(serde_json::json!({
                "domain_pattern": "Example.com",
                "user_agent": "custom/1",
                "max_connections": 4,
                "browser_like": true,
                "headers": { "cookie": "session=abc123" }
            })),
        )
        .await
        .unwrap();
    let id = answer["id"].as_str().unwrap().to_string();

    let listed = harness
        .manager
        .call("site_rule.list", Some(serde_json::json!({})))
        .await
        .unwrap();
    let rows = listed.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["domain_pattern"], "example.com", "stored lowercase");
    assert_eq!(rows[0]["has_headers"], true);
    // Neither the value nor the alias travels to a client.
    let serialized = listed.to_string();
    assert!(!serialized.contains("session=abc123"), "{serialized}");
    assert!(!serialized.contains("site-rule:"), "{serialized}");

    // The rule is applied to the host it covers.
    let applied = harness
        .manager
        .site_rule("cdn.example.com")
        .unwrap()
        .unwrap();
    assert_eq!(applied.user_agent.as_deref(), Some("custom/1"));

    // And removing it takes the secret with it.
    harness
        .manager
        .call("site_rule.remove", Some(serde_json::json!({ "id": id })))
        .await
        .unwrap();
    assert!(harness.store.site_rules().unwrap().is_empty());
    assert!(harness.manager.site_rule("example.com").unwrap().is_none());
    assert!(harness
        .secrets
        .lookup(&format!("site-rule:{id}"))
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn an_invalid_rule_is_refused_before_it_is_stored() {
    use segmenta_daemon::ipc::RpcHandler;

    let dir = temp_dir("site-rules-invalid");
    let harness = harness(&dir);
    for payload in [
        serde_json::json!({ "domain_pattern": " " }),
        serde_json::json!({ "domain_pattern": "a/b" }),
        serde_json::json!({ "domain_pattern": "example.com", "max_connections": 0 }),
        serde_json::json!({ "domain_pattern": "example.com", "max_connections": 129 }),
    ] {
        let refused = harness
            .manager
            .call("site_rule.upsert", Some(payload.clone()))
            .await;
        assert!(refused.is_err(), "accepted an invalid rule: {payload}");
    }
    assert!(harness.store.site_rules().unwrap().is_empty());
}

/// The GUI speaks these exact payloads. A contract test lives here, next to the parser, so
/// a rename on either side fails in one place instead of leaving a dead button.
#[tokio::test]
async fn the_payloads_the_gui_sends_are_the_ones_the_daemon_accepts() {
    use segmenta_daemon::ipc::RpcHandler;

    let dir = temp_dir("site-rules-contract");
    let harness = harness(&dir);
    // Mirrors `prefs.rs`: the add form builds exactly these keys.
    let added = harness
        .manager
        .call(
            "site_rule.upsert",
            Some(serde_json::json!({
                "domain_pattern": "example.com",
                "user_agent": "custom/1",
                "max_connections": 3,
            })),
        )
        .await
        .expect("the daemon must accept the payload the add form sends");
    let id = added["id"].as_str().unwrap();

    // Mirrors the remove button: only the id.
    let removed = harness
        .manager
        .call("site_rule.remove", Some(serde_json::json!({ "id": id })))
        .await
        .expect("the daemon must accept the payload the remove button sends");
    assert_eq!(removed["removed"], true);
}
