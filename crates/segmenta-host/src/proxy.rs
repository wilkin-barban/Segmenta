//! Browser ⇄ daemon proxy: one framed Native Messaging request in, one framed
//! JSON-RPC response out (OpenSpec 03 §4, ODD S1).
//!
//! Security boundaries of this module:
//!
//! * only methods under the `media.`, `system.`, `download.` and `settings.` prefixes
//!   reach the daemon; everything else is answered with `-32601` locally;
//! * the browser never supplies a command, a path or an argument vector; the only
//!   external process is `systemctl --user start segmenta.socket` with a fixed argv;
//! * diagnostics carry method names and error codes, never payloads.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::framing::{
    read_message, write_message, FrameError, ERR_FRAME_INVALID_JSON, ERR_FRAME_TOO_LARGE,
};

/// Numeric code the browser sees when the daemon cannot be reached (ODD S1).
pub const ERR_DAEMON_UNAVAILABLE: i64 = 1029;

/// Environment variable that overrides the daemon socket path.
pub const SOCKET_ENV: &str = "SEGMENTA_SOCKET";

/// systemd socket unit that exposes the daemon.
pub const SYSTEMD_UNIT: &str = "segmenta.socket";

/// Prefixes the host is willing to forward. Nothing else leaves the process.
pub const ALLOWED_METHOD_PREFIXES: [&str; 4] = ["media.", "system.", "download.", "settings."];

/// How long a single daemon round trip may take before the browser gets an error.
pub const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INTERNAL_ERROR: i64 = -32603;

/// `$SEGMENTA_SOCKET`, else `$XDG_RUNTIME_DIR/segmenta/daemon.sock`, else
/// `/tmp/segmenta-$UID/daemon.sock` — the same resolution as the CLI.
pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os(SOCKET_ENV) {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() {
            return PathBuf::from(runtime).join("segmenta").join("daemon.sock");
        }
    }
    // SAFETY: getuid has no preconditions and cannot fail.
    PathBuf::from(format!("/tmp/segmenta-{}", unsafe { libc::getuid() })).join("daemon.sock")
}

/// Something that can ask systemd to bring the socket unit up.
pub trait Activator: Send + Sync {
    /// Ask systemd to start the socket unit. Called at most once per process.
    fn activate(&self, socket_path: &Path) -> std::io::Result<()>;
}

/// Production activator: `systemctl --user start segmenta.socket`.
///
/// The argv is fixed and separated; there is no shell and no browser input.
pub struct SystemctlActivator;

impl Activator for SystemctlActivator {
    fn activate(&self, _socket_path: &Path) -> std::io::Result<()> {
        let status = std::process::Command::new("systemctl")
            .arg("--user")
            .arg("start")
            .arg(SYSTEMD_UNIT)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "`systemctl --user start {SYSTEMD_UNIT}` exited with {status}"
            )))
        }
    }
}

/// Activator for tests and dry runs: never starts anything.
pub struct NoopActivator;

impl Activator for NoopActivator {
    fn activate(&self, _socket_path: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

/// Runtime configuration of the host loop.
pub struct HostOptions {
    /// Daemon socket this session talks to.
    pub socket_path: PathBuf,
    /// Round-trip budget of a single request.
    pub response_timeout: Duration,
    /// Used once when the socket is missing.
    pub activator: Box<dyn Activator>,
}

impl HostOptions {
    /// Options with the default round-trip budget.
    pub fn new(socket_path: PathBuf, activator: Box<dyn Activator>) -> HostOptions {
        HostOptions {
            socket_path,
            response_timeout: DEFAULT_RESPONSE_TIMEOUT,
            activator,
        }
    }

    /// Override the round-trip budget.
    pub fn with_response_timeout(mut self, timeout: Duration) -> HostOptions {
        self.response_timeout = timeout;
        self
    }
}

/// Serve one browser session: framed requests from `input`, framed responses to
/// `output`. Returns `Ok(())` on stdin EOF so the process exits with code 0.
pub fn serve<R: Read + ?Sized, W: Write + ?Sized>(
    input: &mut R,
    output: &mut W,
    options: &HostOptions,
) -> std::io::Result<()> {
    let mut session = Session::new(options);
    loop {
        let message = match read_message(input) {
            Ok(Some(message)) => message,
            // Clean EOF: the browser closed the pipe.
            Ok(None) => return Ok(()),
            Err(FrameError::Empty) => {
                let response = error_frame(
                    Value::Null,
                    INVALID_REQUEST,
                    "INVALID_REQUEST",
                    "ERR_FRAME_EMPTY: message length is zero",
                );
                write_frame(output, &response)?;
                continue;
            }
            Err(FrameError::InvalidJson { .. }) => {
                let response = error_frame(
                    Value::Null,
                    PARSE_ERROR,
                    "PARSE_ERROR",
                    "ERR_FRAME_INVALID_JSON: the message is not UTF-8 JSON",
                );
                write_frame(output, &response)?;
                continue;
            }
            Err(FrameError::Truncated) => {
                eprintln!(
                    "segmenta-host: ERR_FRAME_TRUNCATED: stdin ended in the middle of a message"
                );
                return Ok(());
            }
            Err(FrameError::TooLarge { length }) => {
                // The body is still in the stream; resynchronizing it is not possible.
                eprintln!("segmenta-host: {ERR_FRAME_TOO_LARGE}: {length} bytes from the browser");
                let response = error_frame(
                    Value::Null,
                    INVALID_REQUEST,
                    "INVALID_REQUEST",
                    &format!("{ERR_FRAME_TOO_LARGE}: the message exceeds the 1 MiB limit"),
                );
                write_frame(output, &response)?;
                return Ok(());
            }
            Err(FrameError::Io(err)) => return Err(err),
        };
        let response = session.handle(&message);
        write_frame(output, &response)?;
    }
}

/// True when `method` is inside one of the allowed prefixes with a real name after it.
pub fn is_allowed_method(method: &str) -> bool {
    ALLOWED_METHOD_PREFIXES
        .iter()
        .any(|prefix| method.strip_prefix(prefix).is_some_and(is_method_tail))
}

/// A forwarded method name is a dotted identifier: no paths, spaces or control bytes.
fn is_method_tail(tail: &str) -> bool {
    !tail.is_empty()
        && tail
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '_' || char == '.')
}

/// One browser session: the request loop plus the daemon link it reuses.
struct Session<'a> {
    options: &'a HostOptions,
    link: Option<Link>,
    /// Whether systemd was already asked to start the socket unit.
    activated: bool,
}

impl<'a> Session<'a> {
    fn new(options: &'a HostOptions) -> Session<'a> {
        Session {
            options,
            link: None,
            activated: false,
        }
    }

    /// Validate one browser message and produce the single response frame.
    fn handle(&mut self, message: &Value) -> Value {
        let Some(object) = message.as_object() else {
            return error_frame(
                Value::Null,
                INVALID_REQUEST,
                "INVALID_REQUEST",
                "the message must be a JSON object",
            );
        };
        let id = object.get("id").cloned().unwrap_or(Value::Null);
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return error_frame(
                id,
                INVALID_REQUEST,
                "INVALID_REQUEST",
                "jsonrpc must be \"2.0\"",
            );
        }
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return error_frame(id, INVALID_REQUEST, "INVALID_REQUEST", "missing method");
        };
        if !is_allowed_method(method) {
            // Logged by name only: the payload never leaves the process.
            eprintln!("segmenta-host: rejected method with {METHOD_NOT_FOUND}");
            return error_frame(
                id,
                METHOD_NOT_FOUND,
                "METHOD_NOT_FOUND",
                &format!("unknown method `{method}`"),
            );
        }
        match self.forward(message, &id) {
            Ok(response) => response,
            Err(err) => {
                eprintln!(
                    "segmenta-host: {ERR_DAEMON_UNAVAILABLE} for `{method}`: {}",
                    err.kind()
                );
                let message = self.unavailable_message(&err);
                error_frame(
                    id,
                    ERR_DAEMON_UNAVAILABLE,
                    "ERR_DAEMON_UNAVAILABLE",
                    &message,
                )
            }
        }
    }

    /// The browser-facing explanation of a failed daemon call.
    fn unavailable_message(&self, err: &std::io::Error) -> String {
        format!(
            "cannot reach the Segmenta daemon at {}: {err}. Run `systemctl --user start {SYSTEMD_UNIT}` and retry",
            self.options.socket_path.display()
        )
    }

    /// One request → one correlated response, reconnecting once on a broken pipe.
    fn forward(&mut self, request: &Value, id: &Value) -> std::io::Result<Value> {
        self.ensure_link()?;
        match self.roundtrip(request, id) {
            Ok(Some(response)) => Ok(response),
            Ok(None) => {
                // The daemon closed the connection; reopen it and try once more.
                self.link = None;
                self.retry(request, id)
            }
            Err(err) => {
                self.link = None;
                if is_recoverable(&err) {
                    self.retry(request, id)
                } else {
                    Err(err)
                }
            }
        }
    }

    fn retry(&mut self, request: &Value, id: &Value) -> std::io::Result<Value> {
        self.ensure_link()?;
        match self.roundtrip(request, id)? {
            Some(response) => Ok(response),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "the daemon closed the connection",
            )),
        }
    }

    fn roundtrip(&mut self, request: &Value, id: &Value) -> std::io::Result<Option<Value>> {
        let link = self.link.as_mut().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "the daemon is not connected",
            )
        })?;
        link.send(request)?;
        link.receive_matching(id)
    }

    /// Connect, asking systemd exactly once when the socket is missing.
    fn ensure_link(&mut self) -> std::io::Result<()> {
        if self.link.is_some() {
            return Ok(());
        }
        let path = self.options.socket_path.clone();
        let timeout = self.options.response_timeout;
        match Link::connect(&path, timeout) {
            Ok(link) => {
                self.link = Some(link);
                Ok(())
            }
            Err(first) => {
                if self.activated {
                    return Err(first);
                }
                self.activated = true;
                eprintln!(
                    "segmenta-host: socket {} is unavailable; asking systemd to start {SYSTEMD_UNIT} once",
                    path.display()
                );
                if let Err(err) = self.options.activator.activate(&path) {
                    eprintln!(
                        "segmenta-host: `systemctl --user start {SYSTEMD_UNIT}` failed: {}",
                        err.kind()
                    );
                    return Err(first);
                }
                let link = Link::connect(&path, timeout)?;
                self.link = Some(link);
                Ok(())
            }
        }
    }
}

/// A persistent NDJSON connection to the daemon.
struct Link {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Link {
    fn connect(path: &Path, timeout: Duration) -> std::io::Result<Link> {
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let writer = stream.try_clone()?;
        Ok(Link {
            reader: BufReader::new(stream),
            writer,
        })
    }

    /// Send one NDJSON line.
    fn send(&mut self, request: &Value) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(request).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{ERR_FRAME_INVALID_JSON}: {err}"),
            )
        })?;
        line.push(b'\n');
        self.writer.write_all(&line)?;
        self.writer.flush()
    }

    /// Read until the response whose `id` matches; notifications are skipped.
    fn receive_matching(&mut self, id: &Value) -> std::io::Result<Option<Value>> {
        loop {
            let Some(line) = read_line(&mut self.reader)? else {
                return Ok(None);
            };
            let value: Value = serde_json::from_slice(&line).map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{ERR_FRAME_INVALID_JSON}: {err}"),
                )
            })?;
            if is_notification(&value) {
                continue;
            }
            if value.get("id").cloned().unwrap_or(Value::Null) == *id {
                return Ok(Some(value));
            }
            // A response to an earlier request that was never delivered: drop it.
        }
    }
}

/// A JSON-RPC notification carries a method and no id (`download.event`).
fn is_notification(value: &Value) -> bool {
    value.get("method").is_some() && value.get("id").map_or(true, Value::is_null)
}

fn is_recoverable(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::UnexpectedEof
    )
}

/// Read one NDJSON line with the 1 MiB ceiling of the daemon protocol.
fn read_line(reader: &mut BufReader<UnixStream>) -> std::io::Result<Option<Vec<u8>>> {
    let mut buffer = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(if buffer.is_empty() {
                None
            } else {
                Some(buffer)
            });
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.unwrap_or(available.len());
        if buffer.len() + take > crate::framing::MAX_MESSAGE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{ERR_FRAME_TOO_LARGE}: the daemon response exceeds 1 MiB"),
            ));
        }
        buffer.extend_from_slice(&available[..take]);
        match newline {
            Some(_) => {
                reader.consume(take + 1);
                return Ok(Some(buffer));
            }
            None => reader.consume(take),
        }
    }
}

/// Write one frame, degrading to a small error frame if the payload cannot be framed.
fn write_frame<W: Write + ?Sized>(output: &mut W, response: &Value) -> std::io::Result<()> {
    match write_message(output, response) {
        Ok(()) => Ok(()),
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData
            ) =>
        {
            let fallback = error_frame(
                Value::Null,
                INTERNAL_ERROR,
                "INTERNAL_ERROR",
                &format!("{ERR_FRAME_TOO_LARGE}: the response cannot be framed"),
            );
            write_message(output, &fallback)
        }
        Err(err) => Err(err),
    }
}

fn error_frame(id: Value, code: i64, name: &str, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
            "data": { "code": name },
        },
    })
}
