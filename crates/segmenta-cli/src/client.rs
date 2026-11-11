//! Blocking JSON-RPC client used by the CLI.
//!
//! The CLI is a short-lived process: a blocking socket, one request at a time and
//! notifications from other tasks skipped while waiting for the matching id.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

/// Failure modes of one CLI call.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The daemon socket does not exist or refuses connections.
    #[error("cannot reach the Segmenta daemon at {path}: {source} (start it with `systemctl --user start segmenta.socket`)")]
    Unavailable {
        /// Socket path that was tried.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The daemon answered with a JSON-RPC error.
    #[error("{message} (code {code})")]
    Rpc {
        /// Numeric code.
        code: i64,
        /// Human message.
        message: String,
    },
    /// The socket carried something that is not a JSON-RPC frame.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// Local I/O failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Where the CLI looks for the socket.
pub fn default_socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("SEGMENTA_SOCKET") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    segmenta_daemon::socket_path()
}

/// One connection to the daemon.
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl Client {
    /// Connect to `path`.
    pub fn connect(path: &Path) -> Result<Client, ClientError> {
        let stream = UnixStream::connect(path).map_err(|source| ClientError::Unavailable {
            path: path.to_path_buf(),
            source,
        })?;
        stream.set_read_timeout(Some(Duration::from_secs(120)))?;
        let writer = stream.try_clone()?;
        Ok(Client {
            reader: BufReader::new(stream),
            writer,
            next_id: 0,
        })
    }

    /// Connect using the default socket path.
    pub fn connect_default() -> Result<Client, ClientError> {
        Client::connect(&default_socket_path())
    }

    /// Send one request and wait for its response, skipping event notifications.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_vec(&request).unwrap_or_default();
        line.push(b'\n');
        self.writer.write_all(&line)?;
        self.writer.flush()?;

        let mut buffer = String::new();
        loop {
            buffer.clear();
            let read = self.reader.read_line(&mut buffer)?;
            if read == 0 {
                return Err(ClientError::Protocol(
                    "the daemon closed the connection".to_string(),
                ));
            }
            let frame: Value = serde_json::from_str(buffer.trim()).map_err(|err| {
                ClientError::Protocol(format!("invalid frame from daemon: {err}"))
            })?;
            // Notifications (events) do not carry the request id; ignore them.
            if frame.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = frame.get("error") {
                let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32603);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
                    .to_string();
                return Err(ClientError::Rpc { code, message });
            }
            return Ok(frame.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}
