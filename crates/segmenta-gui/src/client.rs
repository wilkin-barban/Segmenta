//! IPC client of the daemon: one blocking `UnixStream` on a worker thread.
//!
//! RED stub: only the contract types and the in-memory fake are implemented.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use segmenta_i18n::{tr, Msg};
use serde_json::{json, Value};

/// Failure of one JSON-RPC call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// The daemon could not be reached (socket missing, refused, closed).
    Unavailable(String),
    /// The daemon answered with a JSON-RPC error object.
    Rpc { code: i64, message: String },
    /// The answer could not be understood.
    Protocol(String),
}

impl ClientError {
    pub fn rpc(code: i64, message: impl Into<String>) -> ClientError {
        ClientError::Rpc {
            code,
            message: message.into(),
        }
    }

    /// Spanish message shown in the UI.
    pub fn ui_message(&self) -> String {
        match self {
            ClientError::Unavailable(detail) => {
                format!("No se pudo conectar con el daemon: {detail}")
            }
            ClientError::Rpc { code, message } => {
                // The stable code is the contract; the text is resolved locally.
                let localized = segmenta_i18n::ErrorText::from_code(*code)
                    .map(|error| segmenta_i18n::tr_error(segmenta_i18n::current(), error));
                match localized {
                    Some(text) => format!("{text} ({code})"),
                    None => tr(segmenta_i18n::current(), Msg::ErrRpc)
                        .replace("{code}", &code.to_string())
                        .replace("{message}", message),
                }
            }
            ClientError::Protocol(detail) => {
                tr(segmenta_i18n::current(), Msg::ErrProtocol).replace("{detail}", detail)
            }
        }
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.ui_message())
    }
}

/// Something the daemon pushed without a request id.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    /// The worker established the socket and completed the handshake.
    Connected,
    /// The worker lost the connection; it keeps retrying.
    Disconnected(String),
    /// Push notification (`download.event`, `system.resync_required`).
    Notification { method: String, params: Value },
}

/// Daemon client used by the UI. Implementations must never block GTK:
/// `request` may block the calling thread (callers use a worker thread) and
/// `poll_event` must return immediately.
pub trait Client: Send + Sync {
    fn request(&self, method: &str, params: Value) -> Result<Value, ClientError>;
    fn poll_event(&self) -> Option<ClientEvent>;
    /// Drop the current connection and try again immediately.
    ///
    /// The default is a no-op so fakes do not need to model reconnection.
    fn reconnect(&self) {}
}

/// In-memory fake of the daemon used by the GUI tests and by `--smoke`.
#[derive(Clone)]
pub struct DemoClient {
    state: Arc<Mutex<DemoState>>,
}

#[derive(Default)]
struct DemoState {
    rows: Vec<Value>,
    events: VecDeque<ClientEvent>,
    error: Option<ClientError>,
    settings: Option<Value>,
    list_calls: usize,
}

impl DemoClient {
    pub fn empty() -> DemoClient {
        DemoClient {
            state: Arc::new(Mutex::new(DemoState {
                settings: Some(default_settings()),
                ..DemoState::default()
            })),
        }
    }

    /// Rows are stored exactly as the daemon sends them: `download.list` drops
    /// `segments`, `download.status` keeps them.
    pub fn with_downloads(rows: Vec<Value>) -> DemoClient {
        let client = DemoClient::empty();
        client.state.lock().unwrap().rows = rows;
        client
    }

    /// Every request fails with `ClientError::Unavailable`.
    pub fn failing(message: &str) -> DemoClient {
        let client = DemoClient::empty();
        client.state.lock().unwrap().error = Some(ClientError::Unavailable(message.to_string()));
        client
    }

    pub fn push_notification(&self, method: &str, params: Value) {
        self.state
            .lock()
            .unwrap()
            .events
            .push_back(ClientEvent::Notification {
                method: method.to_string(),
                params,
            });
    }

    pub fn push_download_event(&self, seq_id: u64, event_type: &str, task_id: &str, data: Value) {
        self.push_notification(
            "download.event",
            json!({
                "seq_id": seq_id,
                "timestamp_ms": 1_776_000_000_000u64,
                "event_type": event_type,
                "task_id": task_id,
                "data": data
            }),
        );
    }

    /// Number of `download.list` calls served, used to assert resyncs.
    pub fn list_calls(&self) -> usize {
        self.state.lock().unwrap().list_calls
    }

    pub fn set_settings(&self, settings: Value) {
        self.state.lock().unwrap().settings = Some(settings);
    }
}

fn default_settings() -> Value {
    json!({
        "max_connections": 16,
        "global_speed_limit_bps": null,
        "clip_monitor_enabled": true,
        "autostart": false,
        "download_dir": "/home/user/Descargas",
        "category_dirs": {},
        "capture_extensions": ["iso", "zip"],
        "domain_denylist": [],
        "schedule_start": null,
        "schedule_stop": null,
        "action_on_complete": "none",
        "allow_local_downloads": false,
        "max_retries": 5,
        "proxy_mode": "system",
        "proxy_url": null,
        "postprocess_command": []
    })
}

impl Client for DemoClient {
    fn request(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        let mut state = self.state.lock().unwrap();
        if let Some(error) = state.error.clone() {
            return Err(error);
        }
        match method {
            "system.hello" => Ok(json!({
                "protocol_version": 1,
                "daemon_version": "0.0.1",
                "min_client_version": "0.0.1"
            })),
            "system.status" => Ok(json!({
                "daemon_version": "0.0.1",
                "uptime_seconds": 12u64,
                "memory_bytes": 1024u64,
                "active_connections": 4u64,
                "event_subscribers": 1u64,
                "secret_backend": "file"
            })),
            "download.list" => {
                state.list_calls += 1;
                let mut rows = Vec::with_capacity(state.rows.len());
                for row in &state.rows {
                    let mut row = row.clone();
                    row["segments"] = json!([]);
                    rows.push(row);
                }
                Ok(Value::Array(rows))
            }
            "download.status" => {
                let task_id = params
                    .get("task_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                state
                    .rows
                    .iter()
                    .find(|row| row["task_id"].as_str() == Some(task_id))
                    .cloned()
                    .ok_or_else(|| ClientError::rpc(1001, format!("no task `{task_id}`")))
            }
            "download.pause" => Self::set_status(&mut state, &params, "paused"),
            "download.resume" => Self::set_status(&mut state, &params, "downloading"),
            "download.cancel" => Self::set_status(&mut state, &params, "cancelled"),
            "download.delete" => {
                let task_id = params
                    .get("task_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                state
                    .rows
                    .retain(|row| row["task_id"].as_str() != Some(&task_id));
                Ok(json!({ "task_id": task_id, "deleted": true }))
            }
            "download.add" => {
                let url = params
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let filename = params
                    .get("filename")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| "download.bin".to_string());
                let mut row: Value = json!({
                    "task_id": format!("task-{}", state.rows.len() + 1),
                    "url": url,
                    "final_path": filename,
                    "filename": filename,
                    "status": "downloading",
                    "total_bytes": null,
                    "downloaded_bytes": 0u64,
                    "speed_bps": 0u64,
                    "eta_seconds": null,
                    "retry_count": 0u32,
                    "supports_ranges": true,
                    "queue_name": "main",
                    "created_at": null,
                    "completed_at": null,
                    "error_code": null,
                    "error_message": null,
                    "segments": []
                });
                let task_id = row["task_id"].as_str().unwrap_or_default().to_string();
                state.rows.push(row.clone());
                row["segments"] = json!([]);
                Ok(json!({
                    "task_id": task_id,
                    "status": "downloading",
                    "filename": row["filename"]
                }))
            }
            "settings.get" => Ok(state.settings.clone().unwrap_or_else(default_settings)),
            "extension.status" => Ok(json!({
                "extension_id": "kdeldnifdbpemnhfpmhonlnibbhnoaan",
                "firefox_extension_id": "segmenta@wilkin.github.io",
                "host_path": "/usr/bin/segmenta-host",
                "extension_version": "0.1.2",
                "shipped_archives": {
                    "chromium": "/usr/share/segmenta/extensions/segmenta-chromium.zip",
                    "firefox": "/usr/share/segmenta/extensions/segmenta-firefox.zip"
                },
                "browsers": [
                    {
                        "id": "chrome",
                        "name": "Google Chrome",
                        "family": "chromium",
                        "command": "/usr/bin/google-chrome",
                        "version": "Chrome 120.0",
                        "unsupported": null,
                        "manifest_path": "/etc/opt/chrome/native-messaging-hosts/io.github.wilkin.segmenta.json",
                        "manifest_present": true,
                        "user_manifest_present": false,
                        "extension_installed": false,
                        "prepared_path": "/home/user/.local/share/segmenta/extensions/chromium-0.1.2",
                        "archive_path": "/usr/share/segmenta/extensions/segmenta-chromium.zip",
                        "state": "native-ready"
                    },
                    {
                        "id": "firefox",
                        "name": "Firefox",
                        "family": "firefox",
                        "command": "/usr/bin/firefox",
                        "version": "Firefox 120.0",
                        "unsupported": null,
                        "manifest_path": "/usr/lib/mozilla/native-messaging-hosts/io.github.wilkin.segmenta.json",
                        "manifest_present": true,
                        "user_manifest_present": false,
                        "extension_installed": false,
                        "prepared_path": "/home/user/.local/share/segmenta/extensions/firefox-0.1.2",
                        "archive_path": "/usr/share/segmenta/extensions/segmenta-firefox.zip",
                        "state": "native-ready"
                    }
                ]
            })),
            "extension.prepare" => Ok(json!({
                "prepared": [
                    { "family": "chromium", "path": "/home/user/.local/share/segmenta/extensions/chromium-0.1.2" },
                    { "family": "firefox", "path": "/home/user/.local/share/segmenta/extensions/firefox-0.1.2" }
                ]
            })),
            "extension.install-native" => Ok(json!({
                "installed": [
                    { "id": "chrome", "path": "/home/user/.config/google-chrome/NativeMessagingHosts/io.github.wilkin.segmenta.json" }
                ],
                "errors": []
            })),
            "extension.open" => Ok(json!({ "opened": true })),
            "settings.update" => {
                let mut settings = state.settings.clone().unwrap_or_else(default_settings);
                if let (Some(target), Some(patch)) = (settings.as_object_mut(), params.as_object())
                {
                    for (key, value) in patch {
                        if !target.contains_key(key) {
                            return Err(ClientError::rpc(
                                -32602,
                                format!("unknown setting `{key}`"),
                            ));
                        }
                        target.insert(key.clone(), value.clone());
                    }
                }
                state.settings = Some(settings.clone());
                Ok(settings)
            }
            other => Err(ClientError::rpc(
                -32601,
                format!("unknown method `{other}`"),
            )),
        }
    }

    fn poll_event(&self) -> Option<ClientEvent> {
        self.state.lock().unwrap().events.pop_front()
    }
}

impl DemoClient {
    fn set_status(
        state: &mut DemoState,
        params: &Value,
        status: &str,
    ) -> Result<Value, ClientError> {
        let task_id = params
            .get("task_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let row = state
            .rows
            .iter_mut()
            .find(|row| row["task_id"].as_str() == Some(task_id))
            .ok_or_else(|| ClientError::rpc(1001, format!("no task `{task_id}`")))?;
        row["status"] = json!(status);
        Ok(json!({ "task_id": task_id, "status": status }))
    }
}

/// Environment variable that overrides the daemon socket path.
pub const SOCKET_ENV: &str = "SEGMENTA_SOCKET";

/// Largest line the client accepts, matching OpenSpec 07.
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// How long a single call may wait for its answer before giving up.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Delay between connection attempts while the daemon is down.
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Delay between worker polls.
const WORKER_TICK: std::time::Duration = std::time::Duration::from_millis(50);
/// Verbose diagnostics for the connection path (`SEGMENTA_GUI_LOG=1`).
pub fn trace_enabled() -> bool {
    std::env::var_os("SEGMENTA_GUI_LOG").is_some_and(|value| value != "0")
}

/// How long a queued job waits for the first connection before the UI gets an error.
const CONNECT_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);
/// Bound of the pre-connection queue.
const PENDING_LIMIT: usize = 256;

/// `$SEGMENTA_SOCKET`, else `$XDG_RUNTIME_DIR/segmenta/daemon.sock`.
pub fn socket_path() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os(SOCKET_ENV) {
        return std::path::PathBuf::from(path);
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/run/user/0"));
    runtime.join("segmenta").join("daemon.sock")
}

/// One request handed to the worker thread.
struct Job {
    method: String,
    params: Value,
    reply: std::sync::mpsc::Sender<Result<Value, ClientError>>,
}

/// Blocking NDJSON JSON-RPC 2.0 client with one dedicated worker thread.
///
/// The UI never touches the socket: `request` is meant to be called from a
/// spawned thread and `poll_event` reads an in-memory queue.
pub struct UnixClient {
    jobs: std::sync::mpsc::Sender<Job>,
    control: std::sync::mpsc::Sender<()>,
    events: Arc<Mutex<VecDeque<ClientEvent>>>,
    path: std::path::PathBuf,
}

impl UnixClient {
    /// Connects to `path` (retrying in the background) and returns the handle.
    pub fn connect(path: std::path::PathBuf) -> UnixClient {
        let (jobs, job_rx) = std::sync::mpsc::channel::<Job>();
        let (control, control_rx) = std::sync::mpsc::channel::<()>();
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let worker = Worker {
            path: path.clone(),
            stream: None,
            buffer: Vec::new(),
            jobs: job_rx,
            control: control_rx,
            pending: VecDeque::new(),
            pending_since: None,
            events: Arc::clone(&events),
            next_id: 0,
            last_error: None,
            last_attempt: None,
        };
        std::thread::Builder::new()
            .name("segmenta-ipc".to_string())
            .spawn(move || worker.run())
            .ok();
        UnixClient {
            jobs,
            control,
            events,
            path,
        }
    }

    /// Connects to the path from the environment.
    pub fn from_env() -> UnixClient {
        UnixClient::connect(socket_path())
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Client for UnixClient {
    fn request(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        let (reply, answer) = std::sync::mpsc::channel();
        let job = Job {
            method: method.to_string(),
            params,
            reply,
        };
        if self.jobs.send(job).is_err() {
            return Err(ClientError::Unavailable(
                tr(segmenta_i18n::current(), Msg::ErrThreadUnavailable).to_string(),
            ));
        }
        match answer.recv_timeout(REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(_) => Err(ClientError::Unavailable(
                tr(segmenta_i18n::current(), Msg::ErrTimeout).to_string(),
            )),
        }
    }

    fn poll_event(&self) -> Option<ClientEvent> {
        self.events
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .pop_front()
    }

    fn reconnect(&self) {
        // The worker drops the stream and reconnects on its next tick.
        let _ = self.control.send(());
    }
}

/// State owned by the worker thread.
struct Worker {
    path: std::path::PathBuf,
    stream: Option<std::os::unix::net::UnixStream>,
    buffer: Vec<u8>,
    jobs: std::sync::mpsc::Receiver<Job>,
    control: std::sync::mpsc::Receiver<()>,
    /// Jobs received before the socket existed. They are served as soon as the
    /// connection (and its `system.hello`) succeeds, instead of being rejected.
    pending: VecDeque<Job>,
    /// When the oldest queued job arrived; the grace is measured from here, not from
    /// the last connection attempt (which repeats every 50 ms while work waits).
    pending_since: Option<std::time::Instant>,
    events: Arc<Mutex<VecDeque<ClientEvent>>>,
    next_id: u64,
    last_error: Option<String>,
    last_attempt: Option<std::time::Instant>,
}

impl Worker {
    fn run(mut self) {
        loop {
            if self.control.try_recv().is_ok() {
                self.stream = None;
                self.buffer.clear();
                self.last_attempt = None;
            }
            if self.stream.is_none() {
                self.try_connect();
            }

            loop {
                match self.jobs.try_recv() {
                    Ok(job) => self.serve(job),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                }
            }

            if self.stream.is_some() {
                match self.drain_notifications() {
                    Ok(()) => {}
                    Err(err) => self.drop_connection(err),
                }
            } else {
                self.fail_pending_after_grace();
            }

            std::thread::sleep(WORKER_TICK);
        }
    }

    /// Answer the oldest queued job with the connection error once the grace period
    /// expires, so a dead daemon produces a fast, actionable error instead of a hang.
    fn fail_pending_after_grace(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let waited = self
            .pending_since
            .map(|since| since.elapsed())
            .unwrap_or_default();
        if waited < CONNECT_GRACE {
            return;
        }
        let detail = self
            .last_error
            .clone()
            .unwrap_or_else(|| format!("{}: not connected", self.path.display()));
        let jobs = std::mem::take(&mut self.pending);
        self.pending_since = None;
        for job in jobs {
            let _ = job
                .reply
                .send(Err(ClientError::Unavailable(detail.clone())));
        }
    }

    fn emit(&self, event: ClientEvent) {
        self.events
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push_back(event);
    }

    fn report_disconnect(&mut self, detail: String) {
        if self.last_error.as_deref() != Some(detail.as_str()) {
            // The UI shows the actionable state; stderr keeps the diagnostic for the
            // operator (`systemctl --user enable --now segmenta.socket`).
            eprintln!(
                "segmenta-gui: ipc connection to {} failed: {detail}",
                self.path.display()
            );
            self.last_error = Some(detail.clone());
            self.emit(ClientEvent::Disconnected(detail));
        }
    }

    fn drop_connection(&mut self, detail: String) {
        self.stream = None;
        self.buffer.clear();
        self.report_disconnect(detail);
    }

    fn try_connect(&mut self) {
        if let Some(last) = self.last_attempt {
            // Queued work must not wait for the 2 s backoff.
            let delay = if self.pending.is_empty() {
                RECONNECT_DELAY
            } else {
                std::time::Duration::from_millis(50)
            };
            if last.elapsed() < delay {
                return;
            }
        }
        self.last_attempt = Some(std::time::Instant::now());
        let stream = match std::os::unix::net::UnixStream::connect(&self.path) {
            Ok(stream) => stream,
            Err(err) => {
                self.report_disconnect(format!("{}: {err}", self.path.display()));
                return;
            }
        };
        if let Err(err) = stream.set_nonblocking(true) {
            self.report_disconnect(format!("socket no bloqueante: {err}"));
            return;
        }
        self.stream = Some(stream);
        self.buffer.clear();
        match self.call("system.hello", json!({})) {
            Ok(_) => {
                self.last_error = None;
                self.emit(ClientEvent::Connected);
                // Serve whatever the UI asked for before the socket existed.
                let queued = std::mem::take(&mut self.pending);
                self.pending_since = None;
                for job in queued {
                    self.serve(job);
                }
            }
            Err(err) => self.drop_connection(err.ui_message()),
        }
    }

    fn serve(&mut self, job: Job) {
        if trace_enabled() {
            eprintln!(
                "segmenta-gui: serve {} (connected={})",
                job.method,
                self.stream.is_some()
            );
        }
        if self.stream.is_none() {
            // Queue instead of failing: the UI issues `download.list` before the first
            // connection can possibly succeed (socket activation, slow hello).
            if self.pending.len() >= PENDING_LIMIT {
                if let Some(dropped) = self.pending.pop_front() {
                    let _ = dropped.reply.send(Err(ClientError::Unavailable(
                        "demasiadas peticiones en espera".to_string(),
                    )));
                }
            }
            if self.pending_since.is_none() {
                self.pending_since = Some(std::time::Instant::now());
            }
            self.pending.push_back(job);
            return;
        }
        let result = self.call(&job.method, job.params);
        if trace_enabled() {
            match &result {
                Ok(value) => eprintln!(
                    "segmenta-gui: {} ok ({} bytes)",
                    job.method,
                    value.to_string().len()
                ),
                Err(err) => eprintln!("segmenta-gui: {} failed: {}", job.method, err.ui_message()),
            }
        }
        if let Err(ClientError::Unavailable(detail)) = &result {
            let detail = detail.clone();
            self.drop_connection(detail);
        }
        let _ = job.reply.send(result);
    }

    /// Sends one request and waits for the answer with the same id.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        let mut line = request.to_string();
        line.push('\n');
        self.write_all(line.as_bytes())
            .map_err(|err| ClientError::Unavailable(err.to_string()))?;

        let deadline = std::time::Instant::now() + REQUEST_TIMEOUT;
        loop {
            let messages = self
                .read_messages()
                .map_err(|err| ClientError::Unavailable(err.to_string()))?;
            for message in messages {
                if let Some(answer) = response_id(&message) {
                    if answer == json!(id) {
                        return decode_response(message);
                    }
                    // A stale answer for an abandoned call: ignore it.
                    continue;
                }
                self.handle_notification(message);
            }
            if std::time::Instant::now() >= deadline {
                return Err(ClientError::Unavailable(
                    tr(segmenta_i18n::current(), Msg::ErrTimeout).to_string(),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Reads whatever is buffered and forwards push notifications.
    fn drain_notifications(&mut self) -> Result<(), String> {
        let messages = self.read_messages().map_err(|err| err.to_string())?;
        for message in messages {
            if response_id(&message).is_some() {
                continue;
            }
            self.handle_notification(message);
        }
        Ok(())
    }

    fn handle_notification(&mut self, message: Value) {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return;
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        self.emit(ClientEvent::Notification {
            method: method.to_string(),
            params,
        });
    }

    fn write_all(&mut self, mut data: &[u8]) -> std::io::Result<()> {
        let Some(stream) = self.stream.as_mut() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "socket cerrado",
            ));
        };
        while !data.is_empty() {
            match std::io::Write::write(stream, data) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        tr(segmenta_i18n::current(), Msg::ErrDaemonClosed),
                    ))
                }
                Ok(written) => data = &data[written..],
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    /// Appends everything the socket has and returns the complete lines.
    fn read_messages(&mut self) -> std::io::Result<Vec<Value>> {
        let Some(stream) = self.stream.as_mut() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "socket cerrado",
            ));
        };
        let mut chunk = [0u8; 8192];
        loop {
            match std::io::Read::read(stream, &mut chunk) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        tr(segmenta_i18n::current(), Msg::ErrDaemonClosed),
                    ))
                }
                Ok(read) => self.buffer.extend_from_slice(&chunk[..read]),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
        if self.buffer.len() > MAX_MESSAGE_BYTES * 4 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                tr(segmenta_i18n::current(), Msg::ErrLineTooLarge),
            ));
        }

        let mut messages = Vec::new();
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=position).collect();
            let text = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
            if text.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(&text) {
                Ok(message) => messages.push(message),
                Err(err) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("respuesta inválida: {err}"),
                    ))
                }
            }
        }
        Ok(messages)
    }
}

fn response_id(message: &Value) -> Option<Value> {
    if message.get("method").is_some() {
        return None;
    }
    match message.get("id") {
        Some(Value::Null) | None => None,
        Some(id) => Some(id.clone()),
    }
}

fn decode_response(message: Value) -> Result<Value, ClientError> {
    if let Some(error) = message.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32603);
        let text = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("error sin mensaje")
            .to_string();
        return Err(ClientError::rpc(code, text));
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| ClientError::Protocol("la respuesta no trae `result`".to_string()))
}

/// One finished call: bus id plus the answer.
type FinishedRequest = (u64, Result<Value, ClientError>);

/// Mailbox of finished background requests, drained by the UI tick.
///
/// The window keeps `Rc` state that cannot cross threads, so worker threads
/// publish their results here and the GTK main loop consumes them.
#[derive(Clone, Default)]
pub struct RequestBus {
    results: Arc<Mutex<Vec<FinishedRequest>>>,
    next_id: Arc<std::sync::atomic::AtomicU64>,
}

impl RequestBus {
    pub fn new() -> RequestBus {
        RequestBus::default()
    }

    /// Spawns one worker thread that performs the call and publishes the result.
    pub fn spawn(&self, client: &Arc<dyn Client>, method: &str, params: Value) -> u64 {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let results = Arc::clone(&self.results);
        let client = Arc::clone(client);
        let method = method.to_string();
        std::thread::spawn(move || {
            let result = client.request(&method, params);
            results
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push((id, result));
        });
        id
    }

    /// Drains the finished calls.
    pub fn take(&self) -> Vec<FinishedRequest> {
        std::mem::take(&mut *self.results.lock().unwrap_or_else(|err| err.into_inner()))
    }
}
