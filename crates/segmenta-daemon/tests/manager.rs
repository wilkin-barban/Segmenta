//! Manager contract tests: pause/resume state rules, paused speed, secret backend
//! requirements and the XDG download directory. Specs S2, S4-S8.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use segmenta_core::testserver::{RangeServer, ServerConfig};
use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::RpcHandler;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::secrets::{open_with, KeyringSecretStore, SecretStore};
use segmenta_daemon::store::{
    resolve_download_dir, DownloadRecord, DownloadStatus, Settings, Store,
};
use serde_json::{json, Value};
use tokio::sync::watch;

const MB: usize = 1024 * 1024;

fn record(id: &str, status: DownloadStatus) -> DownloadRecord {
    DownloadRecord {
        id: id.to_string(),
        url: "http://127.0.0.1:1/file.bin".to_string(),
        final_url: None,
        final_path: format!("/tmp/{id}.bin"),
        temp_path: None,
        total_bytes: Some(1024),
        downloaded_bytes: 0,
        status,
        supports_ranges: true,
        etag: None,
        last_modified: None,
        content_type: None,
        expected_checksum: None,
        error_code: None,
        error_message: None,
        retry_count: 0,
        priority: 0,
        queue_name: "main".to_string(),
        download_type: "direct".to_string(),
        created_at: None,
        completed_at: None,
        filename: format!("{id}.bin"),
        media_kind: None,
        media_format: None,
        playlist_title: None,
        playlist_count: None,
        batch_id: None,
    }
}

struct Harness {
    manager: Arc<Manager>,
    store: Arc<Store>,
    bus: Arc<EventBus>,
}

fn harness(dir: &Path) -> Harness {
    let store = Arc::new(Store::open(&dir.join("segmenta.db")).unwrap());
    let secrets: Arc<dyn SecretStore> = Arc::from(open_with(dir, None).unwrap());
    let bus = Arc::new(EventBus::new());
    let (shutdown, _rx) = watch::channel(false);
    let manager = Arc::new(Manager::new(
        Arc::clone(&store),
        secrets,
        Arc::clone(&bus),
        shutdown,
    ));
    Harness {
        manager,
        store,
        bus,
    }
}

async fn call(manager: &Arc<Manager>, method: &str, params: Value) -> Result<Value, (i64, String)> {
    manager
        .call(method, Some(params))
        .await
        .map_err(|err| (err.code, err.message))
}

#[test]
fn download_dir_resolution_follows_the_documented_order() {
    // 1. XDG_DOWNLOAD_DIR wins over everything.
    assert_eq!(
        resolve_download_dir(
            Path::new("/home/u"),
            Some("/data/descargas"),
            Some("/home/u/Otra"),
            Some("XDG_DOWNLOAD_DIR=\"$HOME/Descargas\"\n")
        ),
        Path::new("/data/descargas")
    );
    // 2. xdg-user-dir output, when it is not the home directory itself.
    assert_eq!(
        resolve_download_dir(
            Path::new("/home/u"),
            None,
            Some("/home/u/Descargas"),
            Some("XDG_DOWNLOAD_DIR=\"$HOME/Otra\"\n")
        ),
        Path::new("/home/u/Descargas")
    );
    // 3. user-dirs.dirs with $HOME expansion.
    assert_eq!(
        resolve_download_dir(
            Path::new("/home/u"),
            None,
            Some("/home/u"),
            Some("# comment\nXDG_DOWNLOAD_DIR=\"$HOME/Descargas\"\n")
        ),
        Path::new("/home/u/Descargas")
    );
    // 4. The historical fallback, only when nothing else exists.
    assert_eq!(
        resolve_download_dir(Path::new("/home/u"), None, None, None),
        Path::new("/home/u/Downloads")
    );
    // Relative or empty values are ignored.
    assert_eq!(
        resolve_download_dir(
            Path::new("/home/u"),
            Some("relative"),
            None,
            Some("XDG_DOWNLOAD_DIR=\"$HOME/Descargas\"\n")
        ),
        Path::new("/home/u/Descargas")
    );
}

#[test]
fn the_historical_default_is_migrated_once_and_a_custom_value_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("segmenta.db")).unwrap();
    let legacy = "/home/u/Downloads";
    let resolved = "/home/u/Descargas";

    let mut settings = Settings {
        download_dir: legacy.to_string(),
        ..Settings::default()
    };
    store
        .update_settings(
            json!({ "download_dir": settings.download_dir })
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert!(store.migrate_download_dir(legacy, resolved).unwrap());
    settings = store.settings().unwrap();
    assert_eq!(settings.download_dir, resolved);
    // The migration runs once: the stored value is no longer the legacy one.
    assert!(!store.migrate_download_dir(legacy, resolved).unwrap());

    // A user-chosen directory is never rewritten.
    store
        .update_settings(
            json!({ "download_dir": "/mnt/mis-descargas" })
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert!(!store.migrate_download_dir(legacy, resolved).unwrap());
    assert_eq!(store.settings().unwrap().download_dir, "/mnt/mis-descargas");
}

#[tokio::test]
async fn pause_rejects_terminal_states_without_changing_them() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    for (id, status) in [
        ("failed", DownloadStatus::Failed),
        ("completed", DownloadStatus::Completed),
        ("cancelled", DownloadStatus::Cancelled),
        ("paused", DownloadStatus::Paused),
    ] {
        h.store.insert_download(&record(id, status)).unwrap();
        let mut events = h.bus.subscribe();
        let err = call(&h.manager, "download.pause", json!({ "task_id": id }))
            .await
            .expect_err("pause must be rejected");
        assert_eq!(err.0, 1011, "task {id}: {err:?}");
        assert!(
            err.1.contains("TASK_NOT_PAUSABLE") || err.1.contains("cannot be paused"),
            "message: {}",
            err.1
        );
        assert_eq!(h.store.get(id).unwrap().unwrap().status, status);
        assert!(
            events.try_recv().is_err(),
            "a rejected pause must not publish events"
        );
    }
}

#[tokio::test]
async fn resume_rejects_terminal_states_without_changing_them() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    for (id, status) in [
        ("failed", DownloadStatus::Failed),
        ("completed", DownloadStatus::Completed),
        ("cancelled", DownloadStatus::Cancelled),
    ] {
        h.store.insert_download(&record(id, status)).unwrap();
        let mut events = h.bus.subscribe();
        let err = call(&h.manager, "download.resume", json!({ "task_id": id }))
            .await
            .expect_err("resume must be rejected");
        assert_eq!(err.0, 1003, "task {id}: {err:?}");
        assert_eq!(h.store.get(id).unwrap().unwrap().status, status);
        assert!(
            events.try_recv().is_err(),
            "a rejected resume must not publish events"
        );
    }
}

#[tokio::test]
async fn pause_accepts_a_pending_task_and_resume_starts_a_paused_one() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = RangeServer::start(ServerConfig {
        body: vec![7u8; 256 * 1024],
        ..ServerConfig::default()
    })
    .await;

    // A pending task can be paused (it never started) and then resumed.
    h.store
        .insert_download(&record("pending", DownloadStatus::Pending))
        .unwrap();
    let paused = call(
        &h.manager,
        "download.pause",
        json!({ "task_id": "pending" }),
    )
    .await
    .unwrap();
    assert_eq!(paused["status"], "paused");
    assert_eq!(
        h.store.get("pending").unwrap().unwrap().status,
        DownloadStatus::Paused
    );

    // A paused task pointing at a live server resumes and completes.
    let mut settings = Settings {
        allow_local_downloads: true,
        ..Settings::default()
    };
    settings.download_dir = dir.path().join("downloads").display().to_string();
    call(
        &h.manager,
        "settings.update",
        serde_json::to_value(&settings).unwrap(),
    )
    .await
    .unwrap();
    let mut paused_record = record("real", DownloadStatus::Paused);
    paused_record.url = server.url().to_string();
    h.store.insert_download(&paused_record).unwrap();

    let resumed = call(&h.manager, "download.resume", json!({ "task_id": "real" }))
        .await
        .unwrap();
    assert_eq!(resumed["status"], "downloading");
    let mut status = "downloading".to_string();
    for _ in 0..100 {
        status = h
            .store
            .get("real")
            .unwrap()
            .unwrap()
            .status
            .as_str()
            .to_string();
        if status == "completed" || status == "failed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(status, "completed");
    server.shutdown().await;
}

#[tokio::test]
async fn a_paused_task_reports_zero_speed_and_no_eta() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = RangeServer::start(ServerConfig {
        body: (0..4 * MB).map(|i| (i % 251) as u8).collect(),
        chunk_size: 8 * 1024,
        slow: Some(Duration::from_millis(4)),
        ..ServerConfig::default()
    })
    .await;

    let settings = Settings {
        allow_local_downloads: true,
        download_dir: dir.path().join("downloads").display().to_string(),
        max_connections: 2,
        ..Settings::default()
    };
    call(
        &h.manager,
        "settings.update",
        serde_json::to_value(&settings).unwrap(),
    )
    .await
    .unwrap();

    let mut events = h.bus.subscribe();
    let added = call(
        &h.manager,
        "download.add",
        json!({ "url": server.url().to_string(), "connections": 2 }),
    )
    .await
    .unwrap();
    let task_id = added["task_id"].as_str().unwrap().to_string();

    // Wait for a progress event with a real speed, then pause.
    let mut saw_speed = false;
    for _ in 0..200 {
        match tokio::time::timeout(Duration::from_millis(250), events.recv()).await {
            Ok(Ok(event))
                if event.event_type == "task_progress"
                    && event.data["speed_bps"].as_u64().unwrap_or(0) > 0 =>
            {
                saw_speed = true;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_speed, "the engine never reported a speed");

    call(&h.manager, "download.pause", json!({ "task_id": task_id }))
        .await
        .unwrap();
    let status = call(&h.manager, "download.status", json!({ "task_id": task_id }))
        .await
        .unwrap();
    assert_eq!(status["status"], "paused");
    assert_eq!(status["speed_bps"], 0, "paused speed must be zero");
    assert!(status["eta_seconds"].is_null(), "paused ETA must be null");
    let segments = status["segments"].as_array().expect("segments");
    assert!(!segments.is_empty());
    assert!(segments
        .iter()
        .all(|segment| segment["speed_bps"].as_u64() == Some(0)));
    server.shutdown().await;
}

#[tokio::test]
async fn add_with_sensitive_headers_requires_a_secret_backend() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let err = call(
        &h.manager,
        "download.add",
        json!({
            "url": "https://example.test/file.bin",
            "headers": { "Authorization": "Bearer secret" },
        }),
    )
    .await
    .expect_err("without a backend the secret cannot be stored");
    assert_eq!(err.0, 1031, "{err:?}");
    assert!(
        h.store.list(None, 10, 0).unwrap().is_empty(),
        "no task may be created when its secret cannot be stored"
    );
}

#[tokio::test]
async fn add_with_a_working_keyring_stores_the_secret_out_of_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("keyring");
    std::fs::create_dir_all(&state).unwrap();
    let tool = dir.path().join("secret-tool");
    std::fs::write(
        &tool,
        format!(
            "#!/bin/sh\nstate=\"{}\"\ncmd=\"$1\"; shift || true\ncase \"$cmd\" in\n\
             lookup) key=\"\"; while [ $# -gt 0 ]; do case \"$1\" in key) key=$(printf %s \"$2\" | tr \"/\" \"_\"); shift 2;; *) shift;; esac; done;\n\
             if [ -f \"$state/$key\" ]; then cat \"$state/$key\"; exit 0; fi; exit 1;;\n\
             store) key=\"\"; while [ $# -gt 0 ]; do case \"$1\" in key) key=$(printf %s \"$2\" | tr \"/\" \"_\"); shift 2;; *) shift;; esac; done;\n\
             mkdir -p \"$state\"; cat > \"$state/$key\"; exit 0;;\n\
             clear) exit 0;;\nesac\nexit 1\n",
            state.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&tool).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&tool, permissions).unwrap();
    }
    let secrets: Arc<dyn SecretStore> = Arc::new(KeyringSecretStore::with_tool(tool));
    let store = Arc::new(Store::open(&dir.path().join("segmenta.db")).unwrap());
    let bus = Arc::new(EventBus::new());
    let (shutdown, _rx) = watch::channel(false);
    let manager = Arc::new(Manager::new(
        Arc::clone(&store),
        Arc::clone(&secrets),
        bus,
        shutdown,
    ));

    let added = call(
        &manager,
        "download.add",
        json!({
            "url": "https://example.test/file.bin",
            "headers": { "Authorization": "Bearer secret", "Referer": "https://example.test/" },
            "start_immediately": false,
        }),
    )
    .await
    .unwrap();
    let task_id = added["task_id"].as_str().unwrap().to_string();
    // Non-sensitive headers stay in SQLite; the secret is only an alias.
    let headers = store.headers(&task_id).unwrap();
    assert_eq!(
        headers.get("Referer").map(String::as_str),
        Some("https://example.test/")
    );
    assert!(!headers.contains_key("Authorization"));
    let alias = store.secret_alias(&task_id).unwrap().expect("alias stored");
    let secret = secrets.lookup(&alias).unwrap().expect("secret in keyring");
    assert!(secret.contains("Bearer secret"));
    // The secret value never reaches the database file.
    let mut db = Vec::new();
    for suffix in ["", "-wal"] {
        if let Ok(bytes) = std::fs::read(dir.path().join(format!("segmenta.db{suffix}"))) {
            db.extend(bytes);
        }
    }
    let needle = b"Bearer secret";
    assert!(!db.windows(needle.len()).any(|window| window == needle));
}

#[tokio::test]
async fn system_status_reports_the_selected_backend() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let status = call(&h.manager, "system.status", json!({})).await.unwrap();
    assert_eq!(status["secret_backend"], "unavailable");
}

/// `system.open` must launch the GUI with a fixed argv and never through a shell.
#[tokio::test]
async fn system_open_launches_the_gui_with_the_task_as_an_argument() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    h.store
        .insert_download(&record("focus-me", DownloadStatus::Pending))
        .unwrap();

    let args_file = dir.path().join("argv.txt");
    let gui = dir.path().join("segmenta-gui");
    std::fs::write(
        &gui,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            args_file.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&gui, std::fs::Permissions::from_mode(0o755)).unwrap();

    std::env::set_var("SEGMENTA_GUI", &gui);
    let result = call(&h.manager, "system.open", json!({ "task_id": "focus-me" })).await;
    std::env::remove_var("SEGMENTA_GUI");

    let value = result.expect("an existing task must open the GUI");
    assert_eq!(value["opened"], json!(true));
    assert_eq!(value["task_id"], json!("focus-me"));
    assert_eq!(value["gui_path"], json!(gui.to_string_lossy()));

    // The child is detached, so its argv is read with a bounded wait.
    let mut argv = String::new();
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(&args_file) {
            argv = text;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(argv, "--show-task\nfocus-me\n");
}

/// The GUI is never spawned for a task that does not exist.
#[tokio::test]
async fn system_open_rejects_an_unknown_task_before_touching_the_gui() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let error = call(&h.manager, "system.open", json!({ "task_id": "missing" }))
        .await
        .expect_err("an unknown task must fail");
    assert_eq!(error.0, 1001);
}
