//! Torrent tests (`torrent.probe`, `torrent.add`, `torrent.status`, path hostility).

use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::RpcHandler;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::store::{Settings, Store};
use segmenta_daemon::torrent::sanitize_torrent_path;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::watch;

#[test]
fn hostile_torrent_paths_are_rejected() {
    let base = Path::new("/tmp/downloads");
    assert!(sanitize_torrent_path(base, "../../etc/passwd").is_err());
    assert!(sanitize_torrent_path(base, "/etc/passwd").is_err());
    assert!(sanitize_torrent_path(base, "foo/../../etc/passwd").is_err());
    assert!(sanitize_torrent_path(base, "subdir/movie.mp4").is_ok());
}

#[tokio::test]
async fn torrent_rpc_surface_magnet() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("segmenta.db")).unwrap());
    let settings = Settings {
        download_dir: dir.path().join("downloads").display().to_string(),
        ..Settings::default()
    };
    store
        .update_settings(
            serde_json::to_value(&settings)
                .unwrap()
                .as_object()
                .unwrap(),
        )
        .unwrap();
    let secret_store: Arc<dyn segmenta_daemon::SECRET_STORE_TRAIT> =
        Arc::from(segmenta_daemon::open_secret_store(dir.path(), None).unwrap());
    let bus = Arc::new(EventBus::new());
    let (shutdown, _rx) = watch::channel(false);
    let manager = Arc::new(Manager::new(
        Arc::clone(&store),
        secret_store,
        Arc::clone(&bus),
        shutdown,
    ));

    let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Ubuntu+ISO";
    let probe = manager
        .call("torrent.probe", Some(json!({"url": magnet})))
        .await
        .expect("torrent probe");
    assert_eq!(probe["name"], "Ubuntu ISO");
    assert_eq!(probe["source_type"], "magnet");

    let added = manager
        .call("torrent.add", Some(json!({"url": magnet})))
        .await
        .expect("torrent add");
    let task_id = added["task_id"].as_str().unwrap();
    assert_eq!(added["status"], "pending");

    let status = manager
        .call("torrent.status", Some(json!({"task_id": task_id})))
        .await
        .expect("torrent status");
    assert_eq!(status["status"], "pending");
    assert_eq!(status["files_count"], 1);
    assert_eq!(status["files"].as_array().unwrap().len(), 1);
}
