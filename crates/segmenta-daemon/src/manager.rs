//! Task orchestration: turns JSON-RPC calls into engine runs, bridges engine events to
//! the event bus and persists checkpoints in the order OpenSpec 08 mandates.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch, Notify};

use segmenta_core::engine::{
    run as run_engine, Canceller, DownloadSpec, EngineConfig, EngineEvent, ResumeState,
    SegmentStateRecord, TaskOutcome,
};
use segmenta_core::filename::{resolve_unique_path, sanitize_filename, suggest_filename};
use segmenta_core::probe::HttpConfig;
use segmenta_core::{planner, CoreError};

use crate::events::EventBus;
use crate::ipc::{hello_result, RpcHandler};
use crate::postprocess::{HookOptions, HookRun, HookTarget};
use crate::rpc::{
    AddDownloadParams, CancelParams, ChecksumParam, DeleteParams, DownloadView, ListParams,
    MediaAddParams, MediaProbeParams, MediaStatusParams, QueueCreateParams, QueueRemoveParams,
    QueueScheduleParams, RpcError, SegmentView, SiteLoginOriginParams, SiteLoginSetParams,
    TaskIdParams,
};
use crate::scheduler::{resolve_schedule, Clock, LocalClock, QueueSchedule};
use crate::secrets::SecretStore;
use crate::store::{
    Checkpoint, DownloadRecord, DownloadStatus, PostprocessRecord, SegmentRow, Settings,
    SiteLoginRow, Store, StoreError, TorrentFileInsert,
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
    /// Queue the task belongs to, so its slot can be counted without a second lookup.
    queue: String,
}

/// Coordinates the store, the engine and the IPC layer.
pub struct Manager {
    store: Arc<Store>,
    secrets: Arc<dyn SecretStore>,
    bus: Arc<EventBus>,
    settings: Mutex<Settings>,
    progress: Arc<Mutex<HashMap<String, ProgressSnapshot>>>,
    running: Arc<Mutex<HashMap<String, RunningTask>>>,
    /// Serialises the slot decision of [`Manager::start_pending_for_queue`]: two callers
    /// (an RPC handler and the scheduler tick) must never both see the same free slot.
    start_lock: Mutex<()>,
    shutdown: watch::Sender<bool>,
    /// Wall clock the scheduler gate reads; injectable so tests never depend on the host.
    clock: Mutex<Arc<dyn Clock>>,
    /// Wake-up of the queue scheduler, signalled whenever a task stops running.
    scheduler_wake: Arc<Notify>,
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
            start_lock: Mutex::new(()),
            shutdown,
            clock: Mutex::new(Arc::new(LocalClock)),
            scheduler_wake: Arc::new(Notify::new()),
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

    /// Replace the wall clock the scheduler gate reads.
    ///
    /// The daemon keeps [`LocalClock`]; a test injects a fixed minute so a window can be
    /// evaluated without sleeping and without depending on the host time.
    pub fn set_clock(&self, clock: Arc<dyn Clock>) {
        *self.clock.lock().unwrap_or_else(|err| err.into_inner()) = clock;
    }

    fn minute_of_day(&self) -> u32 {
        self.clock
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .minute_of_day()
    }

    /// Handle the queue scheduler waits on to be woken before the next tick.
    pub fn scheduler_wake(&self) -> Arc<Notify> {
        Arc::clone(&self.scheduler_wake)
    }

    /// Tell the scheduler a task stopped running, so the next pending one can start now.
    fn notify_scheduler(&self) {
        self.scheduler_wake.notify_one();
    }

    /// Ask the daemon to shut down through the existing watch channel.
    ///
    /// It is the ordered shutdown `system.shutdown` already uses, so
    /// `action_on_complete: exit` needs no second mechanism.
    pub fn request_shutdown(&self) {
        let _ = self.shutdown.send(true);
    }

    /// Names of every queue the daemon knows, in `queue.list` order.
    pub fn queue_names(&self) -> Result<Vec<String>, StoreError> {
        Ok(self
            .store
            .queues()?
            .into_iter()
            .map(|row| row.name)
            .collect())
    }

    /// Effective schedule of one queue: its window plus the action run when it drains.
    pub fn queue_schedule(&self, queue: &str) -> QueueSchedule {
        let stored = self.store.queue_schedule(queue).unwrap_or_else(|err| {
            tracing::warn!(queue = %queue, error = %err, "cannot read the persisted schedule");
            None
        });
        resolve_schedule(queue, stored.as_ref(), &self.settings())
    }

    /// Whether the queue still holds work: pending tasks or tasks in flight.
    pub fn queue_has_work(&self, queue: &str) -> Result<bool, StoreError> {
        Ok(!self.store.pending_for_queue(queue, 1)?.is_empty()
            || self.store.active_count(queue)? > 0)
    }

    /// Whether the window of `queue` is open at the current local time.
    fn queue_is_open(&self, queue: &str) -> bool {
        self.queue_schedule(queue).is_open_at(self.minute_of_day())
    }

    /// Tasks of `queue` that already occupy a slot, running here or persisted as active.
    ///
    /// The persisted states cover a task the engine restarts (`retrying`, `verifying`,
    /// `finalizing`); the in-memory map covers a task this process has just spawned but
    /// whose row is not the scheduler's business yet. Task ids make the union exact, so a
    /// task present in both places is only counted once.
    fn occupied_slots(&self, queue: &str) -> Result<usize, StoreError> {
        let mut occupied: std::collections::HashSet<String> =
            self.store.active_ids(queue)?.into_iter().collect();
        let running = self.running.lock().unwrap_or_else(|err| err.into_inner());
        for (id, task) in running.iter() {
            if task.queue == queue {
                occupied.insert(id.clone());
            }
        }
        Ok(occupied.len())
    }

    /// Free `max_active` slots of `queue` right now.
    ///
    /// `None` means the queue does not exist (SQLite rejects a task that references it,
    /// so the caller simply has nothing to start). The limit is read from the `queues`
    /// row on every call, never cached: `queue.create` and the queue update stay the only
    /// source of truth, and a raised limit applies to the next start.
    fn free_slots(&self, queue: &str) -> Result<Option<usize>, RpcError> {
        let queues = self.store.queues().map_err(store_error)?;
        let Some(max_active) = queues
            .iter()
            .find(|row| row.name == queue)
            .map(|row| row.max_active)
        else {
            return Ok(None);
        };
        let active = self.occupied_slots(queue).map_err(store_error)?;
        Ok(Some((max_active as usize).saturating_sub(active)))
    }

    /// Start every pending task of `queue` that fits right now.
    ///
    /// This is the single entry point of OpenSpec 12 §2: the scheduler and the resume
    /// path both come through here, so a closed window or a full queue starts nothing.
    /// Every step is synchronous, so the `running` mutex is never held across an `await`.
    pub fn start_pending_for_queue(&self, queue: &str) -> Result<usize, RpcError> {
        // One start decision at a time: the window, the limit and the pending list must
        // be read and acted on without another caller slipping a task into the same slot.
        let _gate = self
            .start_lock
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if !self.queue_is_open(queue) {
            tracing::debug!(queue = %queue, "queue window is closed");
            return Ok(0);
        }
        let Some(free) = self.free_slots(queue)? else {
            return Ok(0);
        };
        if free == 0 {
            return Ok(0);
        }
        let pending = self
            .store
            .pending_for_queue(queue, free)
            .map_err(store_error)?;
        let connections = self.settings().max_connections;
        let mut started = 0;
        for record in pending {
            // A completion that lands between two iterations frees a slot this loop has
            // already spent; stopping at `free` keeps the limit true either way.
            if started >= free {
                break;
            }
            match self.start_task(&record.id, connections) {
                Ok(()) => {
                    started += 1;
                    tracing::info!(task_id = %record.id, queue = %queue, "queue start");
                }
                Err(err) => tracing::warn!(
                    task_id = %record.id,
                    queue = %queue,
                    error = %err.message,
                    "the queue cannot start this task"
                ),
            }
        }
        Ok(started)
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
            quarantine_path: None,
            postprocess_exit: None,
            postprocess_message: None,
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
                "destination_dir": destination_dir.to_string_lossy(),
                "final_path": record.final_path,
            }),
        );
        if params.start_immediately {
            // `download.add` uses the same gate and the same limit as the scheduler path.
            // When no slot fits, the task stays `pending` and the completion of a running
            // one (which wakes the scheduler) starts it, in queue priority order.
            let _gate = self
                .start_lock
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if self.queue_is_open(&params.queue_name) {
                if let Some(free) = self.free_slots(&params.queue_name)? {
                    if free > 0 {
                        self.start_task(&task_id, connections)?;
                    }
                }
            }
        }
        // A closed window leaves the task `pending`: the scheduler starts it when the
        // window opens instead of the daemon ignoring the schedule on `download.add`.
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
            // S11: the confirmation window shows where the file will actually land, so the
            // reply names the effective directory and the resolved, de-duplicated path.
            "destination_dir": destination_dir.to_string_lossy(),
            "final_path": record.final_path,
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
        let running = Arc::clone(&self.running);
        let id = task_id.to_string();
        let cancel_for_task = cancel.clone();
        let wake = Arc::clone(&self.scheduler_wake);
        let queue = record.queue_name.clone();

        // Claim the slot while holding the same lock that owns the in-flight set: the
        // spawned task removes itself before releasing the lock, so a task that ends
        // immediately can neither leave a stale entry behind nor let a second caller
        // start an extra task in a slot that is already gone.
        let mut inflight = self.running.lock().unwrap_or_else(|err| err.into_inner());
        if inflight.contains_key(task_id) {
            return Err(RpcError::task_already_active(task_id));
        }
        // `downloading` is persisted before the engine exists, so the slot is visible to
        // [`Manager::occupied_slots`] even if the very first event is a terminal one.
        self.store
            .set_status(task_id, DownloadStatus::Downloading, None)
            .map_err(store_error)?;
        let join = tokio::spawn(async move {
            let engine = tokio::spawn(run_engine(cfg, spec, event_tx, cancel_for_task));
            while let Some(event) = event_rx.recv().await {
                handle_event(&store, &bus, &progress, &id, event);
            }
            let outcome = engine.await.unwrap_or_else(|err| TaskOutcome::Failed {
                error: CoreError::Network(format!("engine task panicked: {err}")),
            });
            finish_task(&store, &bus, &progress, &id, outcome).await;
            // The slot is free now: drop the in-flight entry before waking the queue
            // path, which reads it to decide how many tasks still fit, and wake the
            // scheduler instead of waiting for the next tick.
            running
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .remove(&id);
            wake.notify_one();
        });
        inflight.insert(
            task_id.to_string(),
            RunningTask {
                cancel,
                handle: join,
                queue,
            },
        );
        drop(inflight);
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
        // A site login fills the gap only when the task carries no explicit credentials.
        // It is resolved by the exact origin of the URL that will be requested, never by a
        // host alone: another port or scheme is a different origin and gets nothing.
        if !headers.contains_key(reqwest::header::AUTHORIZATION)
            && !sensitive.contains_key(reqwest::header::AUTHORIZATION)
        {
            if let Ok(Some(record)) = self.store.get(task_id) {
                let request_url = record
                    .final_url
                    .as_deref()
                    .filter(|final_url| origin_key(final_url) == origin_key(&record.url))
                    .unwrap_or(&record.url);
                if let Some(value) = self.site_login_header(request_url) {
                    sensitive.insert(reqwest::header::AUTHORIZATION, value);
                }
            }
        }
        Ok((headers, sensitive))
    }

    /// `Authorization: Basic` for the login stored for this URL's exact origin.
    ///
    /// `None` for a different port or scheme, for a legacy row without an origin and for
    /// anything that is not `http`/`https`: the password belongs to one origin only.
    pub fn site_login_header(&self, url: &str) -> Option<HeaderValue> {
        let origin = origin_key(url)?;
        let login = self.store.site_login_for_origin(&origin).ok().flatten()?;
        let password = self.secrets.lookup(&login.alias).ok().flatten()?;
        basic_header(&login.username, &password)
    }

    /// `site_login.list`: origin and user name, never the password.
    fn site_login_list(&self) -> Result<Value, RpcError> {
        let rows = self.store.site_logins().map_err(store_error)?;
        let list: Vec<Value> = rows
            .into_iter()
            .map(|row| {
                json!({
                    "origin": row.origin,
                    "domain": row.domain,
                    "username": row.username,
                    "has_password": !row.alias.is_empty(),
                })
            })
            .collect();
        Ok(Value::Array(list))
    }

    /// `site_login.set`: password to the keyring first, row to SQLite only after it lands.
    fn site_login_set(&self, params: SiteLoginSetParams) -> Result<Value, RpcError> {
        let origin = normalize_origin(&params.origin)?;
        let username = params.username.trim().to_string();
        if username.is_empty() {
            return Err(RpcError::invalid_params("username must not be empty"));
        }
        if params.password.is_empty() {
            return Err(RpcError::invalid_params("password must not be empty"));
        }
        let alias = format!("site/{origin}");
        if self.secrets.store(&alias, &params.password).is_err() {
            return Err(RpcError::new(
                1031,
                "ERR_KEYRING_UNAVAILABLE",
                "the password cannot be stored without a keyring",
            ));
        }
        let host = origin
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(&origin)
            .split(':')
            .next()
            .unwrap_or_default()
            .to_string();
        self.store
            .upsert_site_login(&SiteLoginRow {
                domain: host,
                username: username.clone(),
                alias,
                origin: origin.clone(),
            })
            .map_err(store_error)?;
        Ok(json!({ "origin": origin, "username": username }))
    }

    /// `site_login.remove`: the keyring entry and the row go together.
    fn site_login_remove(&self, params: SiteLoginOriginParams) -> Result<Value, RpcError> {
        let origin = normalize_origin(&params.origin)?;
        let _ = self.secrets.delete(&format!("site/{origin}"));
        let removed = self.store.remove_site_login(&origin).map_err(store_error)?;
        Ok(json!({ "origin": origin, "removed": removed }))
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
        // The paused task no longer holds a queue slot.
        self.notify_scheduler();
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
        tracing::info!(task_id = %task_id, "download.resume");
        // Resume rejoins the queue and goes through the same gate as the scheduler, so a
        // closed window or a full queue leaves the task pending for the next tick instead
        // of bypassing the schedule.
        self.store
            .set_status(task_id, DownloadStatus::Pending, None)
            .map_err(store_error)?;
        self.start_pending_for_queue(&record.queue_name)?;
        let status = self
            .store
            .get(task_id)
            .map_err(store_error)?
            .map(|record| record.status.as_str().to_string())
            .unwrap_or_else(|| DownloadStatus::Pending.as_str().to_string());
        Ok(json!({ "task_id": task_id, "status": status }))
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
        // A cancelled task frees its slot like a completed one.
        self.notify_scheduler();
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
        self.notify_scheduler();
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
                StoreError::Queue(message) | StoreError::Invalid(message) => {
                    RpcError::invalid_params(message)
                }
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

/// `scheme://host[:port]` with the default port removed, or `None` when the URL cannot
/// become an origin (no scheme, not http/https, credentials embedded).
fn origin_key(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let default_port = if parsed.scheme() == "https" { 443 } else { 80 };
    match parsed.port() {
        Some(port) if port != default_port => Some(format!("{}://{host}:{port}", parsed.scheme())),
        _ => Some(format!("{}://{host}", parsed.scheme())),
    }
}

/// Normalize a user-supplied origin, rejecting anything that is not a bare origin.
fn normalize_origin(input: &str) -> Result<String, RpcError> {
    let raw = input.trim();
    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let parsed = url::Url::parse(&candidate)
        .map_err(|err| RpcError::invalid_params(format!("invalid origin: {err}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(RpcError::invalid_params("origin must use http or https"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(RpcError::invalid_params(
            "an origin must not embed credentials; use site_login.set",
        ));
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(RpcError::invalid_params(
            "an origin has no path, query or fragment",
        ));
    }
    origin_key(&candidate).ok_or_else(|| RpcError::invalid_params("origin has no host"))
}

/// `Basic` credentials header for one user name and password.
fn basic_header(username: &str, password: &str) -> Option<HeaderValue> {
    let token = base64_encode(format!("{username}:{password}").as_bytes());
    HeaderValue::from_str(&format!("Basic {token}")).ok()
}

/// Standard base64 (RFC 4648) with padding.
///
/// The daemon needs exactly one encoding for an `Authorization: Basic` value, and pulling
/// a crate for ten lines would add a dependency to the `.deb`.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn store_error(err: StoreError) -> RpcError {
    RpcError::internal(err.to_string())
}

/// One file of a torrent, as `torrent.add` derives it from the probe.
struct TorrentFileSpec {
    file_index: u32,
    path: String,
    length: u64,
    selected: bool,
}

/// Read the `files[]` array `torrent.probe` reported, or the one an add already carries.
///
/// Every entry must have a non-empty `path` and a `length`; `index` falls back to the
/// array position so an add that forwards metadata without indexes still yields
/// contiguous rows, and `selected` defaults to `true` because nothing tracks a per-file
/// selection yet.
fn torrent_file_specs(files: Option<&Value>) -> Result<Vec<TorrentFileSpec>, RpcError> {
    let Some(value) = files else {
        return Ok(Vec::new());
    };
    let entries = value
        .as_array()
        .ok_or_else(|| RpcError::invalid_params("files must be an array"))?;
    let mut specs = Vec::with_capacity(entries.len());
    for (position, entry) in entries.iter().enumerate() {
        let path = entry
            .get("path")
            .and_then(|v| v.as_str())
            .filter(|path| !path.is_empty())
            .ok_or_else(|| {
                RpcError::invalid_params(format!("torrent file {position} has no path"))
            })?;
        let length = entry
            .get("length")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                RpcError::invalid_params(format!("torrent file {position} has no length"))
            })?;
        let index = entry
            .get("index")
            .and_then(|v| v.as_u64())
            .unwrap_or(position as u64);
        let file_index = u32::try_from(index).map_err(|_| {
            RpcError::invalid_params(format!("torrent file {position} has an invalid index"))
        })?;
        let selected = entry
            .get("selected")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        specs.push(TorrentFileSpec {
            file_index,
            path: path.to_string(),
            length,
            selected,
        });
    }
    Ok(specs)
}

/// Validate one reported file path and turn it into the row to persist.
///
/// `sanitize_torrent_path` rejects absolute paths, `..`, `.` and unusable components
/// against `destination_dir`; what is stored is that sanitized path relative to the
/// destination directory, which is exactly what the probe reported.
fn torrent_file_row(
    destination_dir: &Path,
    spec: &TorrentFileSpec,
) -> Result<TorrentFileInsert, RpcError> {
    let resolved = crate::torrent::sanitize_torrent_path(destination_dir, &spec.path)
        .map_err(|err| RpcError::invalid_params(format!("invalid torrent file path: {err}")))?;
    let relative = resolved.strip_prefix(destination_dir).map_err(|_| {
        RpcError::invalid_params(format!(
            "invalid torrent file path: {} escapes the destination directory",
            spec.path
        ))
    })?;
    Ok(TorrentFileInsert {
        file_index: spec.file_index,
        path: relative.to_string_lossy().into_owned(),
        length: spec.length,
        selected: spec.selected,
    })
}

/// Directory a task writes into, derived from its resolved `final_path`.
///
/// `torrent.add` resolves the payload path inside the destination directory and only the
/// path is persisted, so a later `torrent.status` recovers the directory from it.
fn task_directory(final_path: &str) -> String {
    Path::new(final_path)
        .parent()
        .map(|dir| dir.to_string_lossy().into_owned())
        .unwrap_or_else(|| final_path.to_string())
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

async fn finish_task(
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
            // Finalizing step: OpenSpec 12 §4 runs the hook once the bytes are in place and
            // before the task is published as `completed`. No lock of the manager is held
            // across the await: `running` belongs to the spawned task, which only touches it
            // after this function returns.
            run_postprocess(store, bus, task_id).await;
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

/// Run the configured post-processing hook of a task that just finished downloading.
///
/// Contract: OpenSpec 12 §4, implemented in [`crate::postprocess`]. This function only
/// wires it into the lifecycle:
///
/// * no hook configured (`postprocess_command` empty) skips the phase and leaves the task
///   exactly as it was before A1;
/// * the task moves to `finalizing` while the hook runs, so a task is never published as
///   `completed` before its post-processing ended;
/// * the hook runs on the blocking pool — it waits on a child process, which must not
///   occupy a runtime worker — and no manager lock is held across that await;
/// * the outcome is recorded instead of swallowed, and the downloaded file is never
///   touched: a failing hook leaves the bytes in place.
///
/// A non-zero exit is a recorded warning, not a failure. The branch that would fail the
/// task (`HookRun::fails_task(true)`) cannot run in 0.2: no setting enables it and the 07
/// catalog assigns no `ERR_POSTPROCESS_*` code, so no code is invented here (OpenSpec 12
/// §8).
async fn run_postprocess(store: &Arc<Store>, bus: &Arc<EventBus>, task_id: &str) {
    let settings = match store.settings() {
        Ok(settings) => settings,
        Err(err) => {
            tracing::warn!(task_id = %task_id, error = %err, "post-processing skipped: settings unreadable");
            return;
        }
    };
    if settings.postprocess_command.is_empty() {
        return;
    }
    let record = match store.get(task_id) {
        Ok(Some(record)) => record,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(task_id = %task_id, error = %err, "post-processing skipped: task unreadable");
            return;
        }
    };
    let _ = store.set_status(task_id, DownloadStatus::Finalizing, None);
    bus.publish(
        "state_changed",
        Some(task_id),
        json!({ "old_state": "downloading", "new_state": "finalizing" }),
    );

    let command = settings.postprocess_command.clone();
    let binary = command.first().cloned().unwrap_or_default();
    let target = HookTarget::from_file(Path::new(&record.final_path));
    let options = HookOptions::default();
    let run = {
        let command = command.clone();
        let target = target.clone();
        let options = options.clone();
        tokio::task::spawn_blocking(move || crate::postprocess::run(&command, &target, &options))
            .await
            .unwrap_or_else(|err| {
                HookRun::Ran(crate::postprocess::HookOutcome::not_run(
                    &binary,
                    &format!("the post-processing thread failed: {err}"),
                ))
            })
    };

    if let HookRun::Ran(outcome) = &run {
        store.record_postprocess(
            task_id,
            PostprocessRecord {
                exit_code: outcome.exit.code(),
                message: outcome.message.clone(),
            },
        );
        tracing::info!(
            task_id = %task_id,
            exit_code = ?outcome.exit.code(),
            succeeded = outcome.exit.succeeded(),
            message = %outcome.message,
            "post-processing finished"
        );
    }
    if run.fails_task(options.fail_on_error) {
        // Unreachable in 0.2 (no setting enables it), kept so the decision of OpenSpec 12
        // §4 cannot be lost when the setting and an `ERR_POSTPROCESS_*` code land.
        tracing::warn!(
            task_id = %task_id,
            "post-processing is configured to fail the task, but no error code is assigned yet"
        );
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
            let version = crate::media::check_version(&tool).map_err(Self::media_error)?;
            tracing::info!(host = %host, version = %version, "media backend");
            // Probing a playlist stops at the bound: later private entries abort `-J`.
            let probe_limit = if kind == "playlist" {
                Some(limit)
            } else {
                None
            };
            let outcome = crate::media::probe_with(&tool, url.as_str(), &kind, probe_limit)
                .await
                .map_err(Self::media_error)?;
            tracing::info!(host = %host, kind = %outcome.kind, formats = outcome.formats.len(), "media.probe");
            return serde_json::to_value(outcome)
                .map_err(|err| RpcError::internal(err.to_string()));
        }
        // Public HLS/DASH: parse the manifest and report its variants.
        let client = media_http_client()?;
        let mut probe = crate::stream_media::probe(&client, &url)
            .await
            .map_err(Self::media_error)?;
        // Same grouped ladder as YouTube: dedupe by height and cap the options (S9).
        crate::media::annotate_stream_probe(&mut probe);
        Ok(probe)
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
            // The manifest pipeline resolves the requested height instead of a format id.
            return self.add_stream_task(url, params.height).await;
        }
        let tool = crate::media::locate_tool("yt-dlp").map_err(Self::media_error)?;
        let kind_hint = if mode == "playlist" {
            "playlist"
        } else {
            "video"
        };
        let probe_limit = if kind_hint == "playlist" {
            Some(crate::media::clamp_limit(params.playlist_limit).map_err(Self::media_error)?)
        } else {
            None
        };
        let probe = crate::media::probe_with(&tool, url.as_str(), kind_hint, probe_limit)
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
        // A pre-existing file made yt-dlp resume into foreign bytes (HTTP 416). Every
        // media task therefore owns a fresh path: "Title (2).m4a" instead of "Title.m4a".
        let suggested = if is_playlist {
            title.clone()
        } else {
            format!("{title}.{extension}")
        };
        let final_path = resolve_unique_path(&dir, &suggested).map_err(|err| {
            RpcError::new(
                1005,
                "ERR_DESTINATION_UNWRITABLE",
                format!("cannot reserve {}: {err}", dir.display()),
            )
        })?;
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
            // Video quality is recorded by height: a raw `format_id` is never used to
            // pick a video stream (S9). Only the audio mode keeps the audio id.
            media_format: if mode == "audio" {
                params.format_id.clone()
            } else {
                params.height.map(|height| format!("{height}p"))
            },
            playlist_title: is_playlist.then(|| probe.title.clone()),
            playlist_count: is_playlist.then_some(limit.min(probe.playlist.count.max(1)) as u32),
            batch_id: batch_id.clone(),
            quarantine_path: None,
            postprocess_exit: None,
            postprocess_message: None,
        };
        self.store.insert_download(&record).map_err(store_error)?;
        self.bus.publish(
            "task_added",
            Some(&task_id),
            json!({
                "task_id": task_id,
                "status": "downloading",
                "url": record.url,
                "destination_dir": dir.to_string_lossy(),
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
            format: if mode == "audio" {
                params.format_id.clone()
            } else {
                None
            },
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
    async fn add_stream_task(&self, url: url::Url, height: Option<u32>) -> Result<Value, RpcError> {
        let client = media_http_client()?;
        // The requested height is honoured end to end: the manifest pipeline resolves
        // it to one variant/representation instead of falling back to the best one.
        let selection = crate::stream_media::resolve(&client, &url, height)
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
            quarantine_path: None,
            postprocess_exit: None,
            postprocess_message: None,
        };
        self.store.insert_download(&record).map_err(store_error)?;
        self.bus.publish(
            "task_added",
            Some(&task_id),
            json!({
                "task_id": task_id,
                "status": "downloading",
                "url": record.url,
                "destination_dir": dir.to_string_lossy(),
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
        let wake = Arc::clone(&self.scheduler_wake);
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
            // The stream task released its queue slot.
            wake.notify_one();
        });
        self.running
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(
                task_id.clone(),
                RunningTask {
                    cancel,
                    handle,
                    // Media and torrent tasks are added to the default queue.
                    queue: "main".to_string(),
                },
            );
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

    async fn torrent_probe(&self, params: Value) -> Result<Value, RpcError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcError::invalid_params("url parameter is required"))?;
        let client = media_http_client()?;
        crate::torrent::probe_torrent(&client, url).await
    }

    async fn torrent_add(&self, params: Value) -> Result<Value, RpcError> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcError::invalid_params("url parameter is required"))?;
        let settings = self.settings();
        let destination_dir = params
            .get("destination_dir")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&settings.download_dir));
        if !destination_dir.is_absolute() {
            return Err(RpcError::invalid_params(
                "destination_dir must be an absolute path",
            ));
        }
        let seed_after_complete = params
            .get("seed_after_complete")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // The probe is what tells the add which files to persist. An add that already
        // carries `files[]` — the caller just called `torrent.probe` — is taken at its
        // word: re-fetching a `.torrent` the caller already decoded would only repeat
        // the network cost and could disagree with what it was shown.
        let metadata = if params.get("files").is_some() {
            params.clone()
        } else {
            let client = media_http_client()?;
            crate::torrent::probe_torrent(&client, url).await?
        };
        let name = metadata
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("download.bin");
        let info_hash = metadata
            .get("info_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("0000000000000000000000000000000000000000");
        let total_bytes = metadata.get("total_bytes").and_then(|v| v.as_u64());
        let source_type = metadata
            .get("source_type")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                if url.starts_with("magnet:") {
                    "magnet"
                } else {
                    "torrent"
                }
            });

        let mut specs = torrent_file_specs(metadata.get("files"))?;
        if specs.is_empty() {
            // A magnet has no file list until its session resolves the metadata, so the
            // 0.1.x name-derived row is the only thing there is to persist. A decodable
            // `.torrent` always reports at least one file, so an empty list there is a
            // failed probe: fail the add instead of inventing an index-0 row.
            if source_type != "magnet" {
                return Err(RpcError::invalid_params(
                    "invalid .torrent: the probe reported no files",
                ));
            }
            specs.push(TorrentFileSpec {
                file_index: 0,
                path: name.to_string(),
                length: total_bytes.unwrap_or(0),
                selected: true,
            });
        }
        // Validate every path before the first write, so a hostile metainfo never leaves
        // a half-persisted task behind.
        let files: Vec<TorrentFileInsert> = specs
            .iter()
            .map(|spec| torrent_file_row(&destination_dir, spec))
            .collect::<Result<_, _>>()?;

        let task_id = uuid::Uuid::new_v4().to_string();
        if !destination_dir.exists() {
            std::fs::create_dir_all(&destination_dir).map_err(|err| {
                RpcError::new(
                    1005,
                    "ERR_DESTINATION_UNWRITABLE",
                    format!("cannot create {}: {err}", destination_dir.display()),
                )
            })?;
        }
        let final_path = resolve_unique_path(&destination_dir, name).map_err(|err| {
            RpcError::new(
                1005,
                "ERR_DESTINATION_UNWRITABLE",
                format!("cannot pick file name: {err}"),
            )
        })?;

        let record = DownloadRecord {
            id: task_id.clone(),
            url: url.to_string(),
            final_url: None,
            final_path: final_path.to_string_lossy().into_owned(),
            temp_path: None,
            total_bytes,
            downloaded_bytes: 0,
            status: DownloadStatus::Pending,
            supports_ranges: false,
            etag: None,
            last_modified: None,
            content_type: Some(
                if source_type == "magnet" {
                    "application/x-magnet"
                } else {
                    "application/x-bittorrent"
                }
                .to_string(),
            ),
            expected_checksum: None,
            error_code: None,
            error_message: None,
            retry_count: 0,
            priority: 0,
            queue_name: "main".to_string(),
            download_type: source_type.to_string(),
            created_at: None,
            completed_at: None,
            filename: name.to_string(),
            media_kind: Some(source_type.to_string()),
            media_format: None,
            playlist_title: None,
            playlist_count: None,
            batch_id: None,
            quarantine_path: None,
            postprocess_exit: None,
            postprocess_message: None,
        };

        self.store.insert_download(&record).map_err(store_error)?;
        self.store
            .insert_torrent(
                &task_id,
                if info_hash.is_empty() {
                    "0000000000000000000000000000000000000000"
                } else {
                    info_hash
                },
                url,
                seed_after_complete,
                files.len() as u32,
            )
            .map_err(store_error)?;
        self.store
            .insert_torrent_files(&task_id, &files)
            .map_err(store_error)?;

        self.bus.publish(
            "task_added",
            Some(&task_id),
            json!({
                "task_id": task_id,
                "status": "pending",
                "url": url,
                "destination_dir": destination_dir.to_string_lossy(),
                "final_path": record.final_path,
            }),
        );

        // One shared librqbit session drives every torrent: seeding stays off, so the
        // task ends as soon as the payload is complete or the user cancels it.
        let cancel = Canceller::new();
        let store = Arc::clone(&self.store);
        let bus = Arc::clone(&self.bus);
        let progress = Arc::clone(&self.progress);
        let session_dir = destination_dir.clone();
        let source = url.to_string();
        let id = task_id.clone();
        let path = record.final_path.clone();
        let task_cancel = cancel.clone();
        let wake = Arc::clone(&self.scheduler_wake);
        let handle = tokio::spawn(async move {
            let result = async {
                let session = crate::torrent::shared_session(&session_dir)
                    .await
                    .map_err(|err| (1044, err))?;
                let _ = store.set_status(&id, DownloadStatus::Downloading, None);
                bus.publish(
                    "task_started",
                    Some(&id),
                    json!({ "status": "downloading" }),
                );
                crate::torrent::run_torrent(session, source, task_cancel, |line| {
                    let _ = store.update_progress(&id, line.downloaded, Some(line.total));
                    progress
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .insert(
                            id.clone(),
                            ProgressSnapshot {
                                downloaded_bytes: line.downloaded,
                                total_bytes: Some(line.total),
                                speed_bps: 0,
                                eta_seconds: None,
                                segments: Vec::new(),
                            },
                        );
                    bus.publish(
                        "task_progress",
                        Some(&id),
                        json!({
                            "downloaded_bytes": line.downloaded,
                            "total_bytes": line.total,
                            "speed_bps": 0,
                            "eta_seconds": null,
                            "segments": [],
                        }),
                    );
                })
                .await
                .map_err(|err| (1044, err))
            }
            .await;
            progress
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .remove(&id);
            match result {
                Ok(()) => {
                    let _ = store.set_status(&id, DownloadStatus::Completed, None);
                    bus.publish(
                        "task_completed",
                        Some(&id),
                        json!({
                            "final_path": path,
                            "total_bytes": null,
                            "checksum_ok": null,
                            "media": false,
                        }),
                    );
                }
                Err((code, message)) => {
                    let cancelled = message == "cancelled";
                    let _ = store.set_status(
                        &id,
                        if cancelled {
                            DownloadStatus::Cancelled
                        } else {
                            DownloadStatus::Failed
                        },
                        if cancelled {
                            None
                        } else {
                            Some((code, message.clone()))
                        },
                    );
                    bus.publish(
                        if cancelled {
                            "task_cancelled"
                        } else {
                            "task_failed"
                        },
                        Some(&id),
                        json!({
                            "error_code": code,
                            "message": message,
                            "retryable": false,
                        }),
                    );
                }
            }
            // The torrent released its queue slot.
            wake.notify_one();
        });
        self.running
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(
                task_id.clone(),
                RunningTask {
                    cancel,
                    handle,
                    // Media and torrent tasks are added to the default queue.
                    queue: "main".to_string(),
                },
            );

        Ok(json!({
            "task_id": task_id,
            "status": "downloading",
            "filename": name,
            "info_hash": info_hash,
            // S11: the effective location of the payload, not just its name.
            "destination_dir": destination_dir.to_string_lossy(),
            "final_path": record.final_path,
        }))
    }

    fn torrent_status(&self, params: Value) -> Result<Value, RpcError> {
        let task_id = params
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcError::invalid_params("task_id parameter is required"))?;
        let record = self
            .store
            .get(task_id)
            .map_err(store_error)?
            .ok_or_else(|| RpcError::task_not_found(task_id))?;
        let torrent = self.store.get_torrent(task_id).map_err(store_error)?;
        let files = self.store.torrent_files(task_id).unwrap_or_default();

        let mut view = serde_json::to_value(self.view(&record, true))
            .map_err(|err| RpcError::internal(err.to_string()))?;
        if let Value::Object(map) = &mut view {
            if let Some(t) = torrent {
                map.insert("info_hash".to_string(), json!(t.info_hash));
                map.insert(
                    "seed_after_complete".to_string(),
                    json!(t.seed_after_complete),
                );
                map.insert("uploaded_bytes".to_string(), json!(t.uploaded_bytes));
                map.insert("peers".to_string(), json!(t.peers));
                map.insert("files_count".to_string(), json!(t.files_count));
            }
            map.insert("files".to_string(), json!(files));
            map.insert(
                "destination_dir".to_string(),
                json!(task_directory(&record.final_path)),
            );
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
        let wake = Arc::clone(&self.scheduler_wake);
        let handle = tokio::spawn(async move {
            let mut last_downloaded = 0u64;
            let result = crate::media::run_download(&tool, &plan, task_cancel, |line| {
                last_downloaded = last_downloaded.max(line.downloaded);
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
                    // The finished file is the authority: a completed task never reports
                    // 0 bytes just because yt-dlp did not print a parseable progress line.
                    let produced = crate::media::output_size(&final_path);
                    let total = produced
                        .or(Some(last_downloaded))
                        .filter(|bytes| *bytes > 0);
                    if let Some(bytes) = total {
                        let _ = store.update_progress(&id, bytes, Some(bytes));
                    }
                    let _ = store.set_status(&id, DownloadStatus::Completed, None);
                    progress
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .remove(&id);
                    progress
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .remove(&id);
                    bus.publish(
                        "task_completed",
                        Some(&id),
                        json!({
                            "final_path": final_path.to_string_lossy(),
                            "total_bytes": total,
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
            // The media task released its queue slot.
            wake.notify_one();
        });
        self.running
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(
                task_id,
                RunningTask {
                    cancel,
                    handle,
                    // Media tasks are added to the default queue.
                    queue: "main".to_string(),
                },
            );
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
                "system.open" => {
                    let parsed: TaskIdParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.store
                        .get(&parsed.task_id)
                        .map_err(store_error)?
                        .ok_or_else(|| RpcError::task_not_found(&parsed.task_id))?;
                    let gui = open_in_gui(&parsed.task_id)?;
                    Ok(json!({
                        "opened": true,
                        "task_id": parsed.task_id,
                        "gui_path": gui.to_string_lossy(),
                    }))
                }
                "extension.status" => {
                    let browsers = crate::browsers::detect();
                    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
                    let config_home = std::env::var("XDG_CONFIG_HOME")
                        .ok()
                        .filter(|value| !value.is_empty())
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| home.join(".config"));
                    Ok(json!({
                        "extension_id": crate::browsers::CHROMIUM_EXTENSION_ID,
                        "firefox_extension_id": crate::browsers::FIREFOX_EXTENSION_ID,
                        "host_path": crate::browsers::HOST_PATH,
                        "extension_version": env!("CARGO_PKG_VERSION"),
                        "shipped_archives": {
                            "chromium": crate::browsers::shipped_archive_path("chromium").map(|p| p.to_string_lossy().into_owned()),
                            "firefox": crate::browsers::shipped_archive_path("firefox").map(|p| p.to_string_lossy().into_owned()),
                        },
                        "browsers": browsers.iter().map(|browser| json!({
                            "id": browser.id,
                            "name": browser.name,
                            "family": browser.family,
                            "command": browser.command,
                            "version": browser.version,
                            "unsupported": browser.unsupported,
                            "manifest_path": browser.manifest_path.to_string_lossy(),
                            "manifest_present": browser.manifest_present,
                            "user_manifest_present": crate::browsers::user_manifest_path(
                                browser.id,
                                &home,
                                &config_home,
                            )
                            .map(|path| path.is_file())
                            .unwrap_or(false),
                            "extension_installed": crate::browsers::extension_installed(
                                browser.id,
                                &home,
                                &config_home,
                            ),
                            "prepared_path": browser.prepared_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                            "archive_path": browser.archive_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                            "state": if browser.unsupported.is_some() {
                                "unsupported"
                            } else if browser.command.is_none() {
                                "not-installed"
                            } else if crate::browsers::extension_installed(browser.id, &home, &config_home) {
                                "extension-installed"
                            } else if browser.manifest_present {
                                "native-ready"
                            } else {
                                "native-missing"
                            },
                        })).collect::<Vec<_>>(),
                    }))
                }
                "extension.prepare" => {
                    let params_obj: Value = parse(params)?;
                    let family = params_obj
                        .get("family")
                        .and_then(Value::as_str)
                        .unwrap_or("all");
                    let prepared = crate::browsers::prepare_extension(family)
                        .map_err(|err| RpcError::new(1051, "ERR_EXTENSION_PREPARE", err))?;
                    Ok(json!({
                        "prepared": prepared.into_iter().map(|(fam, path)| json!({
                            "family": fam,
                            "path": path.to_string_lossy(),
                        })).collect::<Vec<_>>()
                    }))
                }
                "extension.open" => {
                    let params_obj: Value = parse(params)?;
                    let browser_id = params_obj
                        .get("browser_id")
                        .or_else(|| params_obj.get("browser"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| RpcError::invalid_params("missing browser_id"))?;
                    crate::browsers::open_browser(browser_id)
                        .map_err(|err| RpcError::new(1052, "ERR_EXTENSION_OPEN", err))?;
                    Ok(json!({ "opened": true, "browser_id": browser_id }))
                }
                "extension.install-native" => {
                    let home = std::env::var("HOME").unwrap_or_default();
                    if home.is_empty() {
                        return Err(RpcError::new(1050, "ERR_NO_HOME", "HOME is not set"));
                    }
                    let mut installed = Vec::new();
                    let mut errors = Vec::new();
                    for browser in crate::browsers::detect() {
                        if browser.command.is_none() {
                            continue;
                        }
                        match crate::browsers::install_user_manifest(
                            &browser,
                            std::path::Path::new(&home),
                        ) {
                            Ok(path) => installed.push(json!({
                                "id": browser.id,
                                "path": path.to_string_lossy(),
                            })),
                            Err(message) => errors.push(json!({
                                "id": browser.id,
                                "message": message,
                            })),
                        }
                    }
                    Ok(json!({ "installed": installed, "errors": errors }))
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
                "torrent.probe" => {
                    let parsed = parse(params)?;
                    self.torrent_probe(parsed).await
                }
                "torrent.add" => {
                    let parsed = parse(params)?;
                    self.torrent_add(parsed).await
                }
                "torrent.status" => {
                    let parsed = parse(params)?;
                    self.torrent_status(parsed)
                }
                "site_login.list" => self.site_login_list(),
                "site_login.set" => {
                    let parsed: SiteLoginSetParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.site_login_set(parsed)
                }
                "site_login.remove" => {
                    let parsed: SiteLoginOriginParams = serde_json::from_value(parse(params)?)
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    self.site_login_remove(parsed)
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

/// Whether `path` is a regular file this user can execute.
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// GUI binary used by `system.open`.
///
/// `SEGMENTA_GUI` wins so a development build or a test can point at another
/// binary; otherwise the GUI is expected next to the daemon, which is where the
/// package installs both.
fn gui_binary() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("SEGMENTA_GUI") {
        let path = PathBuf::from(value);
        return is_executable(&path).then_some(path);
    }
    let current = std::env::current_exe().ok()?;
    let sibling = current.parent()?.join("segmenta-gui");
    is_executable(&sibling).then_some(sibling)
}

/// Brings the GUI forward with one task focused.
///
/// The argv is fixed and no shell is involved, so a task id can never turn into
/// a command. The child is reaped by a detached thread: the daemon outlives the
/// GUI and must not accumulate zombies.
fn open_in_gui(task_id: &str) -> Result<PathBuf, RpcError> {
    let gui = gui_binary().ok_or_else(|| {
        RpcError::new(
            1053,
            "ERR_GUI_NOT_FOUND",
            "segmenta-gui is not installed next to the daemon",
        )
    })?;
    let mut child = std::process::Command::new(&gui)
        .arg("--show-task")
        .arg(task_id)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| {
            RpcError::new(
                1053,
                "ERR_GUI_NOT_FOUND",
                format!("cannot run {}: {err}", gui.display()),
            )
        })?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(gui)
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
