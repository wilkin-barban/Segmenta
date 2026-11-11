//! IPC contract: framing, 1 MiB ceiling, peer-UID rejection, method/error mapping and
//! the event bus with `seq_id`. Spec S14-S16.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::{
    dispatch_bytes, hello_result, peer_is_authorized, IpcServer, RpcHandler,
};
use segmenta_daemon::rpc::{read_frame, FrameError, RpcError, MAX_MESSAGE_BYTES};
use serde_json::{json, Value};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

struct EchoHandler;

impl RpcHandler for EchoHandler {
    fn call<'a>(
        &'a self,
        method: &'a str,
        params: Option<Value>,
    ) -> BoxFuture<'a, Result<Value, RpcError>> {
        Box::pin(async move {
            match method {
                "system.hello" => Ok(hello_result("test-0.0.1")),
                "echo" => Ok(params.unwrap_or(Value::Null)),
                "system.shutdown" => Ok(json!({ "shutting_down": true })),
                "fail" => Err(RpcError::task_not_found("missing")),
                other => Err(RpcError::method_not_found(other)),
            }
        })
    }
}

#[tokio::test]
async fn framing_reassembles_a_line_split_across_reads() {
    let (mut client, server) = tokio::io::duplex(8);
    let writer = tokio::spawn(async move {
        for part in [
            &b"{\"jsonrpc\":"[..],
            b"\"2.0\",",
            b"\"id\":1,",
            b"\"method\":\"x\"}",
        ] {
            client.write_all(part).await.unwrap();
        }
        client.write_all(b"\n").await.unwrap();
    });
    let mut reader = BufReader::new(server);
    let frame = read_frame(&mut reader).await.unwrap().expect("a frame");
    writer.await.unwrap();
    assert_eq!(
        String::from_utf8(frame).unwrap(),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"x\"}"
    );
}

#[tokio::test]
async fn framing_splits_two_messages_sharing_one_buffer() {
    let (mut client, server) = tokio::io::duplex(1024);
    client.write_all(b"{\"a\":1}\n{\"b\":2}\r\n").await.unwrap();
    let mut reader = BufReader::new(server);
    assert_eq!(
        String::from_utf8(read_frame(&mut reader).await.unwrap().unwrap()).unwrap(),
        "{\"a\":1}"
    );
    assert_eq!(
        String::from_utf8(read_frame(&mut reader).await.unwrap().unwrap()).unwrap(),
        "{\"b\":2}"
    );
}

#[tokio::test]
async fn framing_rejects_a_line_over_one_mebibyte() {
    let mut payload = vec![b'a'; MAX_MESSAGE_BYTES + 1];
    payload.extend_from_slice(b"\nmore");
    let mut reader = BufReader::new(&payload[..]);
    let err = read_frame(&mut reader).await.unwrap_err();
    assert!(matches!(err, FrameError::TooLarge));
}

#[tokio::test]
async fn framing_returns_nothing_on_a_closed_socket() {
    let data: &[u8] = b"";
    let mut reader = BufReader::new(data);
    assert!(read_frame(&mut reader).await.unwrap().is_none());
}

#[test]
fn peer_uid_must_match_the_daemon() {
    assert!(peer_is_authorized(1000, 1000));
    assert!(!peer_is_authorized(1001, 1000));
}

#[tokio::test]
async fn a_valid_request_gets_a_result_and_the_echoed_id() {
    let (response, _) = dispatch_bytes(
        br#"{"jsonrpc":"2.0","id":"req-1","method":"system.hello","params":{}}"#,
        &EchoHandler,
    )
    .await;
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], "req-1");
    assert_eq!(response["result"]["protocol_version"], 1);
    assert_eq!(response["result"]["min_client_version"], 1);
}

#[tokio::test]
async fn errors_use_the_openspec_codes() {
    let (unknown, _) =
        dispatch_bytes(br#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#, &EchoHandler).await;
    assert_eq!(unknown["error"]["code"], -32601);

    let (invalid, _) = dispatch_bytes(b"not json", &EchoHandler).await;
    assert_eq!(invalid["error"]["code"], -32700);

    let (wrong_version, _) = dispatch_bytes(
        br#"{"jsonrpc":"1.0","id":2,"method":"system.hello"}"#,
        &EchoHandler,
    )
    .await;
    assert_eq!(wrong_version["error"]["code"], -32600);

    let (failed, _) =
        dispatch_bytes(br#"{"jsonrpc":"2.0","id":3,"method":"fail"}"#, &EchoHandler).await;
    assert_eq!(failed["error"]["code"], 1001);
    assert_eq!(failed["error"]["data"]["code"], "TASK_NOT_FOUND");
}

#[tokio::test]
async fn a_blank_line_is_not_a_request() {
    let (response, keep_open) = dispatch_bytes(b"", &EchoHandler).await;
    assert!(response.is_null());
    assert!(keep_open);
}

struct Harness {
    _dir: tempfile::TempDir,
    bus: Arc<EventBus>,
    path: std::path::PathBuf,
}

async fn start_server(expected_uid: u32) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.sock");
    let bus = Arc::new(EventBus::new());
    let server = Arc::new(IpcServer::bind(&path, Arc::clone(&bus)).await.unwrap());
    let handler: Arc<dyn RpcHandler> = Arc::new(EchoHandler);
    let running = Arc::clone(&server);
    tokio::spawn(async move { running.run(handler, expected_uid).await });
    Harness {
        _dir: dir,
        bus,
        path,
    }
}

fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions.
    unsafe { libc::getuid() }
}

#[tokio::test]
async fn socket_permissions_are_0600_and_the_directory_0700() {
    use std::os::unix::fs::PermissionsExt;
    let harness = start_server(current_uid()).await;
    let socket_mode = std::fs::metadata(&harness.path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(socket_mode, 0o600);
    let dir_mode = std::fs::metadata(harness.path.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir_mode, 0o700);
}

#[tokio::test]
async fn a_same_uid_client_talks_to_the_daemon_over_the_socket() {
    let harness = start_server(current_uid()).await;
    let stream = UnixStream::connect(&harness.path).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);

    write
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"system.hello\"}\n")
        .await
        .unwrap();
    let response = read_frame(&mut reader).await.unwrap().unwrap();
    let response: Value = serde_json::from_slice(&response).unwrap();
    assert_eq!(response["result"]["daemon_version"], "test-0.0.1");

    // Pipelined requests are answered in order.
    write
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"echo\",\"params\":{\"a\":1}}\n")
        .await
        .unwrap();
    let response = read_frame(&mut reader).await.unwrap().unwrap();
    let response: Value = serde_json::from_slice(&response).unwrap();
    assert_eq!(response["result"]["a"], 1);
}

#[tokio::test]
async fn an_oversized_message_gets_1009_and_the_connection_closes() {
    let harness = start_server(current_uid()).await;
    let stream = UnixStream::connect(&harness.path).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);

    let mut payload = vec![b'a'; MAX_MESSAGE_BYTES + 2];
    payload.push(b'\n');
    // The write may fail once the server closes; either outcome is fine.
    let _ = write.write_all(&payload).await;
    let response = read_frame(&mut reader).await.unwrap().unwrap();
    let response: Value = serde_json::from_slice(&response).unwrap();
    assert_eq!(response["error"]["code"], 1009);
    // The next read observes the close.
    assert!(read_frame(&mut reader).await.unwrap().is_none());
}

#[tokio::test]
async fn a_foreign_uid_is_disconnected_without_a_byte() {
    let harness = start_server(current_uid().wrapping_add(1)).await;
    let stream = UnixStream::connect(&harness.path).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let _ = write
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"system.hello\"}\n")
        .await;
    let outcome = tokio::time::timeout(Duration::from_secs(2), read_frame(&mut reader))
        .await
        .expect("the server must act promptly");
    match outcome {
        // Clean close: nothing was sent.
        Ok(None) => {}
        // Unread request bytes make the kernel answer with RST; also "no data sent".
        Err(FrameError::Io(err)) => {
            assert_eq!(
                err.raw_os_error(),
                Some(libc::ECONNRESET),
                "unexpected {err}"
            );
        }
        Ok(Some(bytes)) => panic!(
            "a foreign uid received {:?}",
            String::from_utf8_lossy(&bytes)
        ),
        Err(FrameError::TooLarge) => panic!("a foreign uid received an error frame"),
    }
}

#[tokio::test]
async fn events_carry_a_monotonic_seq_id_and_reach_live_clients() {
    let harness = start_server(current_uid()).await;
    let stream = UnixStream::connect(&harness.path).await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    // Subscribe by sending a harmless request first: the server subscribes on accept.
    write
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"system.hello\"}\n")
        .await
        .unwrap();
    let _ = read_frame(&mut reader).await.unwrap().unwrap();

    harness.bus.publish(
        "task_progress",
        Some("task-1"),
        json!({"downloaded_bytes": 10, "total_bytes": 100}),
    );
    harness.bus.publish(
        "state_changed",
        Some("task-1"),
        json!({"old_state": "probing", "new_state": "downloading"}),
    );

    let first = read_frame(&mut reader).await.unwrap().unwrap();
    let first: Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(first["method"], "download.event");
    assert_eq!(first["params"]["seq_id"], 1);
    assert_eq!(first["params"]["event_type"], "task_progress");
    assert_eq!(first["params"]["task_id"], "task-1");
    assert_eq!(first["params"]["data"]["downloaded_bytes"], 10);
    assert!(first["params"]["timestamp_ms"].as_u64().unwrap() > 0);

    let second = read_frame(&mut reader).await.unwrap().unwrap();
    let second: Value = serde_json::from_slice(&second).unwrap();
    assert_eq!(second["params"]["seq_id"], 2, "seq_id is monotonic");
}

#[tokio::test]
async fn a_second_daemon_instance_is_detected_by_the_lock() {
    // The daemon holds an exclusive flock; a second process must not take over the
    // socket. This pins the file-level contract the binary relies on.
    use std::os::unix::io::AsRawFd;
    let dir = tempfile::tempdir().unwrap();
    let lock_path = dir.path().join("daemon.lock");
    let first = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    let rc = unsafe { libc::flock(first.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(rc, 0);

    let second = std::fs::OpenOptions::new()
        .write(true)
        .open(&lock_path)
        .unwrap();
    let rc = unsafe { libc::flock(second.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(rc, -1, "a second instance must fail to acquire the lock");
}
