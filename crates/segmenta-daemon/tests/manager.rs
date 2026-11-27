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

/// Serializes the tests that mutate `SEGMENTA_GUI`: the variable is process-global, so
/// two of them running at once could spawn the wrong binary or lose the file the other
/// one is waiting for.
static GUI_ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
        quarantine_path: None,
        postprocess_exit: None,
        postprocess_message: None,
        scheduled_at: None,
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

/// The GUI's retry button is `download.resume`: a failed task must be retryable, and
/// the old refusal (1003) left the user stuck with a dead row (0.2.4).
#[tokio::test]
async fn resume_retries_a_failed_task_and_clears_its_error() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let mut failed = record("failed-retry", DownloadStatus::Failed);
    failed.error_code = Some(1044);
    failed.error_message = Some("HTTP Error 403: Forbidden".to_string());
    h.store.insert_download(&failed).unwrap();
    let mut events = h.bus.subscribe();

    let reply = call(
        &h.manager,
        "download.resume",
        json!({ "task_id": "failed-retry" }),
    )
    .await
    .expect("a failed task must be retryable from the interface");
    assert!(
        reply.get("status").and_then(Value::as_str).is_some(),
        "resume did not answer a status: {reply:?}"
    );
    // The retry leaves the failed state and starts without the previous error.
    let mut saw_pending = false;
    while let Ok(event) = events.try_recv() {
        if event.data["status"] == "pending" {
            saw_pending = true;
        }
    }
    assert!(saw_pending, "the retry did not leave the failed state");
    let after = h.store.get("failed-retry").unwrap().unwrap();
    assert_ne!(after.status, DownloadStatus::Failed);
}

/// A failed multimedia task is relaunched with the plan it had, not rejected.
#[tokio::test]
async fn resume_relaunches_a_failed_media_task() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("yt-dlp");
    std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = std::fs::metadata(&fake).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    std::fs::set_permissions(&fake, permissions).unwrap();
    std::env::set_var("SEGMENTA_YT_DLP", &fake);

    let h = harness(dir.path());
    let mut failed = record("media-retry", DownloadStatus::Failed);
    failed.download_type = "youtube_video".to_string();
    failed.media_kind = Some("video".to_string());
    failed.media_format = Some("888p".to_string());
    failed.error_code = Some(1044);
    failed.error_message = Some("HTTP Error 403: Forbidden".to_string());
    h.store.insert_download(&failed).unwrap();

    let reply = call(
        &h.manager,
        "download.resume",
        json!({ "task_id": "media-retry" }),
    )
    .await
    .expect("a failed media task must be retryable");
    assert_eq!(reply["status"], "downloading", "reply: {reply:?}");
    std::env::remove_var("SEGMENTA_YT_DLP");
}

#[tokio::test]
async fn resume_rejects_completed_and_cancelled_without_changing_them() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    for (id, status) in [
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

/// S10: simultaneous HTTP tasks must share the configured global bandwidth budget.
#[tokio::test]
async fn global_speed_limit_is_shared_by_concurrent_http_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let body = vec![42u8; MB];
    let server = RangeServer::start(ServerConfig {
        body: body.clone(),
        ..ServerConfig::default()
    })
    .await;
    let rate = 256 * 1024;
    let settings = call(
        &h.manager,
        "settings.update",
        json!({
            "allow_local_downloads": true,
            "download_dir": dir.path().join("downloads"),
            "max_connections": 1,
            "global_speed_limit_bps": rate,
        }),
    )
    .await
    .unwrap();
    assert_eq!(settings["global_speed_limit_bps"], rate);

    // Starting the clock before either request makes setup overhead conservative:
    // it can hide an excess rate, but cannot create a false excess-rate failure.
    let start = std::time::Instant::now();
    let mut ids = Vec::new();
    for filename in ["first.bin", "second.bin"] {
        let added = call(
            &h.manager,
            "download.add",
            json!({ "url": server.url().to_string(), "filename": filename, "connections": 1 }),
        )
        .await
        .unwrap();
        ids.push(added["task_id"].as_str().unwrap().to_string());
    }
    assert_ne!(ids[0], ids[1]);
    for id in &ids {
        let status = call(&h.manager, "download.status", json!({ "task_id": id }))
            .await
            .unwrap();
        assert_eq!(status["status"], "downloading", "task {id}: {status}");
    }

    let elapsed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let mut completed = 0;
            for id in &ids {
                let status = call(&h.manager, "download.status", json!({ "task_id": id }))
                    .await
                    .unwrap();
                assert_ne!(status["status"], "failed", "task {id}: {status}");
                if status["status"] == "completed" {
                    completed += 1;
                }
            }
            if completed == ids.len() {
                break start.elapsed();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("both HTTP transfers must complete within 30 seconds");
    for filename in ["first.bin", "second.bin"] {
        assert_eq!(
            std::fs::read(dir.path().join("downloads").join(filename)).unwrap(),
            body
        );
    }
    server.shutdown().await;

    // Two MiB at 256 KiB/s needs eight seconds, less the bucket's quarter-second
    // initial burst. A full second of slack also covers polling and clock granularity.
    assert!(
        elapsed >= Duration::from_secs(7),
        "two concurrent 1 MiB transfers exceeded the shared 256 KiB/s limit: {elapsed:?}"
    );
}

/// An active HTTP transfer must observe a change to unlimited without restarting.
#[tokio::test]
async fn global_speed_limit_hot_update_to_unlimited() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let body = vec![42u8; MB];
    let server = RangeServer::start(ServerConfig {
        body: body.clone(),
        ..ServerConfig::default()
    })
    .await;
    call(
        &h.manager,
        "settings.update",
        json!({
            "allow_local_downloads": true,
            "download_dir": dir.path().join("downloads"),
            "max_connections": 1,
            "global_speed_limit_bps": 64 * 1024,
        }),
    )
    .await
    .unwrap();
    let added = call(
        &h.manager,
        "download.add",
        json!({
            "url": server.url().to_string(), "filename": "hot.bin", "connections": 1
        }),
    )
    .await
    .unwrap();
    let id = added["task_id"].as_str().unwrap();
    // Wait long enough for the worker to enter the original limited transfer.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let status = call(&h.manager, "download.status", json!({"task_id": id}))
        .await
        .unwrap();
    assert_eq!(status["status"], "downloading", "{status}");
    call(
        &h.manager,
        "settings.update",
        json!({"global_speed_limit_bps": null}),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let status = call(&h.manager, "download.status", json!({"task_id": id}))
                .await
                .unwrap();
            assert_ne!(status["status"], "failed", "{status}");
            if status["status"] == "completed" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("active transfer must observe unlimited setting");
    assert_eq!(
        std::fs::read(dir.path().join("downloads/hot.bin")).unwrap(),
        body
    );
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

    let _env = GUI_ENV.lock().await;
    std::env::set_var("SEGMENTA_GUI", &gui);
    let result = call(&h.manager, "system.open", json!({ "task_id": "focus-me" })).await;
    std::env::remove_var("SEGMENTA_GUI");

    let value = result.expect("an existing task must open the GUI");
    assert_eq!(value["opened"], json!(true));
    assert_eq!(value["task_id"], json!("focus-me"));
    assert_eq!(value["gui_path"], json!(gui.to_string_lossy()));

    // The child is detached and the whole workspace runs in parallel, so its argv is
    // read with a generous bounded wait and one retry: a starved or lost spawn must not
    // turn a deterministic assertion into a flaky one.
    let wait_args = |file: std::path::PathBuf| async move {
        for _ in 0..400 {
            if let Ok(text) = std::fs::read_to_string(&file) {
                if !text.is_empty() {
                    return text;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        String::new()
    };
    let mut argv = wait_args(args_file.clone()).await;
    if argv.is_empty() {
        let _ = call(&h.manager, "system.open", json!({ "task_id": "focus-me" })).await;
        argv = wait_args(args_file.clone()).await;
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

/// S11: the `download.add` reply reports the directory the file is written into and
/// the resolved, de-duplicated path the task will produce.
#[tokio::test]
async fn download_add_reports_the_effective_destination_and_final_path() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let destination = dir.path().join("capturas");
    std::fs::create_dir_all(&destination).unwrap();
    // The suggested name is already taken, so the effective path must be de-duplicated.
    std::fs::write(destination.join("manual.deb"), b"already here").unwrap();
    let mut events = h.bus.subscribe();

    let added = call(
        &h.manager,
        "download.add",
        json!({
            "url": "https://example.test/manual.deb",
            "destination_dir": destination.display().to_string(),
            "filename": "manual.deb",
            "start_immediately": false,
        }),
    )
    .await
    .unwrap();

    let task_id = added["task_id"].as_str().unwrap().to_string();
    assert_eq!(
        added["destination_dir"],
        json!(destination.display().to_string())
    );
    assert_eq!(
        added["final_path"],
        json!(destination.join("manual (1).deb").display().to_string())
    );
    assert_eq!(
        h.store.get(&task_id).unwrap().unwrap().final_path,
        destination.join("manual (1).deb").display().to_string()
    );

    // The event carries the same pair, so a client never re-resolves the path itself.
    let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
        .await
        .expect("task_added must be published")
        .unwrap();
    assert_eq!(event.event_type, "task_added");
    assert_eq!(event.task_id.as_deref(), Some(task_id.as_str()));
    assert_eq!(
        event.data["destination_dir"],
        json!(destination.display().to_string())
    );
    assert_eq!(
        event.data["final_path"],
        json!(destination.join("manual (1).deb").display().to_string())
    );
}

/// S11: `capture_confirm` defaults to true, is writable and the extension list is widened.
#[tokio::test]
async fn settings_expose_capture_confirm_and_the_widened_extension_list() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());

    let defaults = call(&h.manager, "settings.get", json!({})).await.unwrap();
    assert_eq!(defaults["capture_confirm"], json!(true));
    let extensions = defaults["capture_extensions"].as_array().unwrap();
    for token in [
        "deb", "rpm", "appimage", "xz", "whl", "jar", "epub", "vsix", "xpi", "msix",
    ] {
        assert!(
            extensions.iter().any(|value| value == token),
            "missing capture extension {token}"
        );
    }
    // The previous defaults are kept.
    for token in ["zip", "rar", "iso", "pdf"] {
        assert!(
            extensions.iter().any(|value| value == token),
            "lost capture extension {token}"
        );
    }

    let updated = call(
        &h.manager,
        "settings.update",
        json!({ "capture_confirm": false }),
    )
    .await
    .unwrap();
    assert_eq!(updated["capture_confirm"], json!(false));
    let reread = call(&h.manager, "settings.get", json!({})).await.unwrap();
    assert_eq!(reread["capture_confirm"], json!(false));

    // Unknown keys and wrong types stay rejected.
    for bad in [
        json!({ "capture_confirm_typo": true }),
        json!({ "capture_confirm": "yes" }),
    ] {
        let error = call(&h.manager, "settings.update", bad.clone())
            .await
            .expect_err("must reject");
        assert_eq!(error.0, -32602, "patch {bad}: {error:?}");
    }
    assert_eq!(
        call(&h.manager, "settings.get", json!({})).await.unwrap()["capture_confirm"],
        json!(false)
    );
}

#[tokio::test]
async fn torrent_task_progress_reports_real_speed_and_finished_reports_zero() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let task_id = "torrent-speed-test";
    let mut rec = record(task_id, DownloadStatus::Downloading);
    rec.download_type = "torrent".to_string();
    rec.total_bytes = Some(10_000_000);
    h.store.insert_download(&rec).unwrap();

    let mut events = h.bus.subscribe();
    let mut pipeline = h.manager.torrent_progress_pipeline(task_id);

    // Initial sample
    pipeline.report(1_000_000, Some(10_000_000));
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Second sample over known interval
    pipeline.report(4_000_000, Some(10_000_000));

    let mut saw_speed = 0u64;
    for _ in 0..10 {
        if let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(250), events.recv()).await
        {
            if event.event_type == "task_progress" && event.task_id.as_deref() == Some(task_id) {
                if let Some(speed) = event.data["speed_bps"].as_u64() {
                    if speed > 0 {
                        saw_speed = speed;
                        break;
                    }
                }
            }
        }
    }
    assert!(
        saw_speed > 0,
        "torrent progress never reported a non-zero speed"
    );

    // Finish the task
    pipeline.finish();
    h.store
        .set_status(task_id, DownloadStatus::Completed, None)
        .unwrap();

    let status = call(&h.manager, "torrent.status", json!({ "task_id": task_id }))
        .await
        .unwrap();
    assert_eq!(status["status"], "completed");
    assert_eq!(
        status["speed_bps"], 0,
        "finished torrent speed must be zero"
    );
}

#[tokio::test]
async fn stream_task_progress_reports_real_speed_and_finished_reports_zero() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let task_id = "stream-speed-test";
    let mut rec = record(task_id, DownloadStatus::Downloading);
    rec.download_type = "hls".to_string();
    rec.media_kind = Some("stream".to_string());
    h.store.insert_download(&rec).unwrap();

    let mut events = h.bus.subscribe();
    let mut pipeline = h.manager.stream_progress_pipeline(task_id);

    // Initial sample
    pipeline.report(500_000, None);
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Second sample over known interval (> 250ms throttle)
    pipeline.report(2_000_000, None);

    let mut saw_speed = 0u64;
    for _ in 0..10 {
        if let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(250), events.recv()).await
        {
            if event.event_type == "task_progress" && event.task_id.as_deref() == Some(task_id) {
                if let Some(speed) = event.data["speed_bps"].as_u64() {
                    if speed > 0 {
                        saw_speed = speed;
                        break;
                    }
                }
            }
        }
    }
    assert!(
        saw_speed > 0,
        "stream progress never reported a non-zero speed"
    );

    // Finish the task
    pipeline.finish();
    h.store
        .set_status(task_id, DownloadStatus::Completed, None)
        .unwrap();

    let status = call(&h.manager, "download.status", json!({ "task_id": task_id }))
        .await
        .unwrap();
    assert_eq!(status["status"], "completed");
    assert_eq!(status["speed_bps"], 0, "finished stream speed must be zero");
}

#[tokio::test]
async fn invalid_scheduled_at_is_rejected_with_1004() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());

    // download.add with invalid formats
    for invalid in [
        "not-a-date",
        "25:00",
        "2026-11-16",
        "2026-11-16T21:30:00",
        "2026-13-01T10:00:00Z",
    ] {
        let err = call(
            &h.manager,
            "download.add",
            json!({
                "url": "https://example.test/file.bin",
                "scheduled_at": invalid,
            }),
        )
        .await
        .expect_err(&format!("scheduled_at `{invalid}` must be rejected"));
        assert_eq!(err.0, 1004, "code for `{invalid}`: {err:?}");
    }

    // media.add with invalid format
    let err = call(
        &h.manager,
        "media.add",
        json!({
            "url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "mode": "video",
            "scheduled_at": "bad-time",
        }),
    )
    .await
    .expect_err("media.add with bad scheduled_at must be rejected");
    assert_eq!(err.0, 1004, "{err:?}");
}

#[tokio::test]
async fn task_scheduled_in_future_is_not_started_by_queue() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());

    let added = call(
        &h.manager,
        "download.add",
        json!({
            "url": "https://example.test/file.bin",
            "scheduled_at": "2099-01-01T00:00:00Z",
            "start_immediately": true,
        }),
    )
    .await
    .unwrap();
    let task_id = added["task_id"].as_str().unwrap();

    let status = call(&h.manager, "download.status", json!({ "task_id": task_id }))
        .await
        .unwrap();
    assert_eq!(status["status"], "pending");
    assert_eq!(status["scheduled_at"], "2099-01-01T00:00:00Z");

    let started = h.manager.start_pending_for_queue("main").unwrap();
    assert_eq!(started, 0, "future scheduled task must not start");

    let status_after = call(&h.manager, "download.status", json!({ "task_id": task_id }))
        .await
        .unwrap();
    assert_eq!(status_after["status"], "pending");
}

#[tokio::test]
async fn after_deadline_passes_scheduled_task_starts() {
    let server = RangeServer::start(ServerConfig {
        body: vec![b'x'; 1024 * 64],
        ..ServerConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
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

    let added = call(
        &h.manager,
        "download.add",
        json!({
            "url": server.url().to_string(),
            "scheduled_at": "2000-01-01T00:00:00Z",
            "start_immediately": false,
        }),
    )
    .await
    .unwrap();
    let task_id = added["task_id"].as_str().unwrap().to_string();

    let started = h.manager.start_pending_for_queue("main").unwrap();
    assert_eq!(started, 1, "task with past deadline must start");

    let status = call(&h.manager, "download.status", json!({ "task_id": task_id }))
        .await
        .unwrap();
    assert_eq!(status["status"], "downloading");
    server.shutdown().await;
}

#[tokio::test]
async fn resume_on_not_due_task_leaves_it_pending() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());

    let added = call(
        &h.manager,
        "download.add",
        json!({
            "url": "https://example.test/file.bin",
            "scheduled_at": "2099-01-01T00:00:00Z",
        }),
    )
    .await
    .unwrap();
    let task_id = added["task_id"].as_str().unwrap();

    let resumed = call(&h.manager, "download.resume", json!({ "task_id": task_id }))
        .await
        .unwrap();
    assert_eq!(resumed["status"], "pending");

    let status = call(&h.manager, "download.status", json!({ "task_id": task_id }))
        .await
        .unwrap();
    assert_eq!(status["status"], "pending");
}

#[tokio::test]
async fn scheduled_at_with_hh_mm_normalizes_to_utc_rfc3339() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());

    let added = call(
        &h.manager,
        "download.add",
        json!({
            "url": "https://example.test/file.bin",
            "scheduled_at": "23:59",
        }),
    )
    .await
    .unwrap();
    let task_id = added["task_id"].as_str().unwrap();

    let status = call(&h.manager, "download.status", json!({ "task_id": task_id }))
        .await
        .unwrap();
    let scheduled_at = status["scheduled_at"]
        .as_str()
        .expect("normalized scheduled_at");
    assert!(
        scheduled_at.ends_with('Z'),
        "must be UTC Z format: {scheduled_at}"
    );
    assert_eq!(
        scheduled_at.len(),
        20,
        "RFC-3339 format YYYY-MM-DDTHH:MM:SSZ: {scheduled_at}"
    );
}

#[tokio::test]
async fn manager_host_limits_shared_and_defaults_for_unseen_host() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());

    let limits_a = h.manager.host_limits();
    let limits_b = h.manager.host_limits();
    assert!(
        Arc::ptr_eq(&limits_a, &limits_b),
        "host_limits() must return the same Arc instance across calls"
    );
    assert_eq!(limits_a.limit("unseen-host.example"), 8);
    assert_eq!(limits_a.tracked(), 0);

    // Triangulate: mutating through limits_a affects limits_b and later manager.host_limits() calls
    limits_a.on_throttled("throttled.example");
    assert_eq!(limits_b.limit("throttled.example"), 4);
    assert_eq!(h.manager.host_limits().limit("throttled.example"), 4);
    assert_eq!(h.manager.host_limits().tracked(), 1);
    // Unseen host still yields default 8
    assert_eq!(h.manager.host_limits().limit("other.example"), 8);
}

#[tokio::test]
async fn download_path_passes_manager_host_limits_to_engine() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = RangeServer::start(ServerConfig {
        force_status: Some(429),
        retry_after: Some("60".to_string()),
        ..ServerConfig::default()
    })
    .await;

    let mut settings = Settings {
        allow_local_downloads: true,
        max_retries: 0,
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

    let url = server.url();
    let host = url.host_str().unwrap();
    assert_eq!(h.manager.host_limits().limit(host), 8);
    assert_eq!(h.manager.host_limits().tracked(), 0);

    let added = call(
        &h.manager,
        "download.add",
        json!({ "url": server.url().to_string() }),
    )
    .await
    .unwrap();
    let task_id = added["task_id"].as_str().unwrap();

    for _ in 0..100 {
        let status = h.store.get(task_id).unwrap().unwrap().status;
        if status == DownloadStatus::Failed || status == DownloadStatus::Completed {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(
        h.manager.host_limits().limit(host),
        4,
        "server 429 must throttle the host in the manager's shared registry"
    );
    assert_eq!(h.manager.host_limits().tracked(), 1);
    // Other hosts are unaffected by this task's throttle
    assert_eq!(h.manager.host_limits().limit("unrelated.example"), 8);
    server.shutdown().await;
}
