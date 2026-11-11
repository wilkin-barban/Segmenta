//! HLS end-to-end: local manifest + real fragments generated with ffmpeg (S9).
//!
//! The fixture is produced by ffmpeg itself so the concat/remux path runs on real
//! media; if ffmpeg is unavailable the test reports the limitation instead of failing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::RpcHandler;
use segmenta_daemon::manager::Manager;
use segmenta_daemon::store::{Settings, Store};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;

fn generate_hls(dir: &std::path::Path) -> bool {
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=160x120:rate=10",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            "-f",
            "hls",
            "-hls_time",
            "1",
            "-hls_list_size",
            "0",
            "-hls_segment_filename",
        ])
        .arg(dir.join("seg%d.ts"))
        .arg(dir.join("media.m3u8"))
        .status();
    status.map(|s| s.success()).unwrap_or(false)
}

async fn serve(dir: PathBuf) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let dir = dir.clone();
            tokio::spawn(async move {
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
                let body = tokio::fs::read(dir.join(&path)).await.unwrap_or_default();
                let status = if body.is_empty() {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
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

#[tokio::test]
async fn hls_manifest_probes_and_downloads_to_a_merged_file() {
    let dir = tempfile::tempdir().unwrap();
    let media_dir = dir.path().join("media");
    std::fs::create_dir_all(&media_dir).unwrap();
    if !generate_hls(&media_dir) {
        eprintln!("skipping HLS test: ffmpeg with libx264 is not available");
        return;
    }
    // Master playlist pointing at the generated media playlist.
    std::fs::write(
        media_dir.join("master.m3u8"),
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=160x120\nmedia.m3u8\n",
    )
    .unwrap();
    let (port, server) = serve(media_dir).await;
    let url = format!("http://127.0.0.1:{port}/master.m3u8");

    let store = Arc::new(Store::open(&dir.path().join("segmenta.db")).unwrap());
    let settings = Settings {
        download_dir: dir.path().join("out").display().to_string(),
        allow_local_downloads: true,
        ..Settings::default()
    };
    store
        .update_settings(
            &serde_json::to_value(&settings)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
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

    let probed = manager
        .call("media.probe", Some(json!({ "url": url, "kind": "stream" })))
        .await
        .expect("probe");
    assert_eq!(probed["container"], "hls");
    assert!(!probed["formats"].as_array().unwrap().is_empty());

    let added = manager
        .call(
            "media.add",
            Some(json!({ "url": url, "mode": "video", "kind": "stream" })),
        )
        .await
        .expect("add");
    let task_id = added["task_id"].as_str().unwrap().to_string();
    let mut status = String::new();
    for _ in 0..100 {
        let value = manager
            .call("media.status", Some(json!({ "task_id": task_id })))
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
    assert_eq!(record.download_type, "hls");
    let output = PathBuf::from(&record.final_path);
    let size = std::fs::metadata(&output)
        .map(|meta| meta.len())
        .unwrap_or(0);
    assert!(size > 0, "the merged file is empty: {}", output.display());
    server.abort();
}
