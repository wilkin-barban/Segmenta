//! Task orchestration: turns JSON-RPC calls into engine runs, bridges engine events to
//! the event bus and persists checkpoints in the order OpenSpec 08 mandates.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

use segmenta_core::engine::{
    run as run_engine, Canceller, DownloadSpec, EngineConfig, EngineEvent, ResumeState,
    SegmentStateRecord, TaskOutcome,
};
use segmenta_core::filename::{resolve_unique_path, sanitize_filename, suggest_filename};
use segmenta_core::probe::HttpConfig;
use segmenta_core::{planner, CoreError};

use crate::events::EventBus;
use crate::ipc::{hello_result, RpcHandler};
use crate::rpc::{
    AddDownloadParams, CancelParams, ChecksumParam, DeleteParams, DownloadView, ListParams,
    MediaAddParams, MediaProbeParams, MediaStatusParams, QueueCreateParams, QueueRemoveParams,
    QueueScheduleParams, RpcError, SegmentView, TaskIdParams,
};
use crate::secrets::SecretStore;
use crate::store::{
    Checkpoint, DownloadRecord, DownloadStatus, SegmentRow, Settings, Store, StoreError,
};

/// Live numbers of a running task, kept out of the database until a checkpoint.
#[derive(Debug, Clone, Default)]
pub struct ProgressSnapshot {
    /// Bytes written so far.
    pub downloaded_bytes: u64,
    /// Total when known.
    pub total_bytes: Option<u64>,
    /// Current speed.
    pub speed_bps: u64,
    /// Seconds remaining.
    pub eta_seconds: Option<u64>,
    /// Segment table as last reported.
    pub segments: Vec<SegmentRow>,
}

struct RunningTask {
    cancel: Canceller,
    handle: tokio::task::JoinHandle<()>,
}

/// Coordinates the store, the engine and the IPC layer.
pub struct Manager {
    store: Arc<Store>,
    secrets: Arc<dyn SecretStore>,
    bus: Arc<EventBus>,
    settings: Mutex<Settings>,
    progress: Arc<Mutex<HashMap<String, ProgressSnapshot>>>,
    running: Arc<Mutex<HashMap<String, RunningTask>>>,
    shutdown: watch::Sender<bool>,
    started: std::time::Instant,
}

impl Manager {
    /// Build a manager over an open store.
    pub fn new(
        store: Arc<Store>,
        secrets: Arc<dyn SecretStore>,
        bus: Arc<EventBus>,
        shutdown: watch::Sender<bool>,
    ) -> Manager {
        let settings = store.settings().unwrap_or_default();
        Manager {
            store,
            secrets,
            bus,
            settings: Mutex::new(settings),
            progress: Arc::new(Mutex::new(HashMap::new())),
            running: Arc::new(Mutex::new(HashMap::new())),
            shutdown,
            started: std::time::Instant::now(),
        }
    }

    /// Reconcile tasks that were in flight when the daemon died.
    pub fn recover(&self) {
        match self.store.recover_crashed() {
            Ok(ids) => {
                for id in ids {
                    self.bus.publish(
                        "state_changed",
                        Some(&id),
                        json!({ "old_state": "downloading", "new_state": "paused", "reason": "daemon_restart" }),
                    );
                }
            }
            Err(err) => eprintln!("segmenta-daemon: recovery failed: {err}"),
        }
    }

    fn settings(&self) -> Settings {
        self.settings
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Tasks the daemon is currently running.
    pub fn running_count(&self) -> usize {
        self.running
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .len()
    }

    /// Build the public view of a record, enriched with live progress.
    fn view(&self, record: &DownloadRecord, with_segments: bool) -> DownloadView {
        let progress = self
            .progress
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(&record.id)
            .cloned();
        let snapshot = progress.unwrap_or_default();
        let segments = if with_segments {
            if snapshot.segments.is_empty() {
                self.store
                    .segments(&record.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|segment| SegmentView {
                        id: segment.id,
                        start_offset: segment.start,
                        end_offset: segment.end,
                        next_offset: segment.next,
                        status: if segment.next > segment.end {
                            "completed".to_string()
                        } else if record.status == DownloadStatus::Downloading {
                            "active".to_string()
                        } else {
                            "idle".to_string()
                        },
                        speed_bps: 0,
                    })
                    .collect()
            } else {
                snapshot
                    .segments
                    .iter()
                    .map(|segment| SegmentView {
                        id: segment.id,
                        start_offset: segment.start,
                        end_offset: segment.end,
                        next_offset: segment.next,
                        status: if segment.next > segment.end {
                            "completed".to_string()
                        } else if record.status == DownloadStatus::Downloading {
                            "active".to_string()
                        } else {
                            "idle".to_string()
                        },
                        speed_bps: 0,
                    })
                    .collect()
            }
        } else {
            Vec::new()
        };
        DownloadView {
            task_id: record.id.clone(),
            url: record.url.clone(),
            final_path: record.final_path.clone(),
            filename: record.filename.clone(),
            status: record.status.as_str().to_string(),
            total_bytes: snapshot.total_bytes.or(record.total_bytes),
            downloaded_bytes: if snapshot.downloaded_bytes > 0 {
                snapshot.downloaded_bytes
            } else {
                record.downloaded_bytes
            },
            speed_bps: snapshot.speed_bps,
            eta_seconds: snapshot.eta_seconds,
            retry_count: record.retry_count,
            supports_ranges: record.supports_ranges,
            queue_name: record.queue_name.clone(),
            created_at: record.created_at.clone(),
            completed_at: record.completed_at.clone(),
            error_code: record.error_code,
            error_message: record.error_message.clone(),
            segments,
            media_kind: record.media_kind.clone(),
            playlist_title: record.playlist_title.clone(),
        }
    }

    fn publish_state(&self, id: &str, old: &str, new: &str, extra: Option<Value>) {
        let mut data = json!({ "old_state": old, "new_state": new });
        if let Some(Value::Object(extra)) = extra {
            if let Value::Object(map) = &mut data {
                for (key, value) in extra {
                    map.insert(key, value);
                }
            }
        }
        self.bus.publish("state_changed", Some(id), data);
    }

    /// `download.add`
    fn add_download(&self, params: AddDownloadParams) -> Result<Value, RpcError> {
        let url = url::Url::parse(&params.url)
            .map_err(|err| RpcError::invalid_params(format!("invalid url: {err}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RpcError::new(
                1004,
                "ERR_INVALID_URL",
                "only http and https URLs are accepted",
            ));
        }
        let settings = self.settings();
        let connections = params
            .connections
            .unwrap_or(settings.max_connections)
            .clamp(1, 32);
        let destination_dir = params
            .destination_dir
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&settings.download_dir));
        if !destination_dir.is_absolute() {
            return Err(RpcError::invalid_params(
                "destination_dir must be an absolute path",
            ));
        }
        let suggested = params
            .filename
            .as_deref()
            .and_then(sanitize_filename)
            .or_else(|| suggest_filename(&url))
            .unwrap_or_else(|| "download.bin".to_string());
        let task_id = uuid::Uuid::new_v4().to_string();
        tracing::info!(
            task_id = %task_id,
            url = %crate::logging::sanitize_url(url.as_str()),
            connections,
            queue = %params.queue_name,
            "download.add"
        );
        if !destination_dir.exists() {
            std::fs::create_dir_all(&destination_dir).map_err(|err| {
                RpcError::new(
                    1005,
                    "ERR_DESTINATION_UNWRITABLE",
                    format!("cannot create {}: {err}", destination_dir.display()),
                )
            })?;
        }
        let final_path = resolve_unique_path(&destination_dir, &suggested).map_err(|err| {
            RpcError::new(
                1005,
                "ERR_DESTINATION_UNWRITABLE",
                format!(
                    "cannot pick a file name in {}: {err}",
                    destination_dir.display()
                ),
            )
        })?;

        let (headers, secret) = split_headers(&params.headers, &params.cookies);
        let expected_checksum = match params.expected_checksum {
            Some(ChecksumParam { algorithm, hash }) => {
                if !algorithm.eq_ignore_ascii_case("sha256") {
                    return Err(RpcError::invalid_params(
                        "expected_checksum.algorithm must be sha256",
                    ));
                }
                Some(hash)
            }
            None => None,
        };
        // A task is pending until the engine starts it: "start immediately" only
        // decides whether the daemon launches it now.
        let status = DownloadStatus::Pending;
        let record = DownloadRecord {
            id: task_id.clone(),
            url: url.to_string(),
            final_url: None,
            final_path: final_path.to_string_lossy().into_owned(),
            temp_path: None,
            total_bytes: None,
            downloaded_bytes: 0,
            status,
            supports_ranges: false,
            etag: None,
            last_modified: None,
            content_type: None,
            expected_checksum,
            error_code: None,
            error_message: None,
            retry_count: 0,
            priority: params.priority,
            queue_name: params.queue_name.clone(),
            download_type: "direct".to_string(),
            created_at: None,
            completed_at: None,
            filename: suggested,
            media_kind: None,
            media_format: None,
            playlist_title: None,
            playlist_count: None,
            batch_id: None,
        };
        // Write the secret first: if there is no keyring the caller gets 1031 and no
        // task is created, so the sensitive header is never silently dropped.
        let secret_alias = match secret {
            Some(secret) => {
                let alias = format!("download/{task_id}");
                self.secrets.store(&alias, &secret).map_err(|err| {
                    RpcError::new(1031, "ERR_KEYRING_UNAVAILABLE", err.to_string())
                })?;
                Some(alias)
            }
            None => None,
        };
        self.store.insert_download(&record).map_err(store_error)?;
        if !headers.is_empty() {
            self.store
                .set_headers(&task_id, &headers)
                .map_err(store_error)?;
        }
        if let Some(alias) = secret_alias {
            self.store
                .set_secret_alias(&task_id, &alias)
                .map_err(store_error)?;
        }
        self.bus.publish(
            "task_added",
            Some(&task_id),
            json!({
                "task_id": task_id,
                "status": "pending",
                "url": record.url,
                "final_path": record.final_path,
            }),
        );
        if params.start_immediately {
            self.start_task(&task_id, connections)?;
        }
        Ok(json!({
            "task_id": task_id,
            "status": self
                .store
                .get(&task_id)
                .ok()
                .flatten()
                .map(|record| record.status.as_str().to_string())
                .unwrap_or_else(|| "pending".to_string()),
            "filename": record.filename,
        }))
    }

    /// Spawn the engine for one task.
    fn start_task(&self, task_id: &str, connections: u32) -> Result<(), RpcError> {
        {
            let running = self.running.lock().unwrap_or_else(|err| err.into_inner());
            if running.contains_key(task_id) {
                return Err(RpcError::task_already_active(task_id));
            }
        }
        let record = self
            .store
            .get(task_id)
            .map_err(store_error)?
            .ok_or_else(|| RpcError::task_not_found(task_id))?;
        let settings = self.settings();
        // A resumed task may outlive its destination folder (removed by the user, or a
        // pending task that was never started); recreate it instead of failing ENOENT.
        if let Some(parent) = PathBuf::from(&record.final_path).parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                RpcError::new(
                    1005,
                    "ERR_DESTINATION_UNWRITABLE",
                    format!("cannot create {}: {err}", parent.display()),
                )
            })?;
        }
        let resume = self.build_resume(&record)?;
        if resume.is_none() {
            // Nothing was checkpointed, so the bytes on disk are unverified: discard them
            // and let the engine publish a fresh file (the path is already task-private).
            let path = PathBuf::from(&record.final_path);
            if path.exists() {
                std::fs::remove_file(&path).map_err(|err| {
                    RpcError::internal(format!("cannot reset {}: {err}", path.display()))
                })?;
            }
        }
        let (headers, sensitive) = self.request_headers(task_id)?;
        let http = HttpConfig {
            user_agent: format!("Segmenta/{}", crate::DAEMON_VERSION),
            allow_local: settings.allow_local_downloads,
            proxy: if settings.proxy_mode == "manual" {
                settings.proxy_url.clone()
            } else if settings.proxy_mode == "none" {
                None
            } else {
                std::env::var("https_proxy")
                    .ok()
                    .or_else(|| std::env::var("HTTPS_PROXY").ok())
                    .or_else(|| std::env::var("http_proxy").ok())
            },
            headers,
            sensitive_headers: sensitive,
            ..HttpConfig::default()
        };
        let cfg = EngineConfig {
            connections,
            max_retries: settings.max_retries,
            http,
            speed_limit_bps: settings.global_speed_limit_bps.filter(|limit| *limit > 0),
            chunk_size: 64 * 1024,
        };
        let spec = DownloadSpec {
            url: url::Url::parse(&record.url)
                .map_err(|err| RpcError::new(1004, "ERR_INVALID_URL", err.to_string()))?,
            path: PathBuf::from(&record.final_path),
            expected_sha256: record.expected_checksum.clone(),
            resume,
            plan_override: None,
        };
        let attempt_id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        tracing::info!(
            task_id = %task_id,
            attempt_id = %attempt_id,
            connections,
            "engine start"
        );
        let cancel = Canceller::new();
        let (event_tx, mut event_rx) = mpsc::channel(512);
        let store = Arc::clone(&self.store);
        let bus = Arc::clone(&self.bus);
        let progress = Arc::clone(&self.progress);
        let id = task_id.to_string();
        let cancel_for_task = cancel.clone();
        let join = tokio::spawn(async move {
            let engine = tokio::spawn(run_engine(cfg, spec, event_tx, cancel_for_task));
            while let Some(event) = event_rx.recv().await {
                handle_event(&store, &bus, &progress, &id, event);
            }
            let outcome = engine.await.unwrap_or_else(|err| TaskOutcome::Failed {
                error: CoreError::Network(format!("engine task panicked: {err}")),
            });
            finish_task(&store, &bus, &progress, &id, outcome);
        });
        self.running
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(
                task_id.to_string(),
                RunningTask {
                    cancel,
                    handle: join,
                },
            );
        self.store
            .set_status(task_id, DownloadStatus::Downloading, None)
            .map_err(store_error)?;
        self.publish_state(task_id, "pending", "downloading", None);
        Ok(())
    }

    fn build_resume(&self, record: &DownloadRecord) -> Result<Option<ResumeState>, RpcError> {
        if record.downloaded_bytes == 0 && !record.supports_ranges {
            return Ok(None);
        }
        let segments = self.store.segments(&record.id).map_err(store_error)?;
        if segments.is_empty() {
            return Ok(None);
        }
        let Some(total) = record.total_bytes else {
            // A chunked transfer has no total and no segment plan: not resumable.
            return Err(RpcError::task_not_resumable(&record.id));
        };
        if !record.supports_ranges {
            return Err(RpcError::task_not_resumable(&record.id));
        }
        Ok(Some(ResumeState {
            total_bytes: total,
            supports_ranges: record.supports_ranges,
            etag: record.etag.clone(),
            last_modified: record.last_modified.clone(),
            segments: segments
                .into_iter()
                .map(|segment| SegmentStateRecord {
                    id: segment.id,
                    start: segment.start,
                    end: segment.end,
                    next: segment.next,
                })
                .collect(),
            single_stream_written: record.downloaded_bytes,
            last_error: None,
        }))
    }

    fn request_headers(&self, task_id: &str) -> Result<(HeaderMap, HeaderMap), RpcError> {
        let stored = self.store.headers(task_id).map_err(store_error)?;
        let mut headers = HeaderMap::new();
        let mut sensitive = HeaderMap::new();
        for (name, value) in stored {
            let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
                continue;
            };
            let Ok(value) = HeaderValue::from_str(&value) else {
                continue;
            };
            if is_sensitive(&name) {
                sensitive.insert(name, value);
            } else {
                headers.insert(name, value);
            }
        }
        if let Some(alias) = self.store.secret_alias(task_id).map_err(store_error)? {
            if let Ok(Some(secret)) = self.secrets.lookup(&alias) {
                apply_secret(&secret, &mut sensitive);
            }
        }
        Ok((headers, sensitive))
    }

    async fn pause_task(&self, task_id: &str) -> Result<Value, RpcError> {
        let record = self
            .store
            .get(task_id)
            .map_err(store_error)?
            .ok_or_else(|| RpcError::task_not_found(task_id))?;
        // Only a task with work in flight (or one that never started) can be paused.
        // A terminal task must keep its state and must not emit a fake transition.
        if !matches!(
            record.status,
            DownloadStatus::Downloading
                | DownloadStatus::Pending
                | DownloadStatus::Probing
                | DownloadStatus::Retrying
                | DownloadStatus::Verifying
                | DownloadStatus::Finalizing
        ) {
            return Err(RpcError::task_not_pausable(task_id));
        }
        tracing::info!(task_id = %task_id, status = "paused", "download.pause");
        let running = self
            .running
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(task_id);
        if let Some(task) = running {
            task.cancel.cancel();
            // The engine checkpoints on its way out; waiting keeps the status honest.
            let _ = task.handle.await;
        } else {
            self.store
                .set_status(task_id, DownloadStatus::Paused, None)
                .map_err(store_error)?;
            self.publish_state(task_id, record.status.as_str(), "paused", None);
        }
        let record = self
            .store
            .get(task_id)
            .map_err(store_error)?
            .ok_or_else(|| RpcError::task_not_found(task_id))?;
        Ok(json!({ "task_id": record.id, "status": record.status.as_str() }))
    }

    fn resume_task(&self, task_id: &str) -> Result<Value, RpcError> {
        let record = self
            .store
            .get(task_id)
            .map_err(store_error)?
            .ok_or_else(|| RpcError::task_not_found(task_id))?;
        match record.status {
            DownloadStatus::Downloading => {
                return Err(RpcError::task_already_active(task_id));
            }
            DownloadStatus::Paused | DownloadStatus::Retrying => {}
            other => {
                return Err(RpcError::task_not_resumable_state(task_id, other.as_str()));
            }
        }
        let connections = self.settings().max_connections;
        tracing::info!(task_id = %task_id, "download.resume");
        self.start_task(task_id, connections)?;
        Ok(json!({ "task_id": task_id, "status": "downloading" }))
    }

    async fn cancel_task(&self, params: CancelParams) -> Result<Value, RpcError> {
        let record = self
            .store
            .get(&params.task_id)
            .map_err(store_error)?
            .ok_or_else(|| RpcError::task_not_found(&params.task_id))?;
        // Bind first: a temporary guard in an `if let` scrutinee would live across await.
        let running_task = self
            .running
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&params.task_id);
        if let Some(task) = running_task {
            task.cancel.cancel();
            let _ = task.handle.await;
        }
        self.store
            .set_status(&params.task_id, DownloadStatus::Cancelled, None)
            .map_err(store_error)?;
        let mut deleted = false;
        if params.delete_file {
            if let Err(err) = std::fs::remove_file(&record.final_path) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    return Err(RpcError::internal(format!(
                        "cancelled, but could not delete {}: {err}",
                        record.final_path
                    )));
                }
            } else {
                deleted = true;
            }
        }
        self.cleanup_secret(&params.task_id);
        self.publish_state(&params.task_id, record.status.as_str(), "cancelled", None);
        Ok(json!({
            "task_id": params.task_id,
            "status": "cancelled",
            "file_deleted": deleted,
        }))
    }

    async fn delete_task(&self, params: DeleteParams) -> Result<Value, RpcError> {
        let record = self
            .store
            .get(&params.task_id)
            .map_err(store_error)?
            .ok_or_else(|| RpcError::task_not_found(&params.task_id))?;
        // Bind first: a temporary guard in an `if let` scrutinee would live across await.
        let running_task = self
            .running
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&params.task_id);
        if let Some(task) = running_task {
            task.cancel.cancel();
            let _ = task.handle.await;
        }
        if params.delete_file {
            let _ = std::fs::remove_file(&record.final_path);
        }
        let deleted = self.store.delete(&params.task_id).map_err(store_error)?;
        self.cleanup_secret(&params.task_id);
        self.progress
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&params.task_id);
        Ok(json!({ "task_id": params.task_id, "deleted": deleted }))
    }

    fn cleanup_secret(&self, task_id: &str) {
        if let Ok(Some(alias)) = self.store.secret_alias(task_id) {
            if let Err(err) = self.secrets.delete(&alias) {
                eprintln!("segmenta-daemon: could not delete secret {alias}: {err}");
            }
        }
    }

    fn list_downloads(&self, params: ListParams) -> Result<Value, RpcError> {
        let filter = match params.filter.as_deref() {
            None | Some("all") => None,
            Some("active") => Some(DownloadStatus::Downloading),
            Some("paused") => Some(DownloadStatus::Paused),
            Some("completed") => Some(DownloadStatus::Completed),
            Some("failed") => Some(DownloadStatus::Failed),
            Some("cancelled") => Some(DownloadStatus::Cancelled),
            Some("pending") => Some(DownloadStatus::Pending),
            Some(other) => {
                return Err(RpcError::invalid_params(format!(
                    "unknown filter `{other}`"
                )));
            }
        };
        let limit = params.limit.unwrap_or(100).clamp(1, 1000);
        let offset = params.offset.unwrap_or(0);
        let rows = self
            .store
            .list(filter, limit, offset)
            .map_err(store_error)?;
        let views: Vec<DownloadView> = rows.iter().map(|row| self.view(row, false)).collect();
        Ok(serde_json::to_value(views).unwrap_or(Value::Array(Vec::new())))
    }

    fn status_download(&self, task_id: &str) -> Result<Value, RpcError> {
        let record = self
            .store
            .get(task_id)
            .map_err(store_error)?
            .ok_or_else(|| RpcError::task_not_found(task_id))?;
        Ok(serde_json::to_value(self.view(&record, true)).unwrap_or(Value::Null))
    }

    fn settings_get(&self) -> Result<Value, RpcError> {
        let settings = self.settings();
        serde_json::to_value(settings).map_err(|err| RpcError::internal(err.to_string()))
    }

    fn settings_update(&self, params: Option<Value>) -> Result<Value, RpcError> {
        let Some(Value::Object(patch)) = params else {
            return Err(RpcError::invalid_params("params must be an object"));
        };
        let updated = match self.store.update_settings(&patch) {
            Ok(settings) => settings,
            Err(StoreError::Invalid(message)) => {
                return Err(RpcError::invalid_params(message));
            }
            Err(err) => return Err(store_error(err)),
        };
        *self.settings.lock().unwrap_or_else(|err| err.into_inner()) = updated.clone();
        serde_json::to_value(updated).map_err(|err| RpcError::internal(err.to_string()))
    }

    fn queues(&self) -> Result<Value, RpcError> {
        let rows = self.store.queues().map_err(store_error)?;
        let mut out = Vec::new();
        for row in rows {
            let all = self
                .store
                .list(None, 1000, 0)
                .map_err(store_error)?
                .into_iter()
                .filter(|record| record.queue_name == row.name)
                .collect::<Vec<_>>();
            let active = all
                .iter()
                .filter(|record| record.status == DownloadStatus::Downloading)
                .count();
            let pending = all
                .iter()
                .filter(|record| record.status == DownloadStatus::Pending)
                .count();
            let paused = all
                .iter()
                .filter(|record| record.status == DownloadStatus::Paused)
                .count();
            out.push(json!({
                "name": row.name,
                "max_active": row.max_active,
                "active": active,
                "pending": pending,
                "paused": paused,
            }));
        }
        Ok(Value::Array(out))
    }

    fn create_queue(&self, params: QueueCreateParams) -> Result<Value, RpcError> {
        self.store
            .create_queue(&params.name, params.max_active)
            .map_err(|err| match err {
                StoreError::Queue(message) => RpcError::invalid_params(message),
                other => store_error(other),
            })?;
        Ok(json!({ "name": params.name, "max_active": params.max_active }))
    }

    fn remove_queue(&self, params: QueueRemoveParams) -> Result<Value, RpcError> {
        self.store
            .remove_queue(&params.name)
            .map_err(|err| match err {
                StoreError::Queue(message) => RpcError::new(1001, "QUEUE_IN_USE", message),
                other => store_error(other),
            })?;
        Ok(json!({ "name": params.name, "removed": true }))
    }

    fn schedule_queue(&self, params: QueueScheduleParams) -> Result<Value, RpcError> {
        let schedule = json!({
            "start_time": params.start_time,
            "stop_time": params.stop_time,
            "action_on_complete": params.action_on_complete,
        });
        self.store
            .schedule_queue(&params.queue_name, &schedule)
            .map_err(|err| match err {
                StoreError::Queue(message) => RpcError::invalid_params(message),
                other => store_error(other),
            })?;
        Ok(json!({ "queue_name": params.queue_name, "schedule": schedule }))
    }

    fn system_status(&self) -> Value {
        json!({
            "daemon_version": crate::DAEMON_VERSION,
            "uptime_seconds": self.started.elapsed().as_secs(),
            "memory_bytes": memory_bytes(),
            "active_connections": self.running_count(),
            "event_subscribers": self.bus.subscriber_count(),
            "secret_backend": self.secrets.backend(),
        })
    }
}

fn memory_bytes() -> u64 {
    // resident set size from /proc, best effort: the field is informational.
    let Ok(contents) = std::fs::read_to_string("/proc/self/statm") else {
        return 0;
    };
    let mut fields = contents.split_whitespace();
    let pages: u64 = fields
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    pages.saturating_mul(4096)
}

fn store_error(err: StoreError) -> RpcError {
    RpcError::internal(err.to_string())
}

fn is_sensitive(name: &HeaderName) -> bool {
    let name = name.as_str().to_ascii_lowercase();
    name == "authorization" || name == "cookie" || name == "proxy-authorization"
}

fn split_headers(
    headers: &BTreeMap<String, String>,
    cookies: &[crate::rpc::CookieParam],
) -> (BTreeMap<String, String>, Option<String>) {
    let mut plain = BTreeMap::new();
    let mut secret: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if lower == "authorization" || lower == "proxy-authorization" {
            secret.insert(lower, value.clone());
        } else if lower == "cookie" {
            secret.insert("cookie".to_string(), value.clone());
        } else {
            plain.insert(name.clone(), value.clone());
        }
    }
    if !cookies.is_empty() {
        let jar: BTreeMap<String, String> = cookies
            .iter()
            .map(|cookie| (cookie.name.clone(), cookie.value.clone()))
            .collect();
        secret.insert(
            "cookies".to_string(),
            serde_json::to_string(&jar).unwrap_or_default(),
        );
        let header = cookies
            .iter()
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>()
            .join("; ");
        secret.insert("cookie".to_string(), header);
    }
    let secret = if secret.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&secret).unwrap_or_default())
    };
    (plain, secret)
}

fn apply_secret(secret: &str, sensitive: &mut HeaderMap) {
    let Ok(map) = serde_json::from_str::<BTreeMap<String, String>>(secret) else {
        return;
    };
    for (name, value) in map {
        if name == "cookies" {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            sensitive.insert(name, value);
        }
    }
}

fn handle_event(
    store: &Arc<Store>,
    bus: &Arc<EventBus>,
    progress: &Arc<Mutex<HashMap<String, ProgressSnapshot>>>,
    task_id: &str,
    event: EngineEvent,
) {
    match event {
        EngineEvent::Probing => {
            bus.publish(
                "state_changed",
                Some(task_id),
                json!({ "old_state": "pending", "new_state": "probing" }),
            );
        }
        EngineEvent::ProbingFinished {
            total_bytes,
            supports_ranges,
            etag,
            last_modified,
        } => {
            let _ = store.set_probe_result(
                task_id,
                total_bytes,
                supports_ranges,
                etag.as_deref(),
                last_modified.as_deref(),
            );
            bus.publish(
                "state_changed",
                Some(task_id),
                json!({
                    "old_state": "probing",
                    "new_state": "downloading",
                    "total_bytes": total_bytes,
                    "supports_ranges": supports_ranges,
                    "etag": etag,
                    "last_modified": last_modified,
                }),
            );
        }
        EngineEvent::SegmentStarted { id, start, end } => {
            bus.publish(
                "segment_started",
                Some(task_id),
                json!({ "segment_id": id, "start_offset": start, "end_offset": end }),
            );
        }
        EngineEvent::SegmentSplit {
            donor_id,
            new_id,
            midpoint,
            original_end,
        } => {
            bus.publish(
                "segment_split",
                Some(task_id),
                json!({
                    "donor_segment_id": donor_id,
                    "new_segment_id": new_id,
                    "midpoint": midpoint,
                    "original_end": original_end,
                }),
            );
        }
        EngineEvent::SegmentCompleted { id, start, end } => {
            bus.publish(
                "segment_completed",
                Some(task_id),
                json!({ "segment_id": id, "start_offset": start, "end_offset": end }),
            );
        }
        EngineEvent::Checkpoint {
            segments,
            downloaded_bytes,
        } => {
            // The engine already fdatasync-ed the bytes; only now may the database
            // record them (OpenSpec 08 §8).
            let rows: Vec<SegmentRow> = segments
                .iter()
                .map(|segment| SegmentRow {
                    id: segment.id,
                    start: segment.start,
                    end: segment.end,
                    next: segment.next,
                })
                .collect();
            // Range support comes from the probe, not from an assumption: a chunked
            // task must never be marked resumable (S7).
            let supports_ranges = store
                .get(task_id)
                .ok()
                .flatten()
                .map(|record| record.supports_ranges)
                .unwrap_or(false);
            if let Err(err) = store.save_checkpoint(
                task_id,
                &Checkpoint {
                    segments: &rows,
                    downloaded_bytes,
                    total_bytes: None,
                    etag: None,
                    last_modified: None,
                    supports_ranges,
                },
            ) {
                eprintln!("segmenta-daemon: checkpoint failed for {task_id}: {err}");
            }
        }
        EngineEvent::Progress {
            downloaded_bytes,
            total_bytes,
            speed_bps,
            eta_seconds,
            segments,
        } => {
            let rows: Vec<SegmentRow> = segments
                .iter()
                .map(|segment| SegmentRow {
                    id: segment.id,
                    start: segment.start,
                    end: segment.end,
                    next: segment.next,
                })
                .collect();
            progress
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .insert(
                    task_id.to_string(),
                    ProgressSnapshot {
                        downloaded_bytes,
                        total_bytes,
                        speed_bps,
                        eta_seconds,
                        segments: rows.clone(),
                    },
                );
            let _ = store.update_progress(task_id, downloaded_bytes, total_bytes);
            let segments_json: Vec<Value> = rows
                .iter()
                .map(|segment| {
                    json!({
                        "segment_id": segment.id,
                        "start_offset": segment.start,
                        "end_offset": segment.end,
                        "next_offset": segment.next,
                        "status": if segment.next > segment.end { "completed" } else { "active" },
                        "speed_bps": 0,
                    })
                })
                .collect();
            bus.publish(
                "task_progress",
                Some(task_id),
                json!({
                    "downloaded_bytes": downloaded_bytes,
                    "total_bytes": total_bytes,
                    "speed_bps": speed_bps,
                    "eta_seconds": eta_seconds,
                    "segments": segments_json,
                }),
            );
        }
        EngineEvent::Restarted { reason } => {
            bus.publish(
                "download_restarted_due_to_stale_cache",
                Some(task_id),
                json!({ "reason": reason }),
            );
        }
        EngineEvent::Verifying => {
            let _ = store.set_status(task_id, DownloadStatus::Verifying, None);
            bus.publish(
                "state_changed",
                Some(task_id),
                json!({ "old_state": "downloading", "new_state": "verifying" }),
            );
        }
        EngineEvent::Completed {
            path,
            total_bytes,
            checksum_ok,
            ..
        } => {
            bus.publish(
                "task_completed",
                Some(task_id),
                json!({
                    "final_path": path.to_string_lossy(),
                    "total_bytes": total_bytes,
                    "checksum_ok": checksum_ok,
                }),
            );
        }
    }
}

fn finish_task(
    store: &Arc<Store>,
    bus: &Arc<EventBus>,
    progress: &Arc<Mutex<HashMap<String, ProgressSnapshot>>>,
    task_id: &str,
    outcome: TaskOutcome,
) {
    match outcome {
        TaskOutcome::Completed {
            total_bytes,
            checksum_ok,
            segments,
        } => {
            // Persist the final table: without it `status` would keep showing the last
            // checkpoint (a few hundred KiB short) for a task that already finished.
            let rows: Vec<SegmentRow> = segments
                .iter()
                .map(|segment| SegmentRow {
                    id: segment.id,
                    start: segment.start,
                    end: segment.end,
                    next: segment.end.saturating_add(1),
                })
                .collect();
            if let Err(err) = store.save_checkpoint(
                task_id,
                &Checkpoint {
                    segments: &rows,
                    downloaded_bytes: total_bytes,
                    total_bytes: Some(total_bytes),
                    etag: None,
                    last_modified: None,
                    supports_ranges: true,
                },
            ) {
                eprintln!("segmenta-daemon: final checkpoint failed for {task_id}: {err}");
            }
            let _ = store.set_status(task_id, DownloadStatus::Completed, None);
            tracing::info!(
                task_id = %task_id,
                total_bytes,
                checksum_ok = ?checksum_ok,
                "task completed"
            );
            bus.publish(
                "state_changed",
                Some(task_id),
                json!({ "old_state": "downloading", "new_state": "completed", "checksum_ok": checksum_ok }),
            );
            progress
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .remove(task_id);
        }
        TaskOutcome::Paused { state } => {
            let rows: Vec<SegmentRow> = state
                .segments
                .iter()
                .map(|segment| SegmentRow {
                    id: segment.id,
                    start: segment.start,
                    end: segment.end,
                    next: segment.next,
                })
                .collect();
            let downloaded: u64 = rows
                .iter()
                .map(|segment| segment.next.saturating_sub(segment.start))
                .sum();
            let error = state
                .last_error
                .as_ref()
                .map(|err| (err.code(), err.to_string()));
            let status = DownloadStatus::Paused;
            tracing::info!(
                task_id = %task_id,
                downloaded_bytes = downloaded,
                error_code = error.as_ref().map(|(code, _)| *code),
                "task paused"
            );
            let _ = store.save_checkpoint(
                task_id,
                &Checkpoint {
                    segments: &rows,
                    downloaded_bytes: downloaded,
                    total_bytes: Some(state.total_bytes),
                    etag: state.etag.as_deref(),
                    last_modified: state.last_modified.as_deref(),
                    supports_ranges: state.supports_ranges,
                },
            );
            let _ = store.set_status(task_id, status, error.clone());
            // Drop the live snapshot: a stopped task reports speed 0 and no ETA, and
            // the persisted checkpoint is the source of truth from here on.
            progress
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .remove(task_id);
            bus.publish(
                "state_changed",
                Some(task_id),
                json!({
                    "old_state": "downloading",
                    "new_state": "paused",
                    "error_code": error.as_ref().map(|(code, _)| code),
                    "message": error.as_ref().map(|(_, message)| message),
                }),
            );
        }
        TaskOutcome::Failed { error } => {
            let _ = store.set_status(
                task_id,
                DownloadStatus::Failed,
                Some((error.code(), error.to_string())),
            );
            tracing::warn!(
                task_id = %task_id,
                error_code = error.code(),
                error = %error,
                retryable = error.is_retryable(),
                "task failed"
            );
            bus.publish(
                "task_failed",
                Some(task_id),
                json!({
                    "error_code": error.code(),
                    "message": error.to_string(),
                    "retryable": error.is_retryable(),
                }),
            );
            progress
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .remove(task_id);
        }
    }
}

impl Manager {
    fn media_error(err: crate::media::MediaError) -> RpcError {
        RpcError::new(err.code, err.name, err.message)
    }

    /// `media.probe`: ask the backend for formats without creating a task.
    async fn media_probe(&self, params: MediaProbeParams) -> Result<Value, RpcError> {
        let url = crate::media::validate_media_url_with(
            &params.url,
            self.settings().allow_local_downloads,
        )
        .map_err(Self::media_error)?;
        let kind = params.kind.unwrap_or_else(|| "video".to_string());
        let limit = crate::media::clamp_limit(params.limit).map_err(Self::media_error)?;
        let host = url.host_str().unwrap_or_default().to_string();
        if crate::media::is_youtube(&host) {
            let tool = crate::media::locate_tool("yt-dlp").map_err(Self::media_error)?;
            let outcome = crate::media::probe_with(&tool, url.as_str(), &kind)
                .await
                .map_err(Self::media_error)?;
            tracing::info!(host = %host, kind = %outcome.kind, formats = outcome.formats.len(), "media.probe");
            return serde_json::to_value(outcome)
                .map_err(|err| RpcError::internal(err.to_string()));
        }
        // Public HLS/DASH: parse the manifest and report its variants.
        let client = media_http_client()?;
        crate::stream_media::probe(&client, &url)
            .await
            .map_err(Self::media_error)
    }

    /// `media.add`: probe, persist the task and start the backend runner.
    async fn media_add(&self, params: MediaAddParams) -> Result<Value, RpcError> {
        let url = crate::media::validate_media_url_with(
            &params.url,
            self.settings().allow_local_downloads,
        )
        .map_err(Self::media_error)?;
        let mode = params.mode.as_str();
        if !matches!(mode, "video" | "audio" | "playlist") {
            return Err(RpcError::invalid_params(
                "mode must be video, audio or playlist",
            ));
        }
        let limit = crate::media::clamp_limit(params.playlist_limit).map_err(Self::media_error)?;
        let host = url.host_str().unwrap_or_default().to_string();
        if !crate::media::is_youtube(&host) {
            return self.add_stream_task(url).await;
        }
        let tool = crate::media::locate_tool("yt-dlp").map_err(Self::media_error)?;
        let kind_hint = if mode == "playlist" {
            "playlist"
        } else {
            "video"
        };
        let probe = crate::media::probe_with(&tool, url.as_str(), kind_hint)
            .await
            .map_err(Self::media_error)?;

        let task_kind = match mode {
            "audio" => crate::media::MediaTaskKind::YoutubeAudio,
            "playlist" => crate::media::MediaTaskKind::YoutubePlaylist,
            _ => crate::media::MediaTaskKind::YoutubeVideo,
        };
        let settings = self.settings();
        let dir = PathBuf::from(&settings.download_dir);
        std::fs::create_dir_all(&dir).map_err(|err| {
            RpcError::new(
                1005,
                "ERR_DESTINATION_UNWRITABLE",
                format!("cannot create {}: {err}", dir.display()),
            )
        })?;
        let title = crate::media::sanitize_title(&probe.title);
        let is_playlist = task_kind.is_playlist();
        let extension = match mode {
            "audio" => "m4a",
            _ => "mp4",
        };
        let final_path = if is_playlist {
            dir.join(&title)
        } else {
            dir.join(format!("{title}.{extension}"))
        };
        let task_id = uuid::Uuid::new_v4().to_string();
        let batch_id = if is_playlist {
            Some(uuid::Uuid::new_v4().to_string())
        } else {
            None
        };
        let record = DownloadRecord {
            id: task_id.clone(),
            url: url.to_string(),
            final_url: None,
            final_path: final_path.to_string_lossy().into_owned(),
            temp_path: None,
            total_bytes: None,
            downloaded_bytes: 0,
            status: DownloadStatus::Downloading,
            supports_ranges: false,
            etag: None,
            last_modified: None,
            content_type: None,
            expected_checksum: None,
            error_code: None,
            error_message: None,
            retry_count: 0,
            priority: 0,
            queue_name: "main".to_string(),
            download_type: task_kind.as_str().to_string(),
            created_at: None,
            completed_at: None,
            filename: title.clone(),
            media_kind: Some(mode.to_string()),
            media_format: params.format_id.clone(),
            playlist_title: is_playlist.then(|| probe.title.clone()),
            playlist_count: is_playlist.then_some(limit.min(probe.playlist.count.max(1)) as u32),
            batch_id: batch_id.clone(),
        };
        self.store.insert_download(&record).map_err(store_error)?;
        self.bus.publish(
            "task_added",
            Some(&task_id),
            json!({
                "task_id": task_id,
                "status": "downloading",
                "url": record.url,
                "final_path": record.final_path,
                "media_kind": mode,
            }),
        );
        let output = if is_playlist {
            final_path.join("%(title)s.%(ext)s")
        } else {
            final_path.clone()
        };
        let plan = crate::media::DownloadPlan {
            kind: task_kind,
            url: url.to_string(),
            output,
            format: params.format_id.clone(),
            playlist_limit: limit,
            height: params.height,
        };
        self.start_media_task(task_id.clone(), plan, tool, final_path)?;
        Ok(json!({
            "task_id": task_id,
            "status": "downloading",
            "kind": task_kind.as_str(),
            "batch_id": batch_id,
        }))
    }

    /// `media.add` for a public HLS/DASH manifest.
    async fn add_stream_task(&self, url: url::Url) -> Result<Value, RpcError> {
        let client = media_http_client()?;
        let selection = crate::stream_media::resolve(&client, &url, None)
            .await
            .map_err(Self::media_error)?;
        let ffmpeg = crate::media::locate_tool("ffmpeg").map_err(Self::media_error)?;
        let settings = self.settings();
        let dir = PathBuf::from(&settings.download_dir);
        std::fs::create_dir_all(&dir).map_err(|err| {
            RpcError::new(
                1005,
                "ERR_DESTINATION_UNWRITABLE",
                format!("cannot create {}: {err}", dir.display()),
            )
        })?;
        let stem = crate::stream_media::stream_stem(&url);
        let final_path = dir.join(format!("{stem}.mp4"));
        let task_id = uuid::Uuid::new_v4().to_string();
        let record = DownloadRecord {
            id: task_id.clone(),
            url: url.to_string(),
            final_url: None,
            final_path: final_path.to_string_lossy().into_owned(),
            temp_path: None,
            total_bytes: None,
            downloaded_bytes: 0,
            status: DownloadStatus::Downloading,
            supports_ranges: false,
            etag: None,
            last_modified: None,
            content_type: None,
            expected_checksum: None,
            error_code: None,
            error_message: None,
            retry_count: 0,
            priority: 0,
            queue_name: "main".to_string(),
            download_type: selection.kind.to_string(),
            created_at: None,
            completed_at: None,
            filename: stem.clone(),
            media_kind: Some("stream".to_string()),
            media_format: selection.height.map(|height| format!("{height}p")),
            playlist_title: None,
            playlist_count: None,
            batch_id: None,
        };
        self.store.insert_download(&record).map_err(store_error)?;
        self.bus.publish(
            "task_added",
            Some(&task_id),
            json!({
                "task_id": task_id,
                "status": "downloading",
                "url": record.url,
                "final_path": record.final_path,
                "media_kind": "stream",
            }),
        );
        let cancel = Canceller::new();
        let store = Arc::clone(&self.store);
        let bus = Arc::clone(&self.bus);
        let progress = Arc::clone(&self.progress);
        let id = task_id.clone();
        let task_cancel = cancel.clone();
        let path = final_path.clone();
        let job = selection.clone();
        let kind = selection.kind;
        let handle = tokio::spawn(async move {
            let mut last = std::time::Instant::now();
            let mut last_bytes = 0u64;
            let result = crate::stream_media::download(
                &client,
                &ffmpeg,
                &job,
                &path,
                task_cancel,
                |downloaded| {
                    last_bytes = downloaded;
                    if last.elapsed() < std::time::Duration::from_millis(250) {
                        return;
                    }
                    last = std::time::Instant::now();
                    let _ = store.update_progress(&id, downloaded, None);
                    progress
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .insert(
                            id.clone(),
                            ProgressSnapshot {
                                downloaded_bytes: downloaded,
                                total_bytes: None,
                                speed_bps: 0,
                                eta_seconds: None,
                                segments: Vec::new(),
                            },
                        );
                    bus.publish(
                        "task_progress",
                        Some(&id),
                        json!({
                            "downloaded_bytes": downloaded,
                            "total_bytes": null,
                            "speed_bps": 0,
                            "eta_seconds": null,
                            "segments": [],
                        }),
                    );
                },
            )
            .await;
            match result {
                Ok(()) => {
                    let size = std::fs::metadata(&path)
                        .map(|meta| meta.len())
                        .unwrap_or(last_bytes);
                    let _ = store.update_progress(&id, size, Some(size));
                    let _ = store.set_status(&id, DownloadStatus::Completed, None);
                    progress
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .remove(&id);
                    bus.publish(
                        "task_completed",
                        Some(&id),
                        json!({
                            "final_path": path.to_string_lossy(),
                            "total_bytes": size,
                            "checksum_ok": null,
                            "media": true,
                        }),
                    );
                }
                Err(error) => {
                    let _ = store.set_status(
                        &id,
                        DownloadStatus::Failed,
                        Some((error.code, error.message.clone())),
                    );
                    progress
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .remove(&id);
                    bus.publish(
                        "task_failed",
                        Some(&id),
                        json!({
                            "error_code": error.code,
                            "message": error.message,
                            "retryable": false,
                        }),
                    );
                }
            }
        });
        self.running
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(task_id.clone(), RunningTask { cancel, handle });
        Ok(json!({
            "task_id": task_id,
            "status": "downloading",
            "kind": kind,
        }))
    }

    /// `media.status`: task view plus playlist counters.
    fn media_status(&self, params: MediaStatusParams) -> Result<Value, RpcError> {
        let task_id = match (params.task_id, params.batch_id) {
            (Some(task_id), _) => task_id,
            (None, Some(batch_id)) => {
                let record = self
                    .store
                    .list(None, 1000, 0)
                    .map_err(store_error)?
                    .into_iter()
                    .find(|record| record.batch_id.as_deref() == Some(batch_id.as_str()))
                    .ok_or_else(|| RpcError::task_not_found(&batch_id))?;
                record.id
            }
            (None, None) => {
                return Err(RpcError::invalid_params("task_id or batch_id is required"));
            }
        };
        let record = self
            .store
            .get(&task_id)
            .map_err(store_error)?
            .ok_or_else(|| RpcError::task_not_found(&task_id))?;
        let mut view = serde_json::to_value(self.view(&record, true))
            .map_err(|err| RpcError::internal(err.to_string()))?;
        if let Value::Object(map) = &mut view {
            map.insert("media_kind".into(), json!(record.media_kind));
            map.insert("media_format".into(), json!(record.media_format));
            map.insert("playlist_title".into(), json!(record.playlist_title));
            map.insert("playlist_count".into(), json!(record.playlist_count));
            map.insert("batch_id".into(), json!(record.batch_id));
        }
        Ok(view)
    }

    /// Run `yt-dlp` for one media task and report progress on the bus.
    fn start_media_task(
        &self,
        task_id: String,
        plan: crate::media::DownloadPlan,
        tool: PathBuf,
        final_path: PathBuf,
    ) -> Result<(), RpcError> {
        let cancel = Canceller::new();
        let store = Arc::clone(&self.store);
        let bus = Arc::clone(&self.bus);
        let progress = Arc::clone(&self.progress);
        let id = task_id.clone();
        let task_cancel = cancel.clone();
        let handle = tokio::spawn(async move {
            let mut last = std::time::Instant::now();
            let mut last_downloaded = 0u64;
            let result = crate::media::run_download(&tool, &plan, task_cancel, |line| {
                if last.elapsed() < std::time::Duration::from_millis(250) {
                    return;
                }
                last = std::time::Instant::now();
                last_downloaded = line.downloaded;
                let _ = store.update_progress(&id, line.downloaded, line.total);
                progress
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .insert(
                        id.clone(),
                        ProgressSnapshot {
                            downloaded_bytes: line.downloaded,
                            total_bytes: line.total,
                            speed_bps: line.speed,
                            eta_seconds: line.eta,
                            segments: Vec::new(),
                        },
                    );
                bus.publish(
                    "task_progress",
                    Some(&id),
                    json!({
                        "downloaded_bytes": line.downloaded,
                        "total_bytes": line.total,
                        "speed_bps": line.speed,
                        "eta_seconds": line.eta,
                        "segments": [],
                    }),
                );
            })
            .await;
            match result {
                Ok(()) => {
                    let _ = store.set_status(&id, DownloadStatus::Completed, None);
                    let _ = store.update_progress(&id, last_downloaded, None);
                    progress
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .remove(&id);
                    bus.publish(
                        "task_completed",
                        Some(&id),
                        json!({
                            "final_path": final_path.to_string_lossy(),
                            "total_bytes": last_downloaded,
                            "checksum_ok": null,
                            "media": true,
                        }),
                    );
                }
                Err(error) => {
                    let _ = store.set_status(
                        &id,
                        DownloadStatus::Failed,
                        Some((error.code, error.message.clone())),
                    );
                    progress
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .remove(&id);
                    bus.publish(
                        "task_failed",
                        Some(&id),
                        json!({
                            "error_code": error.code,
                            "message": error.message,
                            "retryable": false,
                        }),
                    );
                }
            }
        });
        self.running
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(task_id, RunningTask { cancel, handle });
        Ok(())
    }
}

impl RpcHandler for Manager {
    fn call<'a>(
        &'a self,
        method: &'a str,
        params: Option<Value>,
    ) -> BoxFuture<'a, Result<Value, RpcError>> {
        Box::pin(async move {
            let parse = |params: Option<Value>| -> Result<Value, RpcError> {
                Ok(params.unwrap_or_else(|| json!({})))
            };
            match method {
                "system.hello" => Ok(hello_result(crate::DAEMON_VERSION)),
                "system.status" => Ok(self.system_status()),
                "system.shutdown" => {
                    // Collect first: the guard must not live across an await.
                    let tasks: Vec<RunningTask> = {
                        let mut running =
                            self.running.lock().unwrap_or_else(|err| err.into_inner());
                        running.drain().map(|(_, task)| task).collect()
                    };
                    for task in tasks {
                        task.cancel.cancel();
                        let _ = task.handle.await;
                    }
                    let _ = self.shutdown.send(true);
                    Ok(json!({ "shutting_down": true }))
                }
                "download.add" => {
                    let parsed: AddDownloadParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.add_download(parsed)
                }
                "download.list" => {
                    let parsed: ListParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.list_downloads(parsed)
                }
                "download.status" => {
                    let parsed: TaskIdParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.status_download(&parsed.task_id)
                }
                "download.pause" => {
                    let parsed: TaskIdParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.pause_task(&parsed.task_id).await
                }
                "download.resume" => {
                    let parsed: TaskIdParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.resume_task(&parsed.task_id)
                }
                "download.cancel" => {
                    let parsed: CancelParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.cancel_task(parsed).await
                }
                "download.delete" => {
                    let parsed: DeleteParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.delete_task(parsed).await
                }
                "media.probe" => {
                    let parsed: MediaProbeParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.media_probe(parsed).await
                }
                "media.add" => {
                    let parsed: MediaAddParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.media_add(parsed).await
                }
                "media.status" => {
                    let parsed: MediaStatusParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.media_status(parsed)
                }
                "settings.get" => self.settings_get(),
                "settings.update" => self.settings_update(params),
                "queue.list" => self.queues(),
                "queue.create" => {
                    let parsed: QueueCreateParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.create_queue(parsed)
                }
                "queue.remove" => {
                    let parsed: QueueRemoveParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.remove_queue(parsed)
                }
                "queue.schedule" => {
                    let parsed: QueueScheduleParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.schedule_queue(parsed)
                }
                other => Err(RpcError::method_not_found(other)),
            }
        })
    }
}

/// Planner helper kept public for the daemon tests.
pub fn planned_segments(total: u64, connections: u32) -> Vec<planner::SegmentRange> {
    planner::initial_segments(total, connections)
}

/// HTTPS client for manifest and fragment downloads.
fn media_http_client() -> Result<reqwest::Client, RpcError> {
    reqwest::Client::builder()
        .user_agent(concat!("Segmenta/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|err| RpcError::internal(err.to_string()))
}
