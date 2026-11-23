use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SEGMENTA_SOCKET").filter(|p| !p.is_empty()) {
        return path.into();
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/segmenta-{}", unsafe { libc::geteuid() })));
    if std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|p| !p.is_empty()) {
        runtime.join("segmenta/daemon.sock")
    } else {
        runtime.join("daemon.sock")
    }
}

const MAX_FRAME: u64 = 1024 * 1024;
const MAX_DAEMON_FRAMES: usize = 128;

fn read_frame(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    reader.take(MAX_FRAME + 1).read_until(b'\n', &mut line)?;
    if line.len() as u64 > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Frame too large",
        ));
    }
    if line.is_empty() {
        Ok(None)
    } else {
        Ok(Some(line))
    }
}

fn daemon_call(method: &str, params: Value) -> Result<Value, &'static str> {
    let mut stream = UnixStream::connect(socket_path()).map_err(|_| "Daemon unavailable")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|_| "Daemon unavailable")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|_| "Daemon unavailable")?;
    writeln!(
        stream,
        "{}",
        json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params})
    )
    .map_err(|_| "Daemon communication failed")?;
    let mut reader = BufReader::new(stream);
    for _ in 0..MAX_DAEMON_FRAMES {
        let line = read_frame(&mut reader)
            .map_err(|_| "Daemon communication failed")?
            .ok_or("Daemon connection closed")?;
        let response: Value =
            serde_json::from_slice(&line).map_err(|_| "Invalid daemon response")?;
        if response.get("id") != Some(&json!(1)) {
            continue;
        }
        if response.get("error").is_some() {
            return Err("Daemon rejected the request");
        }
        return response
            .get("result")
            .cloned()
            .ok_or("Invalid daemon response");
    }
    Err("Too many daemon messages")
}

fn tools() -> Value {
    let list = json!({"type":"object","properties":{"filter":{"type":"string"},"limit":{"type":"integer"},"offset":{"type":"integer"}}});
    let add = json!({"type":"object","properties":{"url":{"type":"string"},"destination_dir":{"type":"string"},"filename":{"type":"string"},"connections":{"type":"integer"},"start_immediately":{"type":"boolean"},"queue_name":{"type":"string"},"priority":{"type":"integer"}},"required":["url"]});
    let task =
        json!({"type":"object","properties":{"task_id":{"type":"string"}},"required":["task_id"]});
    let delete = json!({"type":"object","properties":{"task_id":{"type":"string"},"delete_file":{"type":"boolean","default":false}},"required":["task_id"]});
    json!({"tools":[
        {"name":"segmenta_list","description":"List downloads","inputSchema":list},
        {"name":"segmenta_add","description":"Add a download","inputSchema":add},
        {"name":"segmenta_pause","description":"Pause a download","inputSchema":task},
        {"name":"segmenta_resume","description":"Resume a download","inputSchema":task},
        {"name":"segmenta_delete","description":"Delete a download; optionally delete its file","inputSchema":delete}
    ]})
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message}})
}

#[derive(PartialEq)]
enum State {
    New,
    Initializing,
    Ready,
}

fn handle(request: Value, state: &mut State) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str);
    if request.get("jsonrpc") != Some(&json!("2.0")) || method.is_none() {
        return Some(error(id.unwrap_or(Value::Null), -32600, "Invalid Request"));
    }
    let method = method.unwrap();
    if id.is_none() {
        if method == "notifications/initialized" && *state == State::Initializing {
            *state = State::Ready;
        }
        return None;
    }
    let id = id.unwrap();
    if method == "initialize" {
        if *state != State::New {
            return Some(error(id, -32600, "Already initialized"));
        }
        *state = State::Initializing;
    } else if *state != State::Ready {
        return Some(error(id, -32002, "Server not initialized"));
    }
    let result = match method {
        "initialize" => {
            json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"segmenta-mcp","version":env!("CARGO_PKG_VERSION")}})
        }
        "ping" => json!({}),
        "tools/list" => tools(),
        "tools/call" => {
            let params = request.get("params").and_then(Value::as_object);
            let Some(name) = params.and_then(|p| p.get("name")).and_then(Value::as_str) else {
                return Some(error(id, -32602, "Invalid params"));
            };
            let method = match name {
                "segmenta_list" => "download.list",
                "segmenta_add" => "download.add",
                "segmenta_pause" => "download.pause",
                "segmenta_resume" => "download.resume",
                "segmenta_delete" => "download.delete",
                _ => return Some(error(id, -32602, "Unknown tool")),
            };
            let mut args = params
                .and_then(|p| p.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let Some(args_object) = args.as_object_mut() else {
                return Some(error(id, -32602, "Invalid params"));
            };
            if method == "download.delete" {
                args_object.entry("delete_file").or_insert(json!(false));
            }
            match daemon_call(method, args) {
                Ok(value) => json!({"content":[{"type":"text","text":value.to_string()}]}),
                Err(message) => json!({"content":[{"type":"text","text":message}],"isError":true}),
            }
        }
        _ => return Some(error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0", "id":id, "result":result}))
}

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut input = stdin.lock();
    let mut state = State::New;
    while let Some(line) = read_frame(&mut input)? {
        let response = match serde_json::from_slice(&line) {
            Ok(request) => handle(request, &mut state),
            Err(_) => Some(error(Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = response {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_bounded() {
        let mut valid = io::Cursor::new(vec![b'x'; MAX_FRAME as usize]);
        assert_eq!(
            read_frame(&mut valid).unwrap().unwrap().len(),
            MAX_FRAME as usize
        );
        let mut oversized = io::Cursor::new(vec![b'x'; MAX_FRAME as usize + 1]);
        assert_eq!(
            read_frame(&mut oversized).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
