//! Safe download housekeeping: `download.clean_completed` and `download.delete_many`.
//!
//! The contract under test is the one the GUI confirmation dialog relies on:
//! - cleaning terminadas removes rows only and never touches a downloaded file;
//! - a batch delete that does not ask for the files keeps them;
//! - a batch delete with files removes them after the row work and reports counts;
//! - a file that cannot be removed keeps the row in a visible failure state;
//! - an active task is stopped before its file is touched;
//! - a duplicate, an unknown id or an empty batch never aborts the whole request.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use segmenta_core::testserver::{RangeServer, ServerConfig};
use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::RpcHandler;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::secrets::{open_with, SecretStore};
use segmenta_daemon::store::{DownloadRecord, DownloadStatus, Settings, Store};
use serde_json::{json, Value};
use tokio::sync::watch;

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

/// A stored task pointing at `path`.
fn record_at(id: &str, status: DownloadStatus, path: &Path) -> DownloadRecord {
    DownloadRecord {
        id: id.to_string(),
        url: "http://127.0.0.1:1/file.bin".to_string(),
        final_url: None,
        final_path: path.display().to_string(),
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
        filename: "file.bin".to_string(),
        media_kind: None,
        media_format: None,
        playlist_title: None,
        playlist_count: None,
        batch_id: None,
        quarantine_path: None,
        postprocess_exit: None,
        postprocess_message: None,
    }
}

fn results_of(reply: &Value) -> &Vec<Value> {
    reply["results"].as_array().expect("results array")
}

fn result_for<'a>(reply: &'a Value, task_id: &str) -> &'a Value {
    results_of(reply)
        .iter()
        .find(|entry| entry["task_id"] == json!(task_id))
        .unwrap_or_else(|| panic!("no result for `{task_id}` in {reply}"))
}

#[tokio::test]
async fn clean_completed_removes_only_completed_rows_and_keeps_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let finished = dir.path().join("terminada.bin");
    std::fs::write(&finished, b"contenido descargado").unwrap();

    h.store
        .insert_download(&record_at("done", DownloadStatus::Completed, &finished))
        .unwrap();
    for (id, status) in [
        ("failed", DownloadStatus::Failed),
        ("paused", DownloadStatus::Paused),
        ("active", DownloadStatus::Downloading),
        ("pending", DownloadStatus::Pending),
        ("cancelled", DownloadStatus::Cancelled),
    ] {
        let path = dir.path().join(format!("{id}.bin"));
        std::fs::write(&path, b"x").unwrap();
        h.store
            .insert_download(&record_at(id, status, &path))
            .unwrap();
    }

    let reply = call(&h.manager, "download.clean_completed", json!({}))
        .await
        .unwrap();

    assert_eq!(reply["removed"], 1, "solo la tarea completed: {reply}");
    assert_eq!(
        reply["failed"].as_array().expect("failed array").len(),
        0,
        "{reply}"
    );
    assert!(h.store.get("done").unwrap().is_none(), "la fila se borró");
    for id in ["failed", "paused", "active", "pending", "cancelled"] {
        assert!(
            h.store.get(id).unwrap().is_some(),
            "`{id}` no es completed y debe sobrevivir"
        );
    }
    assert_eq!(
        std::fs::read(&finished).unwrap(),
        b"contenido descargado",
        "clean_completed nunca elimina el archivo descargado"
    );
}

#[tokio::test]
async fn clean_completed_with_nothing_finished_reports_zero() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let path = dir.path().join("fallida.bin");
    std::fs::write(&path, b"x").unwrap();
    h.store
        .insert_download(&record_at("failed", DownloadStatus::Failed, &path))
        .unwrap();

    let reply = call(&h.manager, "download.clean_completed", json!({}))
        .await
        .unwrap();
    assert_eq!(reply["removed"], 0, "{reply}");
    assert!(reply["failed"].as_array().expect("failed").is_empty());
    assert!(h.store.get("failed").unwrap().is_some());
    assert!(path.is_file());
}

#[tokio::test]
async fn delete_many_without_files_removes_the_rows_and_keeps_the_files() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let first = dir.path().join("uno.bin");
    let second = dir.path().join("dos.bin");
    std::fs::write(&first, b"uno").unwrap();
    std::fs::write(&second, b"dos").unwrap();
    h.store
        .insert_download(&record_at("uno", DownloadStatus::Completed, &first))
        .unwrap();
    h.store
        .insert_download(&record_at("dos", DownloadStatus::Paused, &second))
        .unwrap();

    // `delete_files` omitted: the documented default is `false`.
    let reply = call(
        &h.manager,
        "download.delete_many",
        json!({ "task_ids": ["uno", "dos"] }),
    )
    .await
    .unwrap();

    assert_eq!(reply["removed"], 2, "{reply}");
    assert_eq!(reply["files_deleted"], 0, "{reply}");
    assert!(reply["files_failed"].as_array().expect("failed").is_empty());
    for id in ["uno", "dos"] {
        assert!(result_for(&reply, id)["ok"].as_bool().unwrap(), "{reply}");
        assert!(
            h.store.get(id).unwrap().is_none(),
            "la fila `{id}` se borró"
        );
    }
    assert!(first.is_file(), "sin delete_files el archivo se conserva");
    assert!(second.is_file(), "sin delete_files el archivo se conserva");
}

#[tokio::test]
async fn delete_many_with_files_removes_them_and_reports_counts() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let first = dir.path().join("uno.bin");
    let second = dir.path().join("dos.bin");
    std::fs::write(&first, b"uno").unwrap();
    std::fs::write(&second, b"dos").unwrap();
    let missing = dir.path().join("ya-no-esta.bin");
    h.store
        .insert_download(&record_at("uno", DownloadStatus::Completed, &first))
        .unwrap();
    h.store
        .insert_download(&record_at("dos", DownloadStatus::Completed, &second))
        .unwrap();
    // A row whose file is already gone is not a failure: there is nothing to delete.
    h.store
        .insert_download(&record_at("gone", DownloadStatus::Completed, &missing))
        .unwrap();

    let reply = call(
        &h.manager,
        "download.delete_many",
        json!({ "task_ids": ["uno", "dos", "gone"], "delete_files": true }),
    )
    .await
    .unwrap();

    assert_eq!(reply["removed"], 3, "{reply}");
    assert_eq!(reply["files_deleted"], 2, "{reply}");
    assert!(reply["files_failed"].as_array().expect("failed").is_empty());
    for id in ["uno", "dos", "gone"] {
        assert!(result_for(&reply, id)["ok"].as_bool().unwrap(), "{reply}");
        assert!(h.store.get(id).unwrap().is_none());
    }
    assert!(!first.exists(), "el archivo de `uno` se eliminó");
    assert!(!second.exists(), "el archivo de `dos` se eliminó");
}

#[tokio::test]
async fn a_file_that_cannot_be_removed_is_reported_and_the_row_stays_visible() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    // A directory where the file should be: `remove_file` can never succeed.
    let blocked: PathBuf = dir.path().join("bloqueada.bin");
    std::fs::create_dir(&blocked).unwrap();
    h.store
        .insert_download(&record_at("bloqueada", DownloadStatus::Completed, &blocked))
        .unwrap();
    let fine = dir.path().join("fina.bin");
    std::fs::write(&fine, b"ok").unwrap();
    h.store
        .insert_download(&record_at("fina", DownloadStatus::Completed, &fine))
        .unwrap();

    let reply = call(
        &h.manager,
        "download.delete_many",
        json!({ "task_ids": ["bloqueada", "fina"], "delete_files": true }),
    )
    .await
    .unwrap();

    let blocked_result = result_for(&reply, "bloqueada");
    assert_eq!(blocked_result["ok"], json!(false), "{reply}");
    assert!(
        !blocked_result["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "el fallo lleva el error exacto: {reply}"
    );
    assert_eq!(reply["removed"], 1, "sólo `fina` se borró: {reply}");
    assert_eq!(reply["files_deleted"], 1, "{reply}");
    let failed = reply["files_failed"].as_array().expect("files_failed");
    assert_eq!(failed.len(), 1, "{reply}");
    assert_eq!(failed[0]["task_id"], json!("bloqueada"));
    assert!(failed[0]["path"]
        .as_str()
        .unwrap_or_default()
        .ends_with("bloqueada.bin"));

    let row = h
        .store
        .get("bloqueada")
        .unwrap()
        .expect("la fila no se borra cuando el archivo no se puede eliminar");
    assert_eq!(
        row.status,
        DownloadStatus::Failed,
        "estado visible de fallo"
    );
    assert!(
        row.error_message
            .as_deref()
            .unwrap_or_default()
            .contains("bloqueada.bin"),
        "el mensaje nombra el archivo: {:?}",
        row.error_message
    );
    assert!(blocked.is_dir(), "el path no se tocó");
    assert!(h.store.get("fina").unwrap().is_none());
    assert!(!fine.exists());
}

#[tokio::test]
async fn delete_many_stops_an_active_task_before_touching_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let server = RangeServer::start(ServerConfig {
        body: vec![9u8; 4 * 1024 * 1024],
        chunk_size: 8 * 1024,
        slow: Some(Duration::from_millis(4)),
        ..ServerConfig::default()
    })
    .await;

    let settings = Settings {
        allow_local_downloads: true,
        download_dir: dir.path().join("descargas").display().to_string(),
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

    // Wait until the engine is really writing: a progress event and the file on disk.
    let mut progress = false;
    for _ in 0..200 {
        match tokio::time::timeout(Duration::from_millis(250), events.recv()).await {
            Ok(Ok(event)) if event.event_type == "task_progress" => {
                progress = event.data["downloaded_bytes"].as_u64().unwrap_or(0) > 0;
                if progress {
                    break;
                }
            }
            _ => {}
        }
    }
    assert!(progress, "el motor nunca reportó progreso");
    let final_path = h.store.get(&task_id).unwrap().unwrap().final_path;
    let final_path = PathBuf::from(final_path);
    assert!(
        final_path.is_file(),
        "el motor ya escribe el archivo antes de borrarlo: {}",
        final_path.display()
    );
    assert_eq!(h.manager.running_count(), 1, "la tarea está activa");

    let reply = call(
        &h.manager,
        "download.delete_many",
        json!({ "task_ids": [task_id], "delete_files": true }),
    )
    .await
    .unwrap();
    let returned_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    assert!(
        result_for(&reply, &task_id)["ok"].as_bool().unwrap(),
        "{reply}"
    );
    assert_eq!(reply["removed"], 1, "{reply}");
    assert_eq!(reply["files_deleted"], 1, "{reply}");
    assert!(h.store.get(&task_id).unwrap().is_none(), "la fila se borró");
    assert!(
        !final_path.exists(),
        "el archivo se eliminó después de parar"
    );
    assert_eq!(
        h.manager.running_count(),
        0,
        "la tarea se detuvo antes de tocar el archivo"
    );

    // No engine event may arrive after the call returned: the writer was already gone.
    let mut late = Vec::new();
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(300), events.recv()).await
    {
        // `timestamp_ms` has millisecond granularity: an event published just before
        // the call returned can share that millisecond. Only a strictly later event
        // proves a writer survived the deletion.
        if event.timestamp_ms > returned_at && event.task_id.as_deref() == Some(&task_id) {
            late.push(event.event_type);
        }
    }
    assert!(
        late.is_empty(),
        "el motor seguía publicando eventos tras el borrado: {late:?}"
    );
    assert!(!final_path.exists(), "nada reescribió el archivo");
    server.shutdown().await;
}

#[tokio::test]
async fn delete_many_reports_unknown_and_duplicate_ids_without_aborting_the_batch() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let path = dir.path().join("buena.bin");
    std::fs::write(&path, b"x").unwrap();
    h.store
        .insert_download(&record_at("buena", DownloadStatus::Completed, &path))
        .unwrap();

    let reply = call(
        &h.manager,
        "download.delete_many",
        json!({ "task_ids": ["fantasma", "buena", "buena"], "delete_files": false }),
    )
    .await
    .unwrap();

    assert_eq!(reply["removed"], 1, "{reply}");
    assert_eq!(result_for(&reply, "fantasma")["ok"], json!(false));
    assert!(
        result_for(&reply, "fantasma")["error"]
            .as_str()
            .unwrap_or_default()
            .contains("fantasma"),
        "el error nombra el id desconocido: {reply}"
    );
    // Both occurrences of the duplicate are reported, and the batch keeps going.
    let duplicated: Vec<&Value> = results_of(&reply)
        .iter()
        .filter(|entry| entry["task_id"] == json!("buena") && entry["ok"] == json!(false))
        .collect();
    assert_eq!(duplicated.len(), 1, "el id repetido se reporta: {reply}");
    assert!(h.store.get("buena").unwrap().is_none());
    assert!(path.is_file(), "sin delete_files el archivo se conserva");
    assert_eq!(
        results_of(&reply).len(),
        3,
        "un resultado por id enviado: {reply}"
    );
}

#[tokio::test]
async fn delete_many_rejects_an_empty_batch() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(dir.path());
    let err = call(
        &h.manager,
        "download.delete_many",
        json!({ "task_ids": [], "delete_files": false }),
    )
    .await
    .expect_err("un lote vacío no es un éxito");
    assert_eq!(err.0, -32602, "{err:?}");
    assert!(
        err.1.contains("task_ids"),
        "el error explica qué falta: {err:?}"
    );
}
