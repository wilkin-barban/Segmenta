//! Blocking JSON-RPC client used by the CLI.
//!
//! The CLI is a short-lived process: a blocking socket, one request at a time and
//! notifications from other tasks skipped while waiting for the matching id.
//!
//! Text is localized at construction time from the catalog: the daemon keeps sending
//! stable numeric codes and the client resolves the human words. Paths, commands, codes
//! and method names are never translated.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use segmenta_i18n::{tr, tr_error, ErrorText, Locale, Msg};
use serde_json::{json, Value};

/// Failure modes of one CLI call.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The daemon socket does not exist or refuses connections.
    ///
    /// `message` is already localized and names the socket path plus the actionable
    /// `systemctl --user enable --now segmenta.socket` command.
    #[error("{message}")]
    Unavailable {
        /// Socket path that was tried.
        path: PathBuf,
        /// Localized text, ready to print.
        message: String,
    },
    /// The daemon answered with a JSON-RPC error.
    ///
    /// `message` is already localized from the stable code and keeps the code visible in
    /// its text; `code` is kept for callers that need to branch on it.
    #[error("{message}")]
    Rpc {
        /// Numeric code.
        code: i64,
        /// Localized text, including the code.
        message: String,
    },
    /// The socket carried something that is not a JSON-RPC frame.
    #[error("{0}")]
    Protocol(String),
    /// A local usage error (bad arguments), already localized.
    #[error("{0}")]
    Usage(String),
    /// Local I/O failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl ClientError {
    /// Localized "the daemon is not reachable" failure.
    ///
    /// The socket path and the systemd command are printed verbatim: they are not human
    /// words and must stay copy-pasteable in every locale.
    pub fn unavailable(locale: Locale, path: &Path, source: std::io::Error) -> ClientError {
        let message = format!(
            "{}: {} ({source}) — {}",
            tr(locale, Msg::ErrorTitle),
            path.display(),
            tr(locale, Msg::ErrorCommand),
        );
        ClientError::Unavailable {
            path: path.to_path_buf(),
            message,
        }
    }

    /// Localized text for a stable JSON-RPC code, keeping the numeric code visible.
    ///
    /// Known codes resolve through [`ErrorText`]. The `-32xxx` bucket maps to
    /// [`ErrorText::Internal`], a catch-all, so its own daemon detail is kept because it is
    /// strictly more specific than "internal error". An unknown code falls back to the
    /// daemon message.
    pub fn rpc(locale: Locale, code: i64, daemon_message: impl Into<String>) -> ClientError {
        let daemon_message = daemon_message.into();
        let message = match ErrorText::from_code(code) {
            Some(ErrorText::Internal) if !daemon_message.is_empty() => format!(
                "{}: {daemon_message} (code {code})",
                tr_error(locale, ErrorText::Internal)
            ),
            Some(error) => format!("{} (code {code})", tr_error(locale, error)),
            None if daemon_message.is_empty() => format!("code {code}"),
            None => format!("{daemon_message} (code {code})"),
        };
        ClientError::Rpc { code, message }
    }
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
    locale: Locale,
}

impl Client {
    /// Connect to `path` using the process locale.
    pub fn connect(path: &Path) -> Result<Client, ClientError> {
        Client::connect_with(path, segmenta_i18n::current())
    }

    /// Connect to `path` and localize every later failure in `locale`.
    pub fn connect_with(path: &Path, locale: Locale) -> Result<Client, ClientError> {
        let stream = UnixStream::connect(path)
            .map_err(|source| ClientError::unavailable(locale, path, source))?;
        stream.set_read_timeout(Some(Duration::from_secs(120)))?;
        let writer = stream.try_clone()?;
        Ok(Client {
            reader: BufReader::new(stream),
            writer,
            next_id: 0,
            locale,
        })
    }

    /// Connect using the default socket path and the process locale.
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
                    tr(self.locale, Msg::ErrDaemonClosed).to_string(),
                ));
            }
            let frame: Value = serde_json::from_str(buffer.trim()).map_err(|err| {
                ClientError::Protocol(
                    tr(self.locale, Msg::ErrProtocol).replace("{detail}", &err.to_string()),
                )
            })?;
            // Notifications (events) do not carry the request id; ignore them.
            if frame.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = frame.get("error") {
                let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32603);
                let message = error.get("message").and_then(Value::as_str).unwrap_or("");
                return Err(ClientError::rpc(self.locale, code, message));
            }
            return Ok(frame.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}
