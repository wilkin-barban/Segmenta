//! Host proxy contract: one framed browser request in, one framed response out.
//!
//! The happy path and the method allowlist run against the real
//! [`segmenta_daemon::ipc::IpcServer`]; the notification, correlation and reconnect
//! cases use a scripted NDJSON server so the assertion is deterministic.

use std::io::{self, BufRead, BufReader, Cursor, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use segmenta_daemon::events::EventBus;
use segmenta_daemon::ipc::{IpcServer, RpcHandler};
use segmenta_daemon::rpc::RpcError;
use segmenta_host::{serve, Activator, HostOptions, NoopActivator};
use serde_json::{json, Value};

/// Numbers the host answers with on its own, before reaching the daemon.
const METHOD_NOT_FOUND: i64 = -32601;
const DAEMON_UNAVAILABLE: i64 = 1029;

fn frame(value: &Value) -> Vec<u8> {
    let payload = serde_json::to_vec(value).expect("serialize");
    let mut bytes = (payload.len() as u32).to_le_bytes().to_vec();
    bytes.extend_from_slice(&payload);
    bytes
}

/// Parses every Native Messaging frame of `bytes`, failing on trailing garbage.
fn read_frames(bytes: &[u8]) -> Vec<Value> {
    let mut frames = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        assert!(
            position + 4 <= bytes.len(),
            "truncated header at byte {position}"
        );
        let length =
            u32::from_le_bytes(bytes[position..position + 4].try_into().expect("header")) as usize;
        position += 4;
        assert!(
            position + length <= bytes.len(),
            "truncated body at byte {position}"
        );
        frames.push(
            serde_json::from_slice(&bytes[position..position + length]).expect("frame is JSON"),
        );
        position += length;
    }
    frames
}

fn run_host(socket: &Path, requests: &[Value]) -> (io::Result<()>, Vec<Value>) {
    run_host_with(socket, requests, Box::new(NoopActivator))
}

fn run_host_with(
    socket: &Path,
    requests: &[Value],
    activator: Box<dyn Activator>,
) -> (io::Result<()>, Vec<Value>) {
    run_with(requests, HostOptions::new(socket.to_path_buf(), activator))
}

fn run_with(requests: &[Value], options: HostOptions) -> (io::Result<()>, Vec<Value>) {
    let mut stdin = Vec::new();
    for request in requests {
        stdin.extend_from_slice(&frame(request));
    }
    let mut input = Cursor::new(stdin);
    let mut stdout = Vec::new();
    let result = serve(&mut input, &mut stdout, &options);
    (result, read_frames(&stdout))
}

/// Stub daemon handler: answers `media.probe` and counts every call.
#[derive(Default)]
struct StubHandler {
    calls: AtomicUsize,
}

impl RpcHandler for StubHandler {
    fn call<'a>(
        &'a self,
        method: &'a str,
        _params: Option<Value>,
    ) -> BoxFuture<'a, Result<Value, RpcError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match method {
                "media.probe" => Ok(json!({
                    "kind": "video",
                    "title": "demo",
                    "duration_seconds": 3,
                    "thumbnail": "https://example.invalid/t.jpg",
                    "formats": [],
                    "audio": [],
                    "playlist": null,
                })),
                "system.hello" => Ok(json!({ "protocol_version": 1 })),
                other => Err(RpcError::method_not_found(other)),
            }
        })
    }
}

/// A real daemon IPC server bound to a private temporary socket.
struct Daemon {
    _dir: tempfile::TempDir,
    path: PathBuf,
    handler: Arc<StubHandler>,
    /// Keeps the runtime (and therefore the serving task) alive for the whole test.
    _runtime: tokio::runtime::Runtime,
}

impl Daemon {
    fn start() -> Daemon {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("daemon.sock");
        let handler = Arc::new(StubHandler::default());
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let bus = Arc::new(EventBus::new());
        let server = runtime
            .block_on(IpcServer::bind(&path, bus))
            .expect("bind the ipc server");
        let serving: Arc<dyn RpcHandler> = handler.clone();
        let uid = current_uid();
        runtime.spawn(async move { Arc::new(server).run(serving, uid).await });
        Daemon {
            _dir: dir,
            path,
            handler,
            _runtime: runtime,
        }
    }

    fn calls(&self) -> usize {
        self.handler.calls.load(Ordering::SeqCst)
    }
}

fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[test]
fn proxies_media_probe_with_the_same_id() {
    let daemon = Daemon::start();
    let request = json!({
        "jsonrpc": "2.0",
        "id": "probe-1",
        "method": "media.probe",
        "params": { "url": "https://www.youtube.com/watch?v=demo", "kind": "video" },
    });

    let (result, frames) = run_host(&daemon.path, &[request]);

    result.expect("the host exits cleanly on stdin EOF");
    assert_eq!(frames.len(), 1, "one request, one response: {frames:?}");
    assert_eq!(frames[0]["jsonrpc"], "2.0");
    assert_eq!(frames[0]["id"], "probe-1");
    assert_eq!(frames[0]["result"]["title"], "demo");
    assert_eq!(daemon.calls(), 1);
}

#[test]
fn an_unknown_method_is_rejected_without_touching_the_daemon() {
    let daemon = Daemon::start();
    let requests = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "extension.eval", "params": { "command": "id" } }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "media.", "params": {} }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "shell", "params": {} }),
        json!({ "jsonrpc": "2.0", "id": 4, "method": "/etc/passwd", "params": {} }),
    ];

    let (result, frames) = run_host(&daemon.path, &requests);

    result.expect("the host keeps running after a rejected method");
    assert_eq!(frames.len(), requests.len());
    for (frame, request) in frames.iter().zip(requests.iter()) {
        assert_eq!(frame["id"], request["id"]);
        assert_eq!(frame["error"]["code"], METHOD_NOT_FOUND, "{frame:?}");
        assert_eq!(frame["error"]["data"]["code"], "METHOD_NOT_FOUND");
    }
    assert_eq!(daemon.calls(), 0, "the daemon must never see the request");
}

#[test]
fn the_allowed_prefixes_reach_the_daemon() {
    let daemon = Daemon::start();
    let requests = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "system.hello", "params": {} }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "download.list", "params": {} }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "settings.get", "params": {} }),
        json!({ "jsonrpc": "2.0", "id": 4, "method": "media.probe", "params": { "url": "https://example.invalid" } }),
    ];

    let (result, frames) = run_host(&daemon.path, &requests);

    result.expect("clean exit");
    assert_eq!(frames.len(), requests.len());
    assert_eq!(frames[0]["result"]["protocol_version"], 1);
    // `download.list`, `settings.get`: the stub answers -32601 itself.
    assert_eq!(frames[1]["error"]["code"], METHOD_NOT_FOUND);
    assert_eq!(frames[2]["error"]["code"], METHOD_NOT_FOUND);
    assert_eq!(frames[3]["result"]["kind"], "video");
    assert_eq!(daemon.calls(), requests.len());
}

/// Records how often the host asked systemd to start the socket unit.
#[derive(Clone, Default)]
struct CountingActivator(Arc<AtomicUsize>);

impl Activator for CountingActivator {
    fn activate(&self, _socket_path: &Path) -> io::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn a_missing_socket_answers_1029_and_asks_systemd_only_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("segmenta").join("daemon.sock");
    let activator = CountingActivator::default();
    let requests = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "media.probe", "params": { "url": "https://example.invalid" } }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "media.probe", "params": { "url": "https://example.invalid" } }),
    ];

    let started = Instant::now();
    let (result, frames) = run_host_with(&socket, &requests, Box::new(activator.clone()));
    let elapsed = started.elapsed();

    result.expect("the host does not die when the daemon is down");
    assert!(
        elapsed < Duration::from_secs(5),
        "the absent-socket path hung for {elapsed:?}"
    );
    assert_eq!(frames.len(), 2);
    for (frame, request) in frames.iter().zip(requests.iter()) {
        assert_eq!(frame["id"], request["id"]);
        assert_eq!(frame["error"]["code"], DAEMON_UNAVAILABLE, "{frame:?}");
        let message = frame["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(socket.to_str().expect("utf-8 path")),
            "the message must name the socket: {message}"
        );
        assert!(
            message.contains("systemctl --user start segmenta.socket"),
            "the message must name the actionable command: {message}"
        );
    }
    assert_eq!(
        activator.0.load(Ordering::SeqCst),
        1,
        "systemd is asked once per host process"
    );
}

/// Scripted NDJSON daemon: accepts one connection and answers with `script`.
fn scripted_daemon(
    dir: &tempfile::TempDir,
    script: fn(&Value) -> Vec<Value>,
) -> (PathBuf, std::thread::JoinHandle<()>) {
    let path = dir.path().join("scripted.sock");
    let listener = UnixListener::bind(&path).expect("bind scripted daemon");
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut writer = stream;
        let mut line = String::new();
        let read = reader.read_line(&mut line).expect("read request");
        assert!(read > 0, "the host must send a request line");
        let request: Value = serde_json::from_str(line.trim()).expect("request is JSON");
        for value in script(&request) {
            let mut bytes = serde_json::to_vec(&value).expect("serialize");
            bytes.push(b'\n');
            writer.write_all(&bytes).expect("write response");
        }
        writer.flush().expect("flush");
    });
    (path, handle)
}

#[test]
fn events_and_stale_responses_are_skipped_until_the_id_matches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, server) = scripted_daemon(&dir, |request| {
        let id = request["id"].clone();
        vec![
            json!({
                "jsonrpc": "2.0",
                "method": "download.event",
                "params": { "seq_id": 4, "event_type": "task_progress", "data": { "percent": 12 } },
            }),
            json!({ "jsonrpc": "2.0", "id": "someone-else", "result": { "stale": true } }),
            json!({ "jsonrpc": "2.0", "id": id, "result": { "kind": "video" } }),
        ]
    });
    let request = json!({ "jsonrpc": "2.0", "id": "probe-9", "method": "media.probe" });

    let (result, frames) = run_host(&path, &[request]);

    result.expect("clean exit");
    server.join().expect("scripted daemon");
    assert_eq!(frames.len(), 1, "only the correlated response is forwarded");
    assert_eq!(frames[0]["id"], "probe-9");
    assert_eq!(frames[0]["result"]["kind"], "video");
}

#[test]
fn a_closed_connection_is_reopened_for_the_next_request() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("reconnect.sock");
    let listener = UnixListener::bind(&path).expect("bind");
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut writer = stream;
            let mut line = String::new();
            reader.read_line(&mut line).expect("read request");
            let request: Value = serde_json::from_str(line.trim()).expect("request is JSON");
            let mut bytes = serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": { "method": request["method"] },
            }))
            .expect("serialize");
            bytes.push(b'\n');
            writer.write_all(&bytes).expect("write");
            writer.flush().expect("flush");
            // Dropping `writer` and `reader` closes the connection.
        }
    });
    let requests = [
        json!({ "jsonrpc": "2.0", "id": "a", "method": "media.probe" }),
        json!({ "jsonrpc": "2.0", "id": "b", "method": "media.probe" }),
    ];

    let (result, frames) = run_host(&path, &requests);

    result.expect("clean exit");
    server.join().expect("server thread");
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert_eq!(frames[0]["id"], "a");
    assert_eq!(frames[1]["id"], "b");
    assert_eq!(frames[1]["result"]["method"], "media.probe");
}

#[test]
fn a_broken_envelope_is_rejected_without_forwarding() {
    let daemon = Daemon::start();
    let requests = [
        json!({ "jsonrpc": "2.0", "id": 1 }),
        json!({ "jsonrpc": "1.0", "id": 2, "method": "media.probe" }),
        json!({ "id": 3, "method": "media.probe" }),
        json!([]),
    ];

    let (result, frames) = run_host(&daemon.path, &requests);

    result.expect("clean exit");
    assert_eq!(frames.len(), requests.len());
    assert_eq!(frames[0]["error"]["code"], -32600, "no method: {frames:?}");
    assert_eq!(frames[1]["error"]["code"], -32600);
    assert_eq!(frames[2]["error"]["code"], -32600);
    assert_eq!(frames[3]["error"]["code"], -32600);
    assert_eq!(daemon.calls(), 0);
}

#[test]
fn one_connection_serves_the_whole_browser_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("persistent.sock");
    let listener = UnixListener::bind(&path).expect("bind");
    let server = std::thread::spawn(move || {
        // A single accept: a host that reconnects per request cannot be answered.
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut writer = stream;
        for expected in ["a", "b"] {
            let mut line = String::new();
            let read = reader.read_line(&mut line).expect("read request");
            assert!(read > 0, "the session ended before request `{expected}`");
            let request: Value = serde_json::from_str(line.trim()).expect("request is JSON");
            assert_eq!(request["id"], expected);
            let mut bytes = serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": { "reused": true },
            }))
            .expect("serialize");
            bytes.push(b'\n');
            writer.write_all(&bytes).expect("write");
            writer.flush().expect("flush");
        }
    });
    let requests = [
        json!({ "jsonrpc": "2.0", "id": "a", "method": "media.probe" }),
        json!({ "jsonrpc": "2.0", "id": "b", "method": "media.status" }),
    ];
    let options = HostOptions::new(path, Box::new(NoopActivator))
        .with_response_timeout(Duration::from_secs(2));

    let (result, frames) = run_with(&requests, options);

    result.expect("clean exit");
    server.join().expect("server thread");
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert_eq!(frames[0]["result"]["reused"], true);
    assert_eq!(frames[1]["result"]["reused"], true);
}
