use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;

fn start(path: &std::path::Path) -> (Child, ChildStdin, BufReader<ChildStdout>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_segmenta-mcp"))
        .env("SEGMENTA_SOCKET", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let output = BufReader::new(child.stdout.take().unwrap());
    (child, input, output)
}
fn call(input: &mut ChildStdin, output: &mut BufReader<ChildStdout>, message: Value) -> Value {
    writeln!(input, "{message}").unwrap();
    input.flush().unwrap();
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    serde_json::from_str(&line).expect("stdout must contain only JSON-RPC")
}
fn initialize(input: &mut ChildStdin, output: &mut BufReader<ChildStdout>) {
    let reply = call(
        input,
        output,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
    );
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
}
#[test]
fn mcp_tools_forward_existing_daemon_methods() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let daemon = thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..7 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let valid = request["method"] != "download.delete"
                || request["params"]["delete_file"].is_boolean();
            let reply = if valid {
                json!({"jsonrpc":"2.0","id":request["id"],"result":{"ok":true}})
            } else {
                json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32602,"message":"missing delete_file"}})
            };
            writeln!(stream, "{reply}").unwrap();
            requests.push(request);
        }
        requests
    });
    let (mut child, mut input, mut output) = start(&path);
    initialize(&mut input, &mut output);
    let tools = call(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    );
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 5);
    let cases = [
        ("segmenta_list", "download.list", json!({}), json!({})),
        (
            "segmenta_add",
            "download.add",
            json!({"url":"https://example.org/file"}),
            json!({"url":"https://example.org/file"}),
        ),
        (
            "segmenta_pause",
            "download.pause",
            json!({"task_id":"test-id"}),
            json!({"task_id":"test-id"}),
        ),
        (
            "segmenta_resume",
            "download.resume",
            json!({"task_id":"test-id"}),
            json!({"task_id":"test-id"}),
        ),
        (
            "segmenta_delete",
            "download.delete",
            json!({"task_id":"test-id"}),
            json!({"task_id":"test-id","delete_file":false}),
        ),
        (
            "segmenta_delete",
            "download.delete",
            json!({"task_id":"test-id","delete_file":true}),
            json!({"task_id":"test-id","delete_file":true}),
        ),
        (
            "segmenta_delete",
            "download.delete",
            json!({"task_id":"test-id","delete_file":false}),
            json!({"task_id":"test-id","delete_file":false}),
        ),
    ];
    for (i, (tool, _, args, _)) in cases.iter().enumerate() {
        let reply = call(
            &mut input,
            &mut output,
            json!({"jsonrpc":"2.0","id":i+3,"method":"tools/call","params":{"name":tool,"arguments":args}}),
        );
        assert_eq!(reply["id"], i + 3);
        assert_ne!(reply["result"]["isError"], true);
        assert!(reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"ok\":true"));
    }
    drop(input);
    assert!(child.wait().unwrap().success());
    for (request, (_, method, _, expected)) in daemon.join().unwrap().iter().zip(cases.iter()) {
        assert_eq!(request["method"], *method);
        assert_eq!(request["params"], *expected);
    }
}
#[test]
fn lifecycle_requires_initialized_notification_and_ignores_notifications() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut input, mut output) = start(&dir.path().join("missing.sock"));
    let list = |id| json!({"jsonrpc":"2.0","id":id,"method":"tools/list"});
    assert!(call(&mut input, &mut output, list(1))
        .get("error")
        .is_some());
    assert!(call(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{}})
    )
    .get("result")
    .is_some());
    assert!(call(&mut input, &mut output, list(3))
        .get("error")
        .is_some());
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/example"})
    )
    .unwrap();
    let reply = call(&mut input, &mut output, list(4));
    assert_eq!(reply["id"], 4);
    assert!(reply.get("result").is_some());
    assert!(call(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":5,"method":"initialize","params":{}})
    )
    .get("error")
    .is_some());
    drop(input);
    assert!(child.wait().unwrap().success());
}
#[test]
fn daemon_errors_and_notifications_are_handled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let daemon = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        writeln!(stream, "{}", json!({"jsonrpc":"2.0","method":"event"})).unwrap();
        writeln!(
            stream,
            "{}",
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"secret"}})
        )
        .unwrap();
    });
    let (mut child, mut input, mut output) = start(&path);
    initialize(&mut input, &mut output);
    let reply = call(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"segmenta_list","arguments":{}}}),
    );
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(
        reply["result"]["content"][0]["text"],
        "Daemon rejected the request"
    );
    drop(input);
    assert!(child.wait().unwrap().success());
    daemon.join().unwrap();
}
#[test]
fn daemon_unavailable_is_a_tool_error_without_stdout_noise() {
    let dir = tempfile::tempdir().unwrap();
    let (mut child, mut input, mut output) = start(&dir.path().join("missing.sock"));
    initialize(&mut input, &mut output);
    let response = call(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"segmenta_list","arguments":{}}}),
    );
    assert_eq!(response["result"]["isError"], true);
    drop(input);
    assert!(child.wait().unwrap().success());
}
