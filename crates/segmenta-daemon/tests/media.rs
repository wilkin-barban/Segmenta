//! `media.probe`/`media.add` against a fake `yt-dlp` (S6-S8).
//!
//! The fake binary prints a fixed `-J` payload, records its argv and emits the progress
//! template lines, so the daemon side is exercised without the network.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::RpcHandler;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::store::{Settings, Store};
use serde_json::json;
use tokio::sync::watch;

fn fake_tool(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("yt-dlp");
    let body = r#"#!/bin/sh
printf '%s\n' "$@" > "$FAKE_ARGS"
case "$*" in
  *--playlist-end*)
    cat <<JSON
{"_type":"playlist","title":"Demo List","entries":[
  {"title":"One","duration":10,"webpage_url":"https://www.youtube.com/watch?v=one"},
  {"title":"Two","duration":20,"webpage_url":"https://www.youtube.com/watch?v=two"}]}
JSON
    exit 0 ;;
esac
for arg in "$@"; do
  if [ "$arg" = "-J" ]; then
    cat <<JSON
{"title":"Demo / Video","duration":213,"thumbnail":"https://i.ytimg.com/x.jpg",
 "formats":[{"format_id":"137","height":1080,"ext":"mp4","vcodec":"avc1","acodec":"none"},
            {"format_id":"22","height":720,"ext":"mp4","vcodec":"avc1","acodec":"mp4a"},
            {"format_id":"140","ext":"m4a","vcodec":"none","acodec":"mp4a"}]}
JSON
    exit 0
  fi
done
printf 'download:1024|4096|512000|6\n' >&2
printf 'download:4096|4096|512000|0\n' >&2
out=""
while [ $# -gt 0 ]; do
  case "$1" in -o) out="$2"; shift 2 ;; *) shift ;; esac
done
if [ -n "$out" ]; then
  case "$out" in *%*) mkdir -p "$out" ;; *) mkdir -p "$(dirname "$out")"; : > "$out" ;; esac
fi
exit 0
"#;
    std::fs::write(&path, body).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

#[tokio::test]
async fn probe_add_playlist_and_private_media() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(dir.path());
    let args_file = dir.path().join("args.txt");
    // One test body: the tool override is process-wide.
    std::env::set_var("SEGMENTA_YT_DLP", &tool);
    std::env::set_var("FAKE_ARGS", &args_file);

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

    let probe = manager
        .call(
            "media.probe",
            Some(json!({"url":"https://www.youtube.com/watch?v=abc"})),
        )
        .await
        .expect("probe");
    assert_eq!(probe["title"], "Demo / Video");
    assert_eq!(probe["formats"].as_array().unwrap().len(), 2);
    assert_eq!(probe["formats"][0]["height"], 1080);
    assert_eq!(probe["audio"][0]["label"], "m4a");

    let added = manager
        .call(
            "media.add",
            Some(json!({"url":"https://www.youtube.com/watch?v=abc","mode":"video","height":720})),
        )
        .await
        .expect("add");
    let task_id = added["task_id"].as_str().unwrap().to_string();
    assert_eq!(added["kind"], "youtube_video");
    let mut status = String::new();
    for _ in 0..60 {
        let value = manager
            .call("media.status", Some(json!({"task_id": task_id})))
            .await
            .expect("status");
        status = value["status"].as_str().unwrap_or_default().to_string();
        if status == "completed" || status == "failed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(status, "completed", "{status}");
    let record = store.get(&task_id).unwrap().unwrap();
    assert_eq!(record.download_type, "youtube_video");
    assert_eq!(record.media_kind.as_deref(), Some("video"));
    let file = Path::new(&record.final_path);
    assert!(file.exists(), "{} missing", file.display());
    assert!(file
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with(".mp4"));

    // Live QA: a second download of the same title must not resume into the first
    // file (that produced HTTP 416). Each task owns a fresh, unique path.
    let again = manager
        .call(
            "media.add",
            Some(json!({"url":"https://www.youtube.com/watch?v=abc","mode":"video","height":720})),
        )
        .await
        .expect("second add");
    let second_id = again["task_id"].as_str().unwrap().to_string();
    for _ in 0..60 {
        let value = manager
            .call("media.status", Some(json!({"task_id": second_id})))
            .await
            .expect("status");
        if matches!(value["status"].as_str(), Some("completed") | Some("failed")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let second = store.get(&second_id).unwrap().unwrap();
    assert_ne!(second.final_path, record.final_path);
    assert_eq!(
        second.status,
        segmenta_daemon::store::DownloadStatus::Completed
    );
    assert!(Path::new(&second.final_path).exists());

    // Probing a playlist must stop at the requested bound: a full `-J` walk aborts
    // when a later entry is private (the PL590/PLbpi failures in live QA).
    let probed_playlist = manager
        .call(
            "media.probe",
            Some(json!({"url":"https://www.youtube.com/playlist?list=PL1","kind":"playlist","limit":2})),
        )
        .await
        .expect("playlist probe");
    let probe_args = std::fs::read_to_string(&args_file).unwrap_or_default();
    assert!(
        probe_args.contains("--playlist-end\n2"),
        "argv: {probe_args} json: {probed_playlist}"
    );
    assert_eq!(
        probed_playlist["playlist"]["count"], 2,
        "json: {probed_playlist}"
    );

    let added = manager
        .call(
            "media.add",
            Some(json!({"url":"https://www.youtube.com/playlist?list=PL1","mode":"playlist","playlist_limit":5})),
        )
        .await
        .expect("playlist add");
    assert_eq!(added["kind"], "youtube_playlist");
    assert!(added["batch_id"].is_string());
    // The download runs in the background: wait until the fake tool recorded its argv.
    let mut args = String::new();
    for _ in 0..40 {
        args = std::fs::read_to_string(&args_file).unwrap_or_default();
        if args.contains("--playlist-end") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(args.contains("--playlist-end\n5"), "{args}");

    std::fs::write(
        &tool,
        "#!/bin/sh\necho 'ERROR: Private video. Sign in if you have been granted access' >&2\nexit 1\n",
    )
    .unwrap();
    let err = manager
        .call(
            "media.probe",
            Some(json!({"url":"https://www.youtube.com/watch?v=priv"})),
        )
        .await
        .expect_err("private");
    assert_eq!(err.code, 1045);
    std::env::remove_var("SEGMENTA_YT_DLP");
    std::env::remove_var("FAKE_ARGS");
}
