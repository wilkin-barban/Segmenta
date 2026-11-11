//! IPC framing tests against a real `UnixListener` standing in for the daemon.
//!
//! They protect the parts of OpenSpec 07 the GUI depends on: NDJSON framing,
//! request/response correlation by `id`, push notifications without `id`,
//! `system.resync_required` and the unreachable-daemon error.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use segmenta_gui::client::{Client, ClientError, ClientEvent, UnixClient};

/// Scripted daemon answer for one request.
type Responder = Arc<dyn Fn(&Value) -> Vec<Value> + Send + Sync>;

/// Answers the handshake and then every request with `respond`.
fn serve(stream: UnixStream, requests: Arc<AtomicUsize>, respond: Responder) {
    let mut writer = stream.try_clone().expect("clone socket");
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { return };
        let value: Value = serde_json::from_str(&line).expect("request is JSON");
        requests.fetch_add(1, Ordering::SeqCst);
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let replies = if method == "system.hello" {
            vec![json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "protocol_version": 1, "daemon_version": "0.0.1", "min_client_version": "0.0.1" }
            })]
        } else {
            respond(&value)
        };
        for reply in replies {
            let mut text = reply.to_string();
            text.push('\n');
            if writer.write_all(text.as_bytes()).is_err() {
                return;
            }
            let _ = writer.flush();
        }
    }
}

fn wait_for<T>(mut poll: impl FnMut() -> Option<T>, timeout: Duration) -> Option<T> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(value) = poll() {
            return Some(value);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    poll()
}

#[test]
fn correlates_responses_by_id_and_forwards_notifications() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&path).expect("bind fake daemon");

    let respond: Responder = Arc::new(|request: &Value| {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = match method {
            "download.list" => json!([{ "task_id": "task-1", "status": "downloading" }]),
            "download.status" => json!({ "task_id": "task-1", "status": "downloading" }),
            _ => json!({}),
        };
        // An unrelated notification, a stale response and the real answer: the
        // client must skip the first two and only resolve the matching id.
        vec![
            json!({
                "jsonrpc": "2.0",
                "method": "download.event",
                "params": { "seq_id": 1, "event_type": "task_progress", "task_id": "task-1", "data": {} }
            }),
            json!({ "jsonrpc": "2.0", "id": 9_999, "result": { "stale": true } }),
            json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        ]
    });

    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            serve(stream, Arc::new(AtomicUsize::new(0)), respond);
        }
    });

    let client = UnixClient::connect(path);
    let answer = client
        .request(
            "download.list",
            json!({ "filter": "all", "limit": 50, "offset": 0 }),
        )
        .expect("list answers");
    assert_eq!(answer.as_array().map(Vec::len), Some(1));
    assert_eq!(answer[0]["task_id"], "task-1");

    let event = wait_for(
        || match client.poll_event() {
            Some(ClientEvent::Notification { method, params }) => Some((method, params)),
            Some(_) => None,
            None => None,
        },
        Duration::from_secs(3),
    )
    .expect("notification forwarded");
    assert_eq!(event.0, "download.event");
    assert_eq!(event.1["seq_id"], 1);
}

#[test]
fn json_rpc_errors_surface_with_their_code() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&path).expect("bind fake daemon");

    let respond: Responder = Arc::new(|request: &Value| {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        vec![json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32602, "message": "max_connections out of range" }
        })]
    });

    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            serve(stream, Arc::new(AtomicUsize::new(0)), respond);
        }
    });

    let client = UnixClient::connect(path);
    let error = client
        .request("settings.update", json!({ "max_connections": 99 }))
        .expect_err("error must surface");
    match error {
        ClientError::Rpc { code, message } => {
            assert_eq!(code, -32602);
            assert!(message.contains("max_connections"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn unreachable_daemon_reports_unavailable_and_keeps_retrying() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("missing.sock");
    let client = UnixClient::connect(path);

    let error = client
        .request("download.list", json!({}))
        .expect_err("no daemon");
    match error {
        ClientError::Unavailable(detail) => assert!(
            detail.contains("missing.sock") || detail.contains("no conectado"),
            "{detail}"
        ),
        other => panic!("unexpected error: {other:?}"),
    }

    // The worker reports the failure once and keeps the connection state.
    let disconnected = wait_for(
        || match client.poll_event() {
            Some(ClientEvent::Disconnected(detail)) => Some(detail),
            Some(_) => None,
            None => None,
        },
        Duration::from_secs(3),
    );
    assert!(disconnected.is_some());
}

#[test]
fn resync_required_is_forwarded_as_a_notification() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&path).expect("bind fake daemon");

    let respond: Responder = Arc::new(|request: &Value| {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        vec![
            json!({
                "jsonrpc": "2.0",
                "method": "system.resync_required",
                "params": { "reason": "event buffer overflow" }
            }),
            json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
        ]
    });

    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            serve(stream, Arc::new(AtomicUsize::new(0)), respond);
        }
    });

    let client = UnixClient::connect(path);
    client.request("download.list", json!({})).expect("answers");
    let notification = wait_for(
        || match client.poll_event() {
            Some(ClientEvent::Notification { method, params }) => Some((method, params)),
            _ => None,
        },
        Duration::from_secs(3),
    )
    .expect("resync forwarded");
    assert_eq!(notification.0, "system.resync_required");
}

#[test]
fn demo_client_mirrors_the_daemon_shapes() {
    // `download.list` never carries segments; `download.status` does.
    let row = json!({
        "task_id": "t", "url": "https://example.com/a.iso", "final_path": "/tmp/a.iso",
        "filename": "a.iso", "status": "downloading", "total_bytes": 10u64,
        "downloaded_bytes": 5u64, "speed_bps": 1u64, "eta_seconds": 5u64,
        "retry_count": 0u32, "supports_ranges": true, "queue_name": "main",
        "created_at": null, "completed_at": null, "error_code": null,
        "error_message": null,
        "segments": [{ "id": 0, "start_offset": 0u64, "end_offset": 9u64, "next_offset": 5u64, "status": "active", "speed_bps": 0u64 }]
    });
    let client = segmenta_gui::client::DemoClient::with_downloads(vec![row]);
    let listed = client.request("download.list", json!({})).expect("list");
    assert_eq!(listed[0]["segments"], json!([]));
    let status = client
        .request("download.status", json!({ "task_id": "t" }))
        .expect("status");
    assert_eq!(status["segments"].as_array().map(Vec::len), Some(1));
}
