//! Unix-socket IPC server: 0600 socket, `SO_PEERCRED` check, NDJSON framing and the
//! event bus with monotonic `seq_id` (OpenSpec 07 §1, §2, §5).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

use crate::events::EventBus;
use crate::rpc::{read_frame, resync_notification, FrameError, Request, Response, RpcError};

/// Capacity of the per-client event buffer. A client that falls further behind gets
/// `system.resync_required` instead of stale events.
pub const EVENT_BUFFER: usize = 1024;

/// Anything that can answer a JSON-RPC method.
pub trait RpcHandler: Send + Sync {
    /// Handle one method call.
    fn call<'a>(
        &'a self,
        method: &'a str,
        params: Option<Value>,
    ) -> BoxFuture<'a, Result<Value, RpcError>>;
}

/// The listening socket plus the event bus.
pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
    bus: Arc<EventBus>,
    owns_socket: bool,
    started: Instant,
}

impl IpcServer {
    /// Bind the socket, creating `$XDG_RUNTIME_DIR/segmenta` with `0700` and the socket
    /// with `0600`.
    pub async fn bind(path: &Path, bus: Arc<EventBus>) -> std::io::Result<IpcServer> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            set_mode(parent, 0o700)?;
        }
        // A stale socket from a crashed daemon must not block the bind.
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
        let listener = UnixListener::bind(path)?;
        set_mode(path, 0o600)?;
        Ok(IpcServer {
            listener,
            path: path.to_path_buf(),
            bus,
            owns_socket: true,
            started: Instant::now(),
        })
    }

    /// Adopt the listening descriptor passed by systemd socket activation.
    ///
    /// SAFETY contract: `LISTEN_FDS == 1` and `LISTEN_PID` matches this process, which
    /// [`systemd_listen_fd`] checks before this function is called.
    pub fn from_systemd(
        fd: std::os::unix::io::RawFd,
        path: &Path,
        bus: Arc<EventBus>,
    ) -> std::io::Result<IpcServer> {
        use std::os::unix::io::FromRawFd;
        // SAFETY: the caller guarantees the descriptor is a listening Unix socket owned
        // by this process after the fork from systemd.
        let std_listener = unsafe { std::os::unix::net::UnixListener::from_raw_fd(fd) };
        std_listener.set_nonblocking(true)?;
        Ok(IpcServer {
            listener: UnixListener::from_std(std_listener)?,
            path: path.to_path_buf(),
            bus,
            // systemd owns the inode; the daemon must not unlink it.
            owns_socket: false,
            started: Instant::now(),
        })
    }

    /// Socket path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Seconds since the socket was bound.
    pub fn uptime_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// Shared event bus.
    pub fn bus(&self) -> Arc<EventBus> {
        Arc::clone(&self.bus)
    }

    /// Accept connections until the process ends.
    pub async fn run(self: Arc<Self>, handler: Arc<dyn RpcHandler>, expected_uid: u32) {
        loop {
            let accepted = self.listener.accept().await;
            let Ok((stream, _addr)) = accepted else {
                continue;
            };
            let peer_uid = match stream.peer_cred() {
                Ok(cred) => cred.uid(),
                Err(_) => continue,
            };
            if !peer_is_authorized(peer_uid, expected_uid) {
                tracing::warn!(peer_uid, "rejected a client with a different uid");
                // Drop the socket without reading or writing a single byte.
                drop(stream);
                continue;
            }
            tracing::debug!(peer_uid, "ipc client connected");
            let handler = Arc::clone(&handler);
            let bus = Arc::clone(&self.bus);
            tokio::spawn(async move {
                let _ = handle_connection(stream, handler, bus).await;
            });
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if self.owns_socket {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// `LISTEN_FDS`/`LISTEN_PID` from systemd, if this process was socket-activated.
pub fn systemd_listen_fd() -> Option<std::os::unix::io::RawFd> {
    let fds: i32 = std::env::var("LISTEN_FDS").ok()?.parse().ok()?;
    let pid: u32 = std::env::var("LISTEN_PID").ok()?.parse().ok()?;
    if fds != 1 || pid != std::process::id() {
        return None;
    }
    Some(3)
}

/// UID check of `SO_PEERCRED`, isolated so it can be tested without a second user.
pub fn peer_is_authorized(peer_uid: u32, daemon_uid: u32) -> bool {
    peer_uid == daemon_uid
}

async fn handle_connection(
    stream: UnixStream,
    handler: Arc<dyn RpcHandler>,
    bus: Arc<EventBus>,
) -> std::io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut events_rx = bus.subscribe();
    loop {
        tokio::select! {
            frame = read_frame(&mut reader) => {
                match frame {
                    Ok(None) => return Ok(()),
                    Ok(Some(bytes)) => {
                        let started = Instant::now();
                        let (response, keep_open) = dispatch_bytes(&bytes, handler.as_ref()).await;
                        if let Some(error) = response.get("error") {
                            tracing::warn!(
                                error_code = error.get("code").and_then(|code| code.as_i64()),
                                duration_ms = started.elapsed().as_millis() as u64,
                                "ipc request failed"
                            );
                        } else {
                            tracing::debug!(
                                duration_ms = started.elapsed().as_millis() as u64,
                                "ipc request"
                            );
                        }
                        if !response.is_null() {
                            write_json(&mut write_half, &response).await?;
                        }
                        if !keep_open {
                            return Ok(());
                        }
                    }
                    Err(FrameError::TooLarge) => {
                        let response = serde_json::to_value(Response::failure(
                            Value::Null,
                            RpcError::payload_too_large(),
                        ))
                        .unwrap_or(Value::Null);
                        write_json(&mut write_half, &response).await?;
                        return Ok(());
                    }
                    Err(FrameError::Io(err)) => return Err(err),
                }
            }
            event = events_rx.recv() => {
                match event {
                    Ok(envelope) => {
                        write_json(&mut write_half, &envelope.notification()).await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        write_json(&mut write_half, &resync_notification()).await?;
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
    }
}

async fn write_json(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    value: &Value,
) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value).unwrap_or_default();
    line.push(b'\n');
    writer.write_all(&line).await
}

/// Parse, validate and dispatch one frame. Returns the response to send and whether the
/// connection stays open.
pub async fn dispatch_bytes(bytes: &[u8], handler: &dyn RpcHandler) -> (Value, bool) {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text.trim(),
        Err(_) => {
            return (
                serde_json::to_value(Response::failure(
                    Value::Null,
                    RpcError::parse_error("frame is not valid UTF-8"),
                ))
                .unwrap_or(Value::Null),
                true,
            );
        }
    };
    if text.is_empty() {
        // A blank line is not a message; ignore it like any other empty frame.
        return (Value::Null, true);
    }
    let request: Request = match serde_json::from_str(text) {
        Ok(request) => request,
        Err(err) => {
            return (
                serde_json::to_value(Response::failure(
                    Value::Null,
                    RpcError::parse_error(err.to_string()),
                ))
                .unwrap_or(Value::Null),
                true,
            );
        }
    };
    dispatch_request(request, handler).await
}

/// Dispatch an already parsed request.
pub async fn dispatch_request(request: Request, handler: &dyn RpcHandler) -> (Value, bool) {
    let id = request.id.clone().unwrap_or(Value::Null);
    if request.jsonrpc.as_deref() != Some("2.0") {
        return (
            serde_json::to_value(Response::failure(
                id,
                RpcError::invalid_request("jsonrpc must be \"2.0\""),
            ))
            .unwrap_or(Value::Null),
            true,
        );
    }
    let response = match handler.call(&request.method, request.params).await {
        Ok(result) => Response::success(id, result),
        Err(error) => Response::failure(id, error),
    };
    (serde_json::to_value(response).unwrap_or(Value::Null), true)
}

/// `system.hello` response shared by the manager and the tests.
pub fn hello_result(daemon_version: &str) -> Value {
    json!({
        "protocol_version": crate::rpc::PROTOCOL_VERSION,
        "daemon_version": daemon_version,
        "min_client_version": 1,
    })
}

fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
}
