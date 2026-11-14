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
 "formats":[{"format_id":"137","height":1080,"ext":"mp4","vcodec":"avc1","acodec":"none","tbr":4500,"protocol":"https"},
            {"format_id":"248","height":1080,"ext":"webm","vcodec":"vp9","acodec":"none","tbr":3000,"protocol":"https"},
            {"format_id":"22","height":720,"ext":"mp4","vcodec":"avc1","acodec":"mp4a","tbr":1500,"protocol":"https"},
            {"format_id":"18b","height":720,"ext":"mp4","vcodec":"avc1","acodec":"none","tbr":900,"protocol":"https"},
            {"format_id":"135","height":480,"ext":"mp4","vcodec":"avc1","acodec":"none","tbr":700,"protocol":"https"},
            {"format_id":"134","height":360,"ext":"mp4","vcodec":"avc1","acodec":"none","tbr":500,"protocol":"https"},
            {"format_id":"160","height":240,"ext":"mp4","vcodec":"avc1","acodec":"none","tbr":200,"protocol":"https"},
            {"format_id":"140","ext":"m4a","vcodec":"none","acodec":"mp4a","abr":128},
            {"format_id":"251","ext":"webm","vcodec":"none","acodec":"opus","abr":160}]}
JSON
    exit 0
  fi
done
printf '1024|4096|512000|6\n' >&2
printf '4096|4096|512000|0\n' >&2
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
    // Formats are deduplicated by height: 1080 and 720 each arrive twice in the
    // fixture and only one candidate per height survives.
    assert_eq!(probe["formats"].as_array().unwrap().len(), 5);
    assert_eq!(probe["formats"][0]["height"], 1080);
    // Same height, no muxed candidate: the higher bitrate one wins.
    assert_eq!(probe["formats"][0]["format_id"], "137");
    // Muxed beats DASH video-only for an equal height (S9).
    assert_eq!(probe["formats"][1]["height"], 720);
    assert_eq!(probe["formats"][1]["format_id"], "22");
    // The ladder keeps only 2160/1080/720/360 plus audio, in that order.
    let qualities: Vec<&str> = probe["qualities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["quality"].as_str().unwrap())
        .collect();
    assert_eq!(qualities, ["1080", "720", "360", "audio"]);
    assert_eq!(probe["qualities"][1]["label"], "720p");
    assert_eq!(probe["qualities"][1]["format_id"], "22");
    assert_eq!(probe["default_quality"], "720");
    // Audio is ordered by bitrate, not by its label.
    assert_eq!(probe["audio"][0]["format_id"], "251");
    assert_eq!(probe["audio"][1]["format_id"], "140");

    let added = manager
        .call(
            "media.add",
            // A raw `format_id` is sent on purpose: video adds must select by height
            // and never hand this id to the backend (S9).
            Some(json!({"url":"https://www.youtube.com/watch?v=abc","mode":"video","height":720,"format_id":"137"})),
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

    // The height reached the backend: `-f` carries the muxed height selector and the
    // raw format id never appears in the argument vector.
    let video_args = std::fs::read_to_string(&args_file).unwrap_or_default();
    assert!(
        video_args.contains("-f\nb[height<=720]"),
        "argv: {video_args}"
    );
    assert!(!video_args.contains("137"), "argv: {video_args}");
    assert_eq!(record.media_format.as_deref(), Some("720p"));

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

/// Minimal fake `ffmpeg` for the manifest pipeline: the merge step only has to
/// succeed, no bytes are inspected.
fn fake_ffmpeg(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("ffmpeg");
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

/// Static HLS fixture: one variant per height, all sharing a single segment.
fn write_hls_fixture(dir: &Path) {
    std::fs::write(
        dir.join("master.m3u8"),
        "#EXTM3U\n\
         #EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\nv1080.m3u8\n\
         #EXT-X-STREAM-INF:BANDWIDTH=2800000,RESOLUTION=1280x720\nv720.m3u8\n\
         #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nv360.m3u8\n",
    )
    .unwrap();
    for variant in ["v1080.m3u8", "v720.m3u8", "v360.m3u8"] {
        std::fs::write(
            dir.join(variant),
            "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1.0,\nseg1.ts\n#EXT-X-ENDLIST\n",
        )
        .unwrap();
    }
    std::fs::write(dir.join("seg1.ts"), b"segment-payload").unwrap();
}

/// Serve the fixture over loopback and record every requested path.
async fn serve_fixture(
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    dir: std::path::PathBuf,
) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let dir = dir.clone();
            let requests = std::sync::Arc::clone(&requests);
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buffer = [0u8; 2048];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let path = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .trim_start_matches('/')
                    .split('?')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                requests.lock().unwrap().push(path.clone());
                let body = tokio::fs::read(dir.join(&path)).await.unwrap_or_default();
                let status = if body.is_empty() {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(&body).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (port, handle)
}

async fn wait_for_terminal(manager: &Manager, task_id: &str) -> String {
    let mut status = String::new();
    for _ in 0..100 {
        let value = manager
            .call("media.status", Some(json!({"task_id": task_id})))
            .await
            .expect("status");
        status = value["status"].as_str().unwrap_or_default().to_string();
        if status == "completed" || status == "failed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    status
}

#[tokio::test]
async fn add_stream_task_selects_the_requested_height() {
    let dir = tempfile::tempdir().unwrap();
    let manifests = dir.path().join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    write_hls_fixture(&manifests);
    let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (port, server) = serve_fixture(std::sync::Arc::clone(&requests), manifests.clone()).await;
    let url = format!("http://127.0.0.1:{port}/master.m3u8");

    // Tool overrides are process wide, like the fake `yt-dlp` above.
    std::env::set_var("SEGMENTA_FFMPEG", fake_ffmpeg(dir.path()));

    let store = Arc::new(Store::open(&dir.path().join("segmenta.db")).unwrap());
    let settings = Settings {
        download_dir: dir.path().join("downloads").display().to_string(),
        allow_local_downloads: true,
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
        bus,
        shutdown,
    ));

    // The requested height reaches the variant selection: 360 must win over the
    // best (1080) variant the manifest also offers.
    let added = manager
        .call(
            "media.add",
            Some(json!({ "url": url, "mode": "video", "height": 360 })),
        )
        .await
        .expect("add 360");
    let task_id = added["task_id"].as_str().unwrap().to_string();
    let record = store.get(&task_id).unwrap().unwrap();
    assert_eq!(record.download_type, "hls");
    assert_eq!(record.media_format.as_deref(), Some("360p"));
    assert_eq!(wait_for_terminal(&manager, &task_id).await, "completed");
    let requested = requests.lock().unwrap().clone();
    assert!(
        requested.iter().any(|path| path == "v360.m3u8"),
        "{requested:?}"
    );
    assert!(
        !requested.iter().any(|path| path == "v1080.m3u8"),
        "{requested:?}"
    );

    // Stream probes carry the same grouped ladder as YouTube ones.
    let probe = manager
        .call("media.probe", Some(json!({"url": url, "kind": "stream"})))
        .await
        .expect("stream probe");
    let heights: Vec<u64> = probe["qualities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["height"].as_u64().unwrap())
        .collect();
    assert_eq!(heights, [1080, 720, 360], "json: {probe}");
    assert_eq!(probe["default_quality"], "720");

    // Without a height the best variant is used: the previous add did not silently
    // install a default in its place.
    requests.lock().unwrap().clear();
    let added = manager
        .call("media.add", Some(json!({"url": url, "mode": "video"})))
        .await
        .expect("add best");
    let task_id = added["task_id"].as_str().unwrap().to_string();
    assert_eq!(wait_for_terminal(&manager, &task_id).await, "completed");
    let requested = requests.lock().unwrap().clone();
    assert!(
        requested.iter().any(|path| path == "v1080.m3u8"),
        "{requested:?}"
    );

    std::env::remove_var("SEGMENTA_FFMPEG");
    server.abort();
}
