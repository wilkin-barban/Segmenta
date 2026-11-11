//! JSON-RPC 2.0 message types and the NDJSON framing limits of OpenSpec 07.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// Protocol version negotiated by `system.hello`.
pub const PROTOCOL_VERSION: u32 = 1;
/// Largest accepted message, including the trailing newline.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// One request line.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    /// Must be `"2.0"` when present.
    #[serde(default)]
    pub jsonrpc: Option<String>,
    /// Request identifier echoed back by the response.
    #[serde(default)]
    pub id: Option<Value>,
    /// Method name.
    pub method: String,
    /// Method parameters.
    #[serde(default)]
    pub params: Option<Value>,
}

/// One response line.
#[derive(Debug, Clone, Serialize)]
pub struct Response {
    /// Always `"2.0"`.
    pub jsonrpc: &'static str,
    /// Echoed request identifier; `null` for notifications.
    pub id: Value,
    /// Successful payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Failure payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// Build a success response.
    pub fn success(id: Value, result: Value) -> Response {
        Response {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Build an error response.
    pub fn failure(id: Value, error: RpcError) -> Response {
        Response {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// JSON-RPC error object with the numeric codes of OpenSpec 07 §4.
#[derive(Debug, Clone, Serialize)]
pub struct RpcError {
    /// Numeric code.
    pub code: i64,
    /// Human message.
    pub message: String,
    /// Optional structured detail (`{"code": "ERR_*"}` for engine errors).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    /// Build an error with the identifier duplicated into `data.code`.
    pub fn new(code: i64, name: &str, message: impl Into<String>) -> RpcError {
        RpcError {
            code,
            message: message.into(),
            data: Some(json!({ "code": name })),
        }
    }

    /// `-32700`.
    pub fn parse_error(message: impl Into<String>) -> RpcError {
        RpcError::new(-32700, "PARSE_ERROR", message)
    }

    /// `-32600`.
    pub fn invalid_request(message: impl Into<String>) -> RpcError {
        RpcError::new(-32600, "INVALID_REQUEST", message)
    }

    /// `-32601`.
    pub fn method_not_found(method: &str) -> RpcError {
        RpcError::new(
            -32601,
            "METHOD_NOT_FOUND",
            format!("unknown method `{method}`"),
        )
    }

    /// `-32602`.
    pub fn invalid_params(message: impl Into<String>) -> RpcError {
        RpcError::new(-32602, "INVALID_PARAMS", message)
    }

    /// `-32603`.
    pub fn internal(message: impl Into<String>) -> RpcError {
        RpcError::new(-32603, "INTERNAL_ERROR", message)
    }

    /// `1001`.
    pub fn task_not_found(task_id: &str) -> RpcError {
        RpcError::new(1001, "TASK_NOT_FOUND", format!("no task `{task_id}`"))
    }

    /// `1002`.
    pub fn task_already_active(task_id: &str) -> RpcError {
        RpcError::new(
            1002,
            "TASK_ALREADY_ACTIVE",
            format!("task `{task_id}` is already running"),
        )
    }

    /// `1003`.
    pub fn task_not_resumable(task_id: &str) -> RpcError {
        RpcError::new(
            1003,
            "TASK_NOT_RESUMABLE",
            format!("task `{task_id}` cannot be resumed: the server does not support ranges"),
        )
    }

    /// `1011`.
    pub fn task_not_pausable(task_id: &str) -> RpcError {
        RpcError::new(
            1011,
            "TASK_NOT_PAUSABLE",
            format!("task `{task_id}` is not running and cannot be paused"),
        )
    }

    /// `1003` for a terminal state (the same code covers a non-range server).
    pub fn task_not_resumable_state(task_id: &str, status: &str) -> RpcError {
        RpcError::new(
            1003,
            "TASK_NOT_RESUMABLE",
            format!("task `{task_id}` is {status} and cannot be resumed"),
        )
    }

    /// `1009`.
    pub fn payload_too_large() -> RpcError {
        RpcError::new(
            1009,
            "PAYLOAD_TOO_LARGE",
            format!("message exceeds {MAX_MESSAGE_BYTES} bytes"),
        )
    }

    /// `1010`.
    pub fn unauthorized_peer() -> RpcError {
        RpcError::new(
            1010,
            "UNAUTHORIZED_PEER",
            "peer uid does not match the daemon",
        )
    }

    /// Map an engine error to its numeric code and `ERR_*` name.
    pub fn from_core(err: &segmenta_core::CoreError) -> RpcError {
        RpcError::new(err.code(), err.err_name(), err.to_string())
    }
}

/// Notification envelope for `download.event`.
#[derive(Debug, Clone, Serialize)]
pub struct EventEnvelope {
    /// Monotonic per daemon.
    pub seq_id: u64,
    /// Wall clock in milliseconds.
    pub timestamp_ms: u64,
    /// `task_added`, `state_changed`, `segment_split`, `task_progress`...
    pub event_type: String,
    /// Task this event belongs to, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Event payload.
    pub data: Value,
}

impl EventEnvelope {
    /// Wrap the envelope in a JSON-RPC notification.
    pub fn notification(&self) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "download.event",
            "params": self,
        })
    }
}

/// `system.resync_required` notification, sent when a client lagged behind.
pub fn resync_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "system.resync_required",
        "params": { "reason": "event buffer overflow; reload with download.list" },
    })
}

/// Framing failure while reading a line.
#[derive(Debug)]
pub enum FrameError {
    /// The line exceeded [`MAX_MESSAGE_BYTES`].
    TooLarge,
    /// The socket failed.
    Io(std::io::Error),
}

/// Read one newline-delimited frame, enforcing the 1 MiB ceiling without buffering
/// past it.
pub async fn read_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Vec<u8>>, FrameError> {
    let mut buffer = Vec::new();
    loop {
        let shape = {
            let available = reader.fill_buf().await.map_err(FrameError::Io)?;
            if available.is_empty() {
                None
            } else {
                match available.iter().position(|byte| *byte == b'\n') {
                    Some(position) => Some((position + 1, position)),
                    None => Some((available.len(), available.len())),
                }
            }
        };
        match shape {
            None => {
                return Ok(if buffer.is_empty() {
                    None
                } else {
                    Some(buffer)
                });
            }
            Some((consume, newline)) => {
                {
                    let available = reader.fill_buf().await.map_err(FrameError::Io)?;
                    buffer.extend_from_slice(&available[..newline]);
                }
                reader.consume(consume);
                let complete = newline < consume;
                if complete {
                    if buffer.last() == Some(&b'\r') {
                        buffer.pop();
                    }
                    if buffer.len() > MAX_MESSAGE_BYTES {
                        return Err(FrameError::TooLarge);
                    }
                    return Ok(Some(buffer));
                }
                if buffer.len() > MAX_MESSAGE_BYTES {
                    return Err(FrameError::TooLarge);
                }
            }
        }
    }
}

/// Parameters of `download.add`.
#[derive(Debug, Clone, Deserialize)]
pub struct AddDownloadParams {
    /// HTTP(S) URL to download.
    pub url: String,
    /// Absolute destination directory.
    #[serde(default)]
    pub destination_dir: Option<String>,
    /// Suggested file name.
    #[serde(default)]
    pub filename: Option<String>,
    /// Non-sensitive headers; `Authorization` and `Cookie` are routed to the secret store.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Cookies sent by a browser extension.
    #[serde(default)]
    pub cookies: Vec<CookieParam>,
    /// Connections requested for this task.
    #[serde(default)]
    pub connections: Option<u32>,
    /// Expected digest.
    #[serde(default)]
    pub expected_checksum: Option<ChecksumParam>,
    /// Start as soon as the task exists.
    #[serde(default = "default_true")]
    pub start_immediately: bool,
    /// Queue name.
    #[serde(default = "default_queue")]
    pub queue_name: String,
    /// Higher runs first.
    #[serde(default)]
    pub priority: i64,
}

fn default_true() -> bool {
    true
}

fn default_queue() -> String {
    "main".to_string()
}

/// One cookie forwarded by a browser client.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CookieParam {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Domain the cookie belongs to.
    #[serde(default)]
    pub domain: String,
    /// Path scope.
    #[serde(default = "slash")]
    pub path: String,
}

fn slash() -> String {
    "/".to_string()
}

/// Expected digest of the finished file.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChecksumParam {
    /// Only `sha256` is supported.
    pub algorithm: String,
    /// Hex digest.
    pub hash: String,
}

/// Public view of a task, shared with the CLI and the GUI clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadView {
    /// Task identifier.
    pub task_id: String,
    /// Original URL.
    pub url: String,
    /// Final path on disk.
    pub final_path: String,
    /// Suggested name.
    pub filename: String,
    /// Lifecycle state.
    pub status: String,
    /// Total size when known.
    pub total_bytes: Option<u64>,
    /// Bytes confirmed.
    pub downloaded_bytes: u64,
    /// Current speed.
    pub speed_bps: u64,
    /// Seconds remaining.
    pub eta_seconds: Option<u64>,
    /// Consecutive retries.
    pub retry_count: u32,
    /// Whether the server honoured ranges.
    pub supports_ranges: bool,
    /// Queue name.
    pub queue_name: String,
    /// Creation timestamp.
    pub created_at: Option<String>,
    /// Completion timestamp.
    pub completed_at: Option<String>,
    /// Numeric error code of the last failure.
    pub error_code: Option<i64>,
    /// Message of the last failure.
    pub error_message: Option<String>,
    /// Segments when the caller asked for them.
    #[serde(default)]
    pub segments: Vec<SegmentView>,
}

/// Public view of one segment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentView {
    /// Segment identifier.
    pub id: u32,
    /// First byte.
    pub start_offset: u64,
    /// Last byte, inclusive.
    pub end_offset: u64,
    /// Next byte to write.
    pub next_offset: u64,
    /// `idle`, `active`, `completed` or `failed`.
    pub status: String,
    /// Per-segment speed.
    pub speed_bps: u64,
}

/// Parameters accepted by `download.pause`/`resume`/`status`.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskIdParams {
    /// Target task.
    pub task_id: String,
}

/// Parameters of `download.cancel`.
#[derive(Debug, Clone, Deserialize)]
pub struct CancelParams {
    /// Target task.
    pub task_id: String,
    /// Delete the partial file too.
    #[serde(default)]
    pub delete_file: bool,
}

/// Parameters of `download.delete`.
#[derive(Debug, Clone, Deserialize)]
pub struct DeleteParams {
    /// Target task.
    pub task_id: String,
    /// Delete the file from disk.
    pub delete_file: bool,
}

/// Parameters of `download.list`.
#[derive(Debug, Clone, Deserialize)]
pub struct ListParams {
    /// `all`, `active`, `paused`, `completed` or `failed`.
    #[serde(default)]
    pub filter: Option<String>,
    /// Page size, max 1000.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Page offset.
    #[serde(default)]
    pub offset: Option<usize>,
}

/// Parameters of `queue.schedule`.
#[derive(Debug, Clone, Deserialize)]
pub struct QueueScheduleParams {
    /// Queue to schedule.
    pub queue_name: String,
    /// `HH:MM` or null.
    #[serde(default)]
    pub start_time: Option<String>,
    /// `HH:MM` or null.
    #[serde(default)]
    pub stop_time: Option<String>,
    /// `none`, `suspend`, `power_off` or `exit`.
    #[serde(default = "none_action")]
    pub action_on_complete: String,
}

fn none_action() -> String {
    "none".to_string()
}

/// Parameters of `queue.create`.
#[derive(Debug, Clone, Deserialize)]
pub struct QueueCreateParams {
    /// Queue name.
    pub name: String,
    /// Concurrency.
    #[serde(default = "default_queue_slots")]
    pub max_active: u32,
}

fn default_queue_slots() -> u32 {
    3
}

/// Parameters of `queue.remove`.
#[derive(Debug, Clone, Deserialize)]
pub struct QueueRemoveParams {
    /// Queue name.
    pub name: String,
}

/// Parameters of `system.shutdown`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EmptyParams {
    /// Ignored; present so clients can send `{}`.
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}
