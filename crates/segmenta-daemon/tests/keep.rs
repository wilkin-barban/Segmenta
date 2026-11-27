//! `download.keep`: a stream with no declared length is published with what it wrote
//! instead of being thrown away (T7).

use std::path::Path;
use std::sync::Arc;

use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::RpcHandler;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::secrets::FileSecretStore;
use segmenta_daemon::store::{DownloadRecord, DownloadStatus, Store};
use serde_json::{json, Value};
use tokio::sync::watch;

fn harness(dir: &Path) -> (Arc<Manager>, Arc<Store>) {
    let store = Arc::new(Store::open(&dir.join("segmenta.db")).unwrap());
    let secrets: Arc<dyn segmenta_daemon::secrets::SecretStore> =
        Arc::new(FileSecretStore::open(&dir.join("secrets.json")).unwrap());
    let bus = Arc::new(EventBus::new());
    let (shutdown, _rx) = watch::channel(false);
    let manager = Arc::new(Manager::new(Arc::clone(&store), secrets, bus, shutdown));
    (manager, store)
}

/// A task row whose file already holds bytes on disk.
fn partial(id: &str, path: &Path, status: DownloadStatus) -> DownloadRecord {
    DownloadRecord {
        id: id.to_string(),
        url: "http://example.test/stream".to_string(),
        final_url: None,
        final_path: path.to_string_lossy().to_string(),
        temp_path: None,
        total_bytes: None,
        downloaded_bytes: 0,
        status,
        supports_ranges: false,
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
        filename: path.file_name().unwrap().to_string_lossy().to_string(),
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

async fn call(manager: &Arc<Manager>, method: &str, params: Value) -> Result<Value, (i64, String)> {
    manager
        .call(method, Some(params))
        .await
        .map_err(|err| (err.code, err.message))
}

#[tokio::test]
async fn a_partial_stream_is_kept_with_the_bytes_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, store) = harness(dir.path());
    // A task that already wrote 11 bytes and then paused: this is the case that used to
    // end with the file discarded, because a stream with no length cannot be resumed.
    let path = dir.path().join("stream.bin");
    std::fs::write(&path, b"hello world").unwrap();
    let record = partial("keep-me", &path, DownloadStatus::Paused);
    store.insert_download(&record).unwrap();

    let kept = call(&manager, "download.keep", json!({ "task_id": "keep-me" }))
        .await
        .unwrap();
    assert_eq!(kept["status"], "completed", "{kept}");
    assert_eq!(kept["downloaded_bytes"], 11);

    let stored = store.get("keep-me").unwrap().unwrap();
    assert_eq!(stored.status, DownloadStatus::Completed);
    assert_eq!(stored.downloaded_bytes, 11);
    assert_eq!(stored.total_bytes, Some(11));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"hello world",
        "the bytes stay on disk"
    );
}

#[tokio::test]
async fn keeping_an_empty_or_running_task_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, store) = harness(dir.path());
    let empty = dir.path().join("empty.bin");
    std::fs::write(&empty, b"").unwrap();
    store
        .insert_download(&partial("empty", &empty, DownloadStatus::Paused))
        .unwrap();

    let refused = call(&manager, "download.keep", json!({ "task_id": "empty" }))
        .await
        .unwrap_err();
    assert_eq!(refused.0, 1086, "nothing on disk: {refused:?}");

    // A task that is still downloading must not be published behind the engine's back.
    let running = dir.path().join("running.bin");
    std::fs::write(&running, b"partial").unwrap();
    store
        .insert_download(&partial("running", &running, DownloadStatus::Downloading))
        .unwrap();
    let refused = call(&manager, "download.keep", json!({ "task_id": "running" }))
        .await
        .unwrap_err();
    assert_eq!(refused.0, 1084, "still running: {refused:?}");
}
