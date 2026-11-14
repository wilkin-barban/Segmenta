//! Download engine: one coordinator driving one worker per segment.
//!
//! The module keeps three invariants that the rest of the daemon relies on
//! (OpenSpec 02, 06 and 08):
//!
//! * **Coverage.** The union of the planned ranges is exactly `[0, total)` with no gap
//!   and no overlap, at every instant. A hot split only ever shrinks a donor
//!   (`end := midpoint`) and creates `[midpoint + 1, original_end]` *after* the donor
//!   acknowledged the new boundary, so the invariant survives the race.
//! * **Offset honesty.** [`SegmentStateRecord::next`] is the next byte that will be
//!   written, never a byte that is not on disk: `pwrite_all` runs before the atomic
//!   `next` is published, and a checkpoint runs `sync_data()` before the `Checkpoint`
//!   event is emitted (data before database, always).
//! * **Clamped reads.** Every chunk is trimmed with [`planner::clamp_read`] before it is
//!   written, so a worker can never write past its current `end`, even when a split
//!   shrank that `end` while the chunk was already in flight.
//!
//! Shape: [`run`] probes, plans and retries a restart; [`coordinate`] owns the worker
//! tasks, the progress ticker, checkpoints and the split handshake; [`SegmentWorker`]
//! is the only network read loop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::header::CONTENT_RANGE;
use tokio::sync::{mpsc, oneshot, Notify};
use tokio::task::JoinSet;
use url::Url;

use crate::checksum;
use crate::disk::DiskWriter;
use crate::error::{CoreError, StatusDecision};
use crate::planner::{self, DonorView, SegmentRange, SplitOutcome};
use crate::probe::{self, HttpConfig, ProbeInfo, ProbeOutcome, Session};
use crate::ratelimit::TokenBucket;
use crate::retry;

/// Progress cadence: at most four events per second per task (S16).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// Durability cadence: a checkpoint every two seconds...
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(2);
/// ...or every five mebibytes of new data, whichever comes first (S13/S6).
const CHECKPOINT_BYTES: u64 = 5 * 1024 * 1024;
/// A donor that does not answer a split request in time is skipped.
const SPLIT_ACK_TIMEOUT: Duration = Duration::from_secs(5);
/// Depth of the per-worker split control channel (one request in flight, some slack).
const SPLIT_CHANNEL_CAPACITY: usize = 4;

/// Engine tuning knobs.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Connections to use when the plan is derived from the file size (1-32).
    pub connections: u32,
    /// Consecutive failures tolerated per worker before the task pauses.
    pub max_retries: u32,
    /// HTTP behaviour shared by the probe and the workers.
    pub http: HttpConfig,
    /// Global write limit in bytes per second; `None` means unlimited.
    pub speed_limit_bps: Option<u64>,
    /// Read/write buffer granularity.
    pub chunk_size: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            connections: 16,
            max_retries: retry::MAX_RETRIES,
            http: HttpConfig::default(),
            speed_limit_bps: None,
            chunk_size: 64 * 1024,
        }
    }
}

/// Durable per-segment offsets, mirroring one row of `segments`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentStateRecord {
    /// Segment identifier, stable across resumes.
    pub id: u32,
    /// First byte of the segment.
    pub start: u64,
    /// Last byte of the segment, inclusive.
    pub end: u64,
    /// Next byte the worker will write.
    pub next: u64,
}

/// Everything needed to continue a paused download.
#[derive(Debug, Clone)]
pub struct ResumeState {
    /// Size of the remote entity.
    pub total_bytes: u64,
    /// Whether the last plan was segmented.
    pub supports_ranges: bool,
    /// `ETag` seen when the file was planned.
    pub etag: Option<String>,
    /// `Last-Modified` seen when the file was planned.
    pub last_modified: Option<String>,
    /// Per-segment offsets.
    pub segments: Vec<SegmentStateRecord>,
    /// Bytes of a sequential (single-stream) download already on disk.
    pub single_stream_written: u64,
    /// Error that paused the task, if any.
    pub last_error: Option<CoreError>,
}

/// One download request.
#[derive(Debug, Clone)]
pub struct DownloadSpec {
    /// Source URL, already sanitized by the caller.
    pub url: Url,
    /// Final destination; must not exist for a fresh download.
    pub path: PathBuf,
    /// Optional expected SHA-256, lowercase hex.
    pub expected_sha256: Option<String>,
    /// Previous state, when resuming.
    pub resume: Option<ResumeState>,
    /// Fixed plan, used by tests instead of the size-based heuristic.
    pub plan_override: Option<Vec<SegmentRange>>,
}

/// Progress and lifecycle notifications.
#[derive(Debug)]
pub enum EngineEvent {
    /// A capability probe started.
    Probing,
    /// The probe learned the transfer mode.
    ProbingFinished {
        /// Declared size, when the server sent one.
        total_bytes: Option<u64>,
        /// Whether ranges work.
        supports_ranges: bool,
        /// Strong validator.
        etag: Option<String>,
        /// Weak validator.
        last_modified: Option<String>,
    },
    /// A worker took a range.
    SegmentStarted {
        /// Segment identifier.
        id: u32,
        /// First byte.
        start: u64,
        /// Last byte, inclusive.
        end: u64,
    },
    /// A donor handed half of its range to an idle worker.
    SegmentSplit {
        /// Donor segment.
        donor_id: u32,
        /// New segment.
        new_id: u32,
        /// Byte the donor now ends at.
        midpoint: u64,
        /// End of the donor before the split.
        original_end: u64,
    },
    /// A segment wrote its last byte.
    SegmentCompleted {
        /// Segment identifier.
        id: u32,
        /// First byte.
        start: u64,
        /// Last byte, inclusive.
        end: u64,
    },
    /// Data was synced to disk; the database may now record these offsets.
    Checkpoint {
        /// Durable per-segment offsets.
        segments: Vec<SegmentStateRecord>,
        /// Bytes on disk.
        downloaded_bytes: u64,
    },
    /// Throttled progress tick.
    Progress {
        /// Bytes on disk.
        downloaded_bytes: u64,
        /// Total size, when known.
        total_bytes: Option<u64>,
        /// Bytes per second over the tick window.
        speed_bps: u64,
        /// Remaining seconds, when known.
        eta_seconds: Option<u64>,
        /// Per-segment offsets.
        segments: Vec<SegmentStateRecord>,
    },
    /// The remote object changed and the download started over.
    Restarted {
        /// Why the engine restarted.
        reason: String,
    },
    /// The digest is being computed.
    Verifying,
    /// The file is complete.
    Completed {
        /// Final path.
        path: PathBuf,
        /// Bytes written.
        total_bytes: u64,
        /// Digest result when a checksum was requested.
        checksum_ok: Option<bool>,
        /// Final segment table, so the daemon can persist a completed checkpoint.
        segments: Vec<SegmentStateRecord>,
    },
}

/// How a run ended.
#[derive(Debug)]
pub enum TaskOutcome {
    /// The file is complete.
    Completed {
        /// Bytes written.
        total_bytes: u64,
        /// Digest result when a checksum was requested.
        checksum_ok: Option<bool>,
        /// Final segment table, so the daemon can persist a completed checkpoint.
        segments: Vec<SegmentStateRecord>,
    },
    /// Stopped but resumable.
    Paused {
        /// State the daemon must persist.
        state: ResumeState,
    },
    /// Terminal failure.
    Failed {
        /// Cause.
        error: CoreError,
    },
}

/// Cooperative cancellation shared by the daemon and the engine.
#[derive(Debug, Clone)]
pub struct Canceller {
    flag: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl Canceller {
    /// Fresh, not cancelled.
    pub fn new() -> Self {
        Canceller {
            flag: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Request cancellation. Idempotent and safe from any task.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
        // Keep a permit for a waiter that has not registered yet.
        self.notify.notify_one();
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Resolve once cancellation is requested.
    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            let waiter = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            waiter.await;
        }
    }
}

impl Default for Canceller {
    fn default() -> Self {
        Canceller::new()
    }
}

/// Download `spec` into `spec.path`, reporting through `events`.
///
/// Returns [`TaskOutcome::Paused`] with a complete [`ResumeState`] whenever the task
/// can continue later (user pause, exhausted retries, missing credentials, disk full)
/// and [`TaskOutcome::Failed`] only for terminal errors.
pub async fn run(
    cfg: EngineConfig,
    spec: DownloadSpec,
    events: mpsc::Sender<EngineEvent>,
    cancel: Canceller,
) -> TaskOutcome {
    let bucket = Arc::new(TokenBucket::new(cfg.speed_limit_bps));
    let _ = events.send(EngineEvent::Probing).await;
    let mut info = match probe_step(&cfg, &spec, &cancel).await {
        ProbeStep::Ready(info) => info,
        ProbeStep::Cancelled => {
            return TaskOutcome::Paused {
                state: paused_from_spec(&spec, None),
            }
        }
        ProbeStep::Stopped(outcome) => return outcome,
    };
    emit_probing_finished(&events, &info).await;

    // A resume continues only on the file we wrote; if it is gone, start clean.
    let mut open = if spec.resume.is_some() && spec.path.exists() {
        OpenMode::Resume
    } else {
        OpenMode::Create
    };
    let mut resume = spec.resume.as_ref();
    let mut restarts = 0u32;

    // The validators/size of a paused task must still describe the remote object,
    // otherwise the local bytes belong to another generation (OpenSpec 06 §2.2).
    if let (Some(state), OpenMode::Resume) = (resume, open) {
        if let Some(reason) = stale_reason(state, &info) {
            restarts += 1;
            open = OpenMode::Restart;
            resume = None;
            let _ = events.send(EngineEvent::Restarted { reason }).await;
        }
    }

    loop {
        let job = Job {
            cfg: &cfg,
            spec: &spec,
            info: &info,
            open,
            resume,
            bucket: &bucket,
            events: &events,
            cancel: &cancel,
        };
        match download_generation(&job).await {
            Cycle::Outcome(outcome) => return outcome,
            Cycle::Restart(reason) => {
                if restarts >= 1 {
                    // Never mix two generations: a second change within one run is fatal.
                    return TaskOutcome::Failed {
                        error: CoreError::Unsupported("remote changed twice".to_string()),
                    };
                }
                restarts += 1;
                let _ = events.send(EngineEvent::Restarted { reason }).await;
                let _ = events.send(EngineEvent::Probing).await;
                match probe_step(&cfg, &spec, &cancel).await {
                    ProbeStep::Ready(fresh) => {
                        emit_probing_finished(&events, &fresh).await;
                        info = fresh;
                    }
                    ProbeStep::Cancelled => {
                        return TaskOutcome::Paused {
                            state: paused_from_spec(&spec, None),
                        }
                    }
                    ProbeStep::Stopped(outcome) => return outcome,
                }
                open = OpenMode::Restart;
                resume = None;
            }
        }
    }
}

impl DownloadSpec {
    /// Whether a paused task is being continued.
    fn resuming(&self) -> bool {
        self.resume.is_some()
    }
}

// ---------------------------------------------------------------------------
// Probe
// ---------------------------------------------------------------------------

/// Result of the probe phase.
enum ProbeStep {
    Ready(ProbeInfo),
    Cancelled,
    Stopped(TaskOutcome),
}

async fn probe_step(cfg: &EngineConfig, spec: &DownloadSpec, cancel: &Canceller) -> ProbeStep {
    let mut attempt = 0u32;
    loop {
        if cancel.is_cancelled() {
            return ProbeStep::Cancelled;
        }
        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => return ProbeStep::Cancelled,
            outcome = probe::probe(&cfg.http, &spec.url) => outcome,
        };
        match outcome {
            Ok(ProbeOutcome::Ready(info)) => return ProbeStep::Ready(info),
            Ok(ProbeOutcome::Retry { retry_after }) => {
                if attempt >= cfg.max_retries {
                    return ProbeStep::Stopped(TaskOutcome::Failed {
                        error: CoreError::Network(format!(
                            "probe still failing after {attempt} retries"
                        )),
                    });
                }
                let delay = retry_after.unwrap_or_else(|| retry::backoff_delay(attempt));
                attempt += 1;
                if !sleep_cancellable(delay, cancel).await {
                    return ProbeStep::Cancelled;
                }
            }
            Ok(ProbeOutcome::Pause(error)) => {
                return ProbeStep::Stopped(if spec.resuming() {
                    TaskOutcome::Paused {
                        state: paused_from_spec(spec, Some(error)),
                    }
                } else {
                    TaskOutcome::Failed { error }
                });
            }
            Ok(ProbeOutcome::Fail(error)) => {
                return ProbeStep::Stopped(TaskOutcome::Failed { error })
            }
            Err(error) => {
                if !error.is_retryable() || attempt >= cfg.max_retries {
                    return ProbeStep::Stopped(TaskOutcome::Failed { error });
                }
                let delay = retry::backoff_delay(attempt);
                attempt += 1;
                if !sleep_cancellable(delay, cancel).await {
                    return ProbeStep::Cancelled;
                }
            }
        }
    }
}

/// `true` when the sleep completed, `false` when the task was cancelled meanwhile.
async fn sleep_cancellable(delay: Duration, cancel: &Canceller) -> bool {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => false,
        _ = tokio::time::sleep(delay) => true,
    }
}

async fn emit_probing_finished(events: &mpsc::Sender<EngineEvent>, info: &ProbeInfo) {
    let _ = events
        .send(EngineEvent::ProbingFinished {
            total_bytes: info.mode.total_bytes(),
            supports_ranges: info.mode.supports_ranges(),
            etag: info.etag.clone(),
            last_modified: info.last_modified.clone(),
        })
        .await;
}

/// Why a resume cannot continue on the bytes already on disk.
fn stale_reason(state: &ResumeState, info: &ProbeInfo) -> Option<String> {
    if state.supports_ranges != info.mode.supports_ranges() {
        return Some("the server changed how it serves ranges".to_string());
    }
    if !state.supports_ranges {
        return None;
    }
    if let Some(total) = info.mode.total_bytes() {
        if total != state.total_bytes {
            return Some(format!(
                "the remote size changed from {} to {total} bytes",
                state.total_bytes
            ));
        }
    }
    if let (Some(planned), Some(current)) = (&state.etag, &info.etag) {
        if planned != current {
            return Some(format!(
                "the remote validator changed from {planned} to {current}"
            ));
        }
    }
    None
}

/// Whether a `Content-Range` on a `200` response describes exactly the requested range.
///
/// iCloud answers a full-range request with `200` instead of `206`; the body and the
/// Content-Range are still the requested bytes. Requiring `start == next`, `end >= end`
/// and the planned total keeps this from blessing a response for another generation.
fn content_range_covers(
    value: &str,
    next: u64,
    end: u64,
    total: Option<u64>,
    length: Option<u64>,
) -> bool {
    let Ok((start, content_end, content_total)) = probe::parse_content_range(value) else {
        return false;
    };
    let expected_len = end.saturating_sub(next).saturating_add(1);
    start == next
        && content_end >= end
        && content_total == total
        && length.map(|length| length == expected_len).unwrap_or(true)
}

/// The state a paused task must persist, derived only from the caller's spec.
fn paused_from_spec(spec: &DownloadSpec, last_error: Option<CoreError>) -> ResumeState {
    match &spec.resume {
        Some(state) => ResumeState {
            last_error,
            ..state.clone()
        },
        None => ResumeState {
            total_bytes: 0,
            supports_ranges: false,
            etag: None,
            last_modified: None,
            segments: Vec::new(),
            single_stream_written: 0,
            last_error,
        },
    }
}

// ---------------------------------------------------------------------------
// One generation: open the file, build the plan, run the coordinator
// ---------------------------------------------------------------------------

/// How the destination file is opened for this generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenMode {
    /// Fresh download: the file must not exist.
    Create,
    /// Continue a paused download on the file already on disk.
    Resume,
    /// The remote changed: drop every byte and start over.
    Restart,
}

/// Everything one generation needs; bundled because it is threaded through
/// `download_generation` unchanged.
struct Job<'a> {
    cfg: &'a EngineConfig,
    spec: &'a DownloadSpec,
    info: &'a ProbeInfo,
    open: OpenMode,
    resume: Option<&'a ResumeState>,
    bucket: &'a Arc<TokenBucket>,
    events: &'a mpsc::Sender<EngineEvent>,
    cancel: &'a Canceller,
}

/// Result of one generation.
enum Cycle {
    /// The generation reached a final state for this run.
    Outcome(TaskOutcome),
    /// The remote changed; `run` may restart once.
    Restart(String),
}

async fn download_generation(job: &Job<'_>) -> Cycle {
    let sequential = !job.info.mode.supports_ranges();
    let declared_total = job.info.mode.total_bytes();
    let resume = match job.open {
        OpenMode::Resume => job.resume,
        _ => None,
    };

    // ---- destination ------------------------------------------------------
    let file = match job.open {
        OpenMode::Create => match DiskWriter::create_new(&job.spec.path, declared_total) {
            Ok((writer, _)) => Arc::new(writer),
            // The daemon resolves collisions before calling the engine (S12).
            Err(error) => {
                return Cycle::Outcome(TaskOutcome::Failed { error });
            }
        },
        OpenMode::Resume => {
            let total = declared_total
                .or_else(|| resume.map(|state| state.total_bytes))
                .unwrap_or(0);
            match DiskWriter::open_resume(&job.spec.path, total) {
                Ok((writer, _)) => Arc::new(writer),
                Err(error) => return Cycle::Outcome(TaskOutcome::Failed { error }),
            }
        }
        OpenMode::Restart => {
            match DiskWriter::open_resume(&job.spec.path, declared_total.unwrap_or(0)) {
                Ok((writer, _)) => {
                    let writer = Arc::new(writer);
                    if let Err(error) = writer.truncate() {
                        return Cycle::Outcome(TaskOutcome::Failed { error });
                    }
                    writer
                }
                Err(error) => return Cycle::Outcome(TaskOutcome::Failed { error }),
            }
        }
    };

    // ---- plan -------------------------------------------------------------
    let (records, total) = if sequential {
        // A sequential stream cannot continue from an offset: continuing would need
        // `Range`, which the server just refused. Rather than corrupt the file we
        // truncate and start at byte zero (OpenSpec 06 §5.1), keeping the task usable.
        if resume.is_some_and(|state| state.single_stream_written > 0) {
            if let Err(error) = file.truncate() {
                return Cycle::Outcome(TaskOutcome::Failed { error });
            }
            let _ = job
                .events
                .send(EngineEvent::Restarted {
                    reason: "sequential downloads restart from byte zero".to_string(),
                })
                .await;
        }
        let range = SegmentRange {
            start: 0,
            end: declared_total
                .map(|total| total.saturating_sub(1))
                .unwrap_or(u64::MAX),
        };
        (range_records(&[range]), declared_total)
    } else {
        let total = resume
            .map(|state| state.total_bytes)
            .or(declared_total)
            .unwrap_or(0);
        let records = match resume {
            Some(state) if !state.segments.is_empty() => state.segments.clone(),
            _ => match &job.spec.plan_override {
                Some(ranges) => range_records(ranges),
                None => range_records(&planner::initial_segments(total, job.cfg.connections)),
            },
        };
        (records, Some(total))
    };

    // ---- environment ------------------------------------------------------
    // A resume keeps the validators that match the bytes on disk; a fresh download
    // uses the probe's.
    let (etag, last_modified) = match resume {
        Some(state) => (
            state.etag.clone().or_else(|| job.info.etag.clone()),
            state
                .last_modified
                .clone()
                .or_else(|| job.info.last_modified.clone()),
        ),
        None => (job.info.etag.clone(), job.info.last_modified.clone()),
    };
    let env = Arc::new(Env {
        http: job.cfg.http.clone(),
        url: job.info.final_url.clone(),
        if_range: etag.clone().or_else(|| last_modified.clone()),
        etag,
        last_modified,
        total,
        max_retries: job.cfg.max_retries,
        chunk_size: job.cfg.chunk_size.max(1),
        sequential,
        file,
        bucket: Arc::clone(job.bucket),
        halt: Canceller::new(),
    });
    let shared = Arc::new(Shared {
        env,
        events: job.events.clone(),
        segments: Mutex::new(Vec::new()),
        uncheckpointed: AtomicU64::new(0),
    });
    coordinate(
        shared,
        Plan {
            records,
            total,
            splits: !sequential,
        },
        job.spec,
        job.cancel,
    )
    .await
}

fn range_records(ranges: &[SegmentRange]) -> Vec<SegmentStateRecord> {
    ranges
        .iter()
        .enumerate()
        .map(|(index, range)| SegmentStateRecord {
            id: index as u32,
            start: range.start,
            end: range.end,
            next: range.start,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// Which network shape a worker uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerKind {
    /// Segmented: `Range: bytes=next-end` plus `If-Range`.
    Range,
    /// Single stream: one unqualified `GET`, read to `EOF`.
    Sequential,
}

/// A control message for a cooperative split.
struct SplitRequest {
    /// Identifier the coordinator will give the new segment.
    new_id: u32,
    /// Byte the donor must clamp its end at.
    midpoint: u64,
    /// Donor end the coordinator based the midpoint on.
    original_end: u64,
    /// Where the donor reports the new boundary.
    reply: oneshot::Sender<SplitAck>,
}

/// The donor's answer to a split request.
struct SplitAck {
    /// `true` when the donor already clamped its end.
    accepted: bool,
    /// First byte of the new segment, valid only when accepted.
    clamped_next: u64,
}

/// A message from a worker to the coordinator.
enum WorkerMsg {
    /// The worker wrote its last byte.
    Completed { id: u32 },
    /// The worker cannot continue.
    Stopped {
        id: u32,
        error: CoreError,
        /// `true` when the task should pause (and stay resumable) instead of failing.
        resumable: bool,
    },
    /// The remote object changed: every worker must stop.
    Restart { reason: String },
}

/// Live offsets of one segment; the atomics are the only cross-task channel.
struct SegHandle {
    id: u32,
    start: u64,
    end: AtomicU64,
    next: AtomicU64,
}

impl SegHandle {
    fn new(record: &SegmentStateRecord) -> Arc<SegHandle> {
        Arc::new(SegHandle {
            id: record.id,
            start: record.start,
            end: AtomicU64::new(record.end),
            next: AtomicU64::new(record.next),
        })
    }

    fn end(&self) -> u64 {
        self.end.load(Ordering::SeqCst)
    }

    fn next(&self) -> u64 {
        self.next.load(Ordering::SeqCst)
    }

    fn record(&self) -> SegmentStateRecord {
        SegmentStateRecord {
            id: self.id,
            start: self.start,
            end: self.end(),
            next: self.next(),
        }
    }

    fn donor_view(&self) -> DonorView {
        DonorView {
            id: self.id,
            start: self.start,
            end: self.end(),
            next: self.next(),
        }
    }
}

/// Per-generation worker environment.
struct Env {
    http: HttpConfig,
    url: Url,
    etag: Option<String>,
    last_modified: Option<String>,
    if_range: Option<String>,
    /// Declared size, when the server sent one.
    total: Option<u64>,
    max_retries: u32,
    chunk_size: usize,
    sequential: bool,
    file: Arc<DiskWriter>,
    bucket: Arc<TokenBucket>,
    /// Internal stop flag: a stale-cache restart or a fatal worker error.
    halt: Canceller,
}

/// State shared by the coordinator and the workers of one generation.
struct Shared {
    env: Arc<Env>,
    events: mpsc::Sender<EngineEvent>,
    segments: Mutex<Vec<Arc<SegHandle>>>,
    /// Bytes written since the last durable checkpoint.
    uncheckpointed: AtomicU64,
}

impl Shared {
    async fn emit(&self, event: EngineEvent) {
        // A closed channel means nobody listens; it must never fail a download.
        let _ = self.events.send(event).await;
    }

    fn push(&self, seg: Arc<SegHandle>) {
        self.segments
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(seg);
    }

    fn find(&self, id: u32) -> Option<Arc<SegHandle>> {
        self.segments
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|seg| seg.id == id)
            .cloned()
    }

    fn records(&self) -> Vec<SegmentStateRecord> {
        let segments = self
            .segments
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut records: Vec<SegmentStateRecord> =
            segments.iter().map(|seg| seg.record()).collect();
        records.sort_by_key(|record| record.id);
        records
    }

    /// Bytes on disk for this generation, including what a resume already had.
    fn downloaded(&self) -> u64 {
        let segments = self
            .segments
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        segments
            .iter()
            .map(|seg| seg.next().saturating_sub(seg.start))
            .sum()
    }
}

fn started_event(env: &Env, record: &SegmentStateRecord) -> EngineEvent {
    // A sequential download has one pseudo-segment; its end is a placeholder until
    // `SegmentCompleted` reports how many bytes actually arrived.
    let end = if env.sequential {
        env.total.map(|total| total.saturating_sub(1)).unwrap_or(0)
    } else {
        record.end
    };
    EngineEvent::SegmentStarted {
        id: record.id,
        start: record.start,
        end,
    }
}

fn completed_event(env: &Env, seg: &SegHandle) -> EngineEvent {
    let record = seg.record();
    let end = if env.sequential {
        record.next.saturating_sub(1).max(record.start)
    } else {
        record.end
    };
    EngineEvent::SegmentCompleted {
        id: record.id,
        start: record.start,
        end,
    }
}

// ---------------------------------------------------------------------------
// Coordinator
// ---------------------------------------------------------------------------

struct Plan {
    records: Vec<SegmentStateRecord>,
    total: Option<u64>,
    /// Hot split is only meaningful for segmented downloads.
    splits: bool,
}

enum Stop {
    Finished,
    Cancelled,
    Worker { error: CoreError, resumable: bool },
    Restart(String),
}

/// Spawn one worker per planned segment, then supervise them.
async fn coordinate(
    shared: Arc<Shared>,
    plan: Plan,
    spec: &DownloadSpec,
    cancel: &Canceller,
) -> Cycle {
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel();
    let mut workers: JoinSet<()> = JoinSet::new();
    let mut live: HashMap<u32, mpsc::Sender<SplitRequest>> = HashMap::new();
    let mut next_id = 1u32;

    for record in &plan.records {
        let seg = SegHandle::new(record);
        next_id = next_id.max(record.id.saturating_add(1));
        shared.push(Arc::clone(&seg));
        shared.emit(started_event(&shared.env, record)).await;
        if record.next > record.end {
            // Already complete on disk (resume of a finished segment).
            continue;
        }
        let (control, control_rx) = mpsc::channel(SPLIT_CHANNEL_CAPACITY);
        live.insert(record.id, control);
        spawn_worker(
            &mut workers,
            Arc::clone(&shared),
            seg,
            control_rx,
            msg_tx.clone(),
            cancel.clone(),
        );
    }

    let mut active = live.len();
    let mut reporter = Reporter::new(Arc::clone(&shared), plan.total);
    let mut ticker = tokio::time::interval(PROGRESS_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick fires immediately: drop it so no zero-byte progress is emitted.
    ticker.tick().await;

    let stop = loop {
        if active == 0 {
            break Stop::Finished;
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break Stop::Cancelled,
            message = msg_rx.recv() => match message {
                Some(WorkerMsg::Completed { id }) => {
                    live.remove(&id);
                    active = active.saturating_sub(1);
                    if let Some(seg) = shared.find(id) {
                        shared.emit(completed_event(&shared.env, &seg)).await;
                    }
                    // The idle worker takes half of the largest donor, if any is worth it.
                    if active > 0
                        && plan.splits
                        && try_split(&shared, &mut live, &mut workers, &mut next_id, &msg_tx, cancel).await
                    {
                        active += 1;
                    }
                }
                Some(WorkerMsg::Stopped { id, error, resumable }) => {
                    live.remove(&id);
                    break Stop::Worker { error, resumable };
                }
                Some(WorkerMsg::Restart { reason }) => break Stop::Restart(reason),
                None => break Stop::Finished,
            },
            // A worker that died without a message (a panic) must never hang the task.
            finished = workers.join_next(), if !workers.is_empty() => {
                if let Some(Err(error)) = finished {
                    break Stop::Worker {
                        error: CoreError::Unsupported(format!("worker task failed: {error}")),
                        resumable: true,
                    };
                }
            }
            _ = ticker.tick() => reporter.tick().await,
        }
    };

    // Stop every survivor and wait for its last published offset before touching the file.
    shared.env.halt.cancel();
    while workers.join_next().await.is_some() {}
    let downloaded = shared.downloaded();

    match stop {
        Stop::Finished => {
            // A final tick so consumers see `downloaded_bytes == total`.
            reporter.tick().await;
            finalize(&shared, spec, plan.total, downloaded).await
        }
        Stop::Cancelled => {
            checkpoint(&shared, downloaded).await;
            Cycle::Outcome(TaskOutcome::Paused {
                state: resumable_state(&shared, plan.total, downloaded, None),
            })
        }
        Stop::Worker { error, resumable } => {
            checkpoint(&shared, downloaded).await;
            Cycle::Outcome(if resumable {
                TaskOutcome::Paused {
                    state: resumable_state(&shared, plan.total, downloaded, Some(error)),
                }
            } else {
                TaskOutcome::Failed { error }
            })
        }
        Stop::Restart(reason) => {
            if let Err(error) = shared.env.file.truncate() {
                return Cycle::Outcome(TaskOutcome::Failed { error });
            }
            Cycle::Restart(reason)
        }
    }
}

fn spawn_worker(
    workers: &mut JoinSet<()>,
    shared: Arc<Shared>,
    seg: Arc<SegHandle>,
    control: mpsc::Receiver<SplitRequest>,
    msg_tx: mpsc::UnboundedSender<WorkerMsg>,
    cancel: Canceller,
) {
    let next = seg.next();
    workers.spawn(
        SegmentWorker {
            shared,
            seg,
            msg_tx,
            cancel,
            next,
            failures: 0,
        }
        .run(control),
    );
}

/// Run the split handshake with the largest eligible donor.
///
/// The new worker is only spawned after [`SplitAck`], so the donor has already
/// clamped its own end: at no instant do two workers own the same byte, and the
/// union of ranges stays `[0, total)`.
async fn try_split(
    shared: &Arc<Shared>,
    live: &mut HashMap<u32, mpsc::Sender<SplitRequest>>,
    workers: &mut JoinSet<()>,
    next_id: &mut u32,
    msg_tx: &mpsc::UnboundedSender<WorkerMsg>,
    cancel: &Canceller,
) -> bool {
    let mut candidates: Vec<DonorView> = live
        .keys()
        .filter_map(|id| shared.find(*id).map(|seg| seg.donor_view()))
        .collect();
    candidates.sort_by_key(|view| view.id);

    while let Some(choice) = planner::choose_donor(&candidates) {
        let (Some(sender), Some(seg)) = (live.get(&choice.id).cloned(), shared.find(choice.id))
        else {
            return false;
        };
        let original_end = seg.end();
        let id = *next_id;
        let (reply, reply_rx) = oneshot::channel();
        let request = SplitRequest {
            new_id: id,
            midpoint: choice.midpoint,
            original_end,
            reply,
        };
        // A donor that cannot take the message now belongs to another split; skip it.
        if sender.try_send(request).is_err() {
            candidates.retain(|view| view.id != choice.id);
            continue;
        }
        let ack = tokio::select! {
            biased;
            _ = cancel.cancelled() => return false,
            _ = shared.env.halt.cancelled() => return false,
            ack = tokio::time::timeout(SPLIT_ACK_TIMEOUT, reply_rx) => ack,
        };
        let clamped_next = match ack {
            Ok(Ok(SplitAck {
                accepted: true,
                clamped_next,
            })) if clamped_next <= original_end => clamped_next,
            _ => {
                candidates.retain(|view| view.id != choice.id);
                continue;
            }
        };

        // The donor registered the new range together with its own clamp, so the table
        // is already consistent; the coordinator only has to start the worker.
        let Some(fresh) = shared.find(id).filter(|seg| seg.start == clamped_next) else {
            return false;
        };
        *next_id = next_id.saturating_add(1);
        let (control, control_rx) = mpsc::channel(SPLIT_CHANNEL_CAPACITY);
        live.insert(id, control);
        shared
            .emit(EngineEvent::SegmentSplit {
                donor_id: choice.id,
                new_id: id,
                midpoint: choice.midpoint,
                original_end,
            })
            .await;
        shared
            .emit(started_event(&shared.env, &fresh.record()))
            .await;
        spawn_worker(
            workers,
            Arc::clone(shared),
            fresh,
            control_rx,
            msg_tx.clone(),
            cancel.clone(),
        );
        return true;
    }
    false
}

/// Sync the data written so far, then publish the durable offsets (data before database).
async fn checkpoint(shared: &Arc<Shared>, downloaded: u64) {
    let _ = shared.env.file.sync_data();
    shared.uncheckpointed.store(0, Ordering::SeqCst);
    shared
        .emit(EngineEvent::Checkpoint {
            segments: shared.records(),
            downloaded_bytes: downloaded,
        })
        .await;
}

fn resumable_state(
    shared: &Shared,
    total: Option<u64>,
    downloaded: u64,
    last_error: Option<CoreError>,
) -> ResumeState {
    ResumeState {
        // A chunked entity has no declared size: the bytes on disk are the best estimate.
        total_bytes: total.unwrap_or(downloaded),
        supports_ranges: !shared.env.sequential,
        etag: shared.env.etag.clone(),
        last_modified: shared.env.last_modified.clone(),
        segments: shared.records(),
        single_stream_written: if shared.env.sequential { downloaded } else { 0 },
        last_error,
    }
}

/// Flush, verify the size, verify the optional digest, then announce completion.
async fn finalize(
    shared: &Arc<Shared>,
    spec: &DownloadSpec,
    total: Option<u64>,
    downloaded: u64,
) -> Cycle {
    if let Err(error) = shared.env.file.sync_all() {
        return Cycle::Outcome(TaskOutcome::Failed { error });
    }
    let total = total.unwrap_or(downloaded);
    let length = match shared.env.file.len() {
        Ok(length) => length,
        Err(error) => return Cycle::Outcome(TaskOutcome::Failed { error }),
    };
    // The offsets must themselves tile the file: a resumed plan that never wrote a
    // byte would otherwise pass the length check thanks to preallocation.
    if downloaded != total || length != total {
        return Cycle::Outcome(TaskOutcome::Failed {
            error: CoreError::SizeMismatch,
        });
    }

    let checksum_ok = match &spec.expected_sha256 {
        None => None,
        Some(expected) => {
            shared.emit(EngineEvent::Verifying).await;
            match checksum::sha256_file(&spec.path).await {
                Ok(actual) if actual.eq_ignore_ascii_case(expected) => Some(true),
                Ok(_) => {
                    let renamed = rename_corrupt(&spec.path);
                    return Cycle::Outcome(match renamed {
                        Ok(()) => TaskOutcome::Failed {
                            error: CoreError::ChecksumMismatch,
                        },
                        Err(error) => TaskOutcome::Failed { error },
                    });
                }
                Err(error) => return Cycle::Outcome(TaskOutcome::Failed { error }),
            }
        }
    };

    let segments = shared.records();
    shared
        .emit(EngineEvent::Completed {
            path: spec.path.clone(),
            total_bytes: total,
            checksum_ok,
            segments: segments.clone(),
        })
        .await;
    Cycle::Outcome(TaskOutcome::Completed {
        total_bytes: total,
        checksum_ok,
        segments,
    })
}

/// Move a corrupt payload out of the way of the expected file name.
fn rename_corrupt(path: &Path) -> crate::error::Result<()> {
    let mut corrupt = path.as_os_str().to_os_string();
    corrupt.push(".corrupt");
    std::fs::rename(path, PathBuf::from(corrupt))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Progress reporting
// ---------------------------------------------------------------------------

/// Emits at most one `Progress` per [`PROGRESS_INTERVAL`] and the periodic checkpoint.
struct Reporter {
    shared: Arc<Shared>,
    total: Option<u64>,
    last_bytes: u64,
    last_tick: Instant,
    last_checkpoint: Instant,
}

impl Reporter {
    fn new(shared: Arc<Shared>, total: Option<u64>) -> Self {
        let now = Instant::now();
        let last_bytes = shared.downloaded();
        Reporter {
            shared,
            total,
            last_bytes,
            last_tick: now,
            last_checkpoint: now,
        }
    }

    async fn tick(&mut self) {
        let now = Instant::now();
        let downloaded = self.shared.downloaded();
        let elapsed = now.saturating_duration_since(self.last_tick).as_secs_f64();
        let speed_bps = if elapsed > 0.0 {
            (downloaded.saturating_sub(self.last_bytes) as f64 / elapsed) as u64
        } else {
            0
        };
        self.last_bytes = downloaded;
        self.last_tick = now;

        let due_time = now.saturating_duration_since(self.last_checkpoint) >= CHECKPOINT_INTERVAL;
        let due_bytes = self.shared.uncheckpointed.load(Ordering::SeqCst) >= CHECKPOINT_BYTES;
        if due_time || due_bytes {
            // Never announce offsets the filesystem has not confirmed.
            if self.shared.env.file.sync_data().is_ok() {
                self.shared.uncheckpointed.store(0, Ordering::SeqCst);
                self.last_checkpoint = now;
                self.shared
                    .emit(EngineEvent::Checkpoint {
                        segments: self.shared.records(),
                        downloaded_bytes: downloaded,
                    })
                    .await;
            }
        }

        let eta_seconds = match self.total {
            Some(total) if speed_bps > 0 && total > downloaded => {
                Some((total - downloaded).div_ceil(speed_bps))
            }
            _ => None,
        };
        self.shared
            .emit(EngineEvent::Progress {
                downloaded_bytes: downloaded,
                total_bytes: self.total,
                speed_bps,
                eta_seconds,
                segments: self.shared.records(),
            })
            .await;
    }
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

/// One segment's network loop.
///
/// The worker never blocks without listening: every `await` is a `select!` that also
/// watches the user canceller, the generation halt flag and the split control channel,
/// so a donor can always be reached between chunks.
struct SegmentWorker {
    shared: Arc<Shared>,
    seg: Arc<SegHandle>,
    msg_tx: mpsc::UnboundedSender<WorkerMsg>,
    cancel: Canceller,
    /// Next byte to write; published to `seg.next` after every write.
    next: u64,
    /// Consecutive failed attempts on the current range.
    failures: u32,
}

impl SegmentWorker {
    async fn run(mut self, mut control: mpsc::Receiver<SplitRequest>) {
        let env = Arc::clone(&self.shared.env);
        let kind = if env.sequential {
            WorkerKind::Sequential
        } else {
            WorkerKind::Range
        };
        let session = match Session::open(&env.http, &env.url).await {
            Ok(session) => session,
            Err(error) => {
                let resumable = error.pauses_task();
                return self.stop(error, resumable);
            }
        };

        loop {
            let end = self.seg.end();
            if self.next > end {
                return self.complete();
            }
            let range = match kind {
                WorkerKind::Range => Some((self.next, end)),
                WorkerKind::Sequential => None,
            };
            let validator = match kind {
                WorkerKind::Range => env.if_range.as_deref(),
                WorkerKind::Sequential => None,
            };

            let response = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return,
                _ = env.halt.cancelled() => return,
                request = control.recv() => {
                    if let Some(request) = request {
                        apply_split(&self.shared, &self.seg, self.next, request);
                    }
                    continue;
                }
                response = session.request(range, validator) => response,
            };

            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    if !error.is_retryable() {
                        return self.stop(error, false);
                    }
                    if !self.retry(&mut control, None).await {
                        if self.stopping() {
                            return;
                        }
                        return self.stop(error, true);
                    }
                    continue;
                }
            };

            let status = response.status().as_u16();
            match StatusDecision::from_status(status) {
                StatusDecision::Proceed => {
                    if kind == WorkerKind::Range
                        && status == 200
                        && !Self::full_range_response(&response, &env, self.next, end)
                    {
                        // A plain `200` to a range request means the remote object changed.
                        // iCloud answers a full-range request with `200` *and* a matching
                        // Content-Range; that is still the requested bytes, not a restart.
                        self.restart(format!(
                            "server answered 200 to the range request for bytes {}-{end}",
                            self.next
                        ));
                        return;
                    }
                }
                StatusDecision::Retry(hint) => {
                    let error = CoreError::Network(format!("HTTP {status}"));
                    if !self.retry(&mut control, hint).await {
                        if self.stopping() {
                            return;
                        }
                        return self.stop(error, true);
                    }
                    continue;
                }
                StatusDecision::Pause(error) => {
                    // 401/403/407: pause with the state so the user can fix credentials.
                    return self.stop(error, true);
                }
                StatusDecision::Fail(error) => {
                    if status == 416
                        && kind == WorkerKind::Range
                        && env.total.is_some_and(|total| self.next >= total)
                    {
                        // Nothing left in this range: `next == total` is a completion.
                        return self.complete();
                    }
                    return self.stop(error, false);
                }
            }

            if let Err(error) = self.validate_response(&env, &response, kind) {
                match error {
                    Report::Restart(reason) => {
                        self.restart(reason);
                        return;
                    }
                    Report::Stop(error) => return self.stop(error, false),
                }
            }

            // ---- read the body ------------------------------------------------
            let mut stream = response.bytes_stream();
            loop {
                let end = self.seg.end();
                if self.next > end {
                    return self.complete();
                }
                // Serve split requests that arrived between chunks before blocking.
                while let Ok(request) = control.try_recv() {
                    apply_split(&self.shared, &self.seg, self.next, request);
                }

                let item = tokio::select! {
                    biased;
                    _ = self.cancel.cancelled() => return,
                    _ = env.halt.cancelled() => return,
                    request = control.recv() => {
                        if let Some(request) = request {
                            apply_split(&self.shared, &self.seg, self.next, request);
                        }
                        continue;
                    }
                    item = tokio::time::timeout(env.http.read_timeout, stream.next()) => item,
                };

                match item {
                    Ok(Some(Ok(bytes))) => {
                        // Trim to the current end: bytes past it belong to a new worker.
                        // A sequential stream has no boundary to clamp at (its end is the
                        // end of the connection), so it takes the whole chunk.
                        let allowed = match kind {
                            WorkerKind::Range => planner::clamp_read(self.next, end, bytes.len()),
                            WorkerKind::Sequential => bytes.len(),
                        };
                        if allowed == 0 || !self.write(&bytes[..allowed]).await {
                            return;
                        }
                    }
                    Ok(Some(Err(error))) => {
                        let error = CoreError::Network(error.to_string());
                        if !self.retry(&mut control, None).await {
                            if self.stopping() {
                                return;
                            }
                            return self.stop(error, true);
                        }
                        if !self.rewind().await {
                            return;
                        }
                        break;
                    }
                    Ok(None) => {
                        if kind == WorkerKind::Sequential {
                            // `EOF` closes a sequential stream; a short one is truncated.
                            return match env.total {
                                Some(total) if self.next != total => {
                                    self.stop(CoreError::TruncatedResponse, false)
                                }
                                _ => self.complete(),
                            };
                        }
                        // A `206` body that ends before the segment end is truncated.
                        if !self.retry(&mut control, None).await {
                            if self.stopping() {
                                return;
                            }
                            return self.stop(CoreError::TruncatedResponse, true);
                        }
                        break;
                    }
                    Err(_elapsed) => {
                        let error =
                            CoreError::Network(format!("no data for {:?}", env.http.read_timeout));
                        if !self.retry(&mut control, None).await {
                            if self.stopping() {
                                return;
                            }
                            return self.stop(error, true);
                        }
                        if !self.rewind().await {
                            return;
                        }
                        break;
                    }
                }
            }
        }
    }

    /// Accept iCloud's nonstandard full-range `200` response.
    ///
    /// A range request normally gets `206`; iCloud answers a request that covers the whole
    /// entity with `200` plus a correct `Content-Range`. Treating that as a new generation
    /// made a small PDF restart twice and fail with `1028` even though the bytes were the
    /// requested ones. The response is accepted only when its Content-Range and length
    /// describe exactly the requested range and the planned total; anything else keeps the
    /// restart path.
    fn full_range_response(response: &reqwest::Response, env: &Env, next: u64, end: u64) -> bool {
        let (Some(value), Some(total)) = (
            response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok()),
            env.total,
        ) else {
            return false;
        };
        content_range_covers(value, next, end, Some(total), response.content_length())
    }

    /// Reject a response whose headers contradict the plan.
    fn validate_response(
        &self,
        env: &Env,
        response: &reqwest::Response,
        kind: WorkerKind,
    ) -> std::result::Result<(), Report> {
        if kind == WorkerKind::Sequential {
            if let (Some(total), Some(length)) = (env.total, response.content_length()) {
                if length != total.saturating_sub(self.next) {
                    return Err(Report::Stop(CoreError::TruncatedResponse));
                }
            }
            return Ok(());
        }
        let Some(value) = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
        else {
            return Err(Report::Stop(CoreError::MalformedHeader(
                "206 without Content-Range".to_string(),
            )));
        };
        match probe::parse_content_range(value) {
            Ok((_, _, Some(total))) if Some(total) != env.total => {
                // Another generation: restart rather than mix bytes (OpenSpec 06 §6).
                Err(Report::Restart(format!(
                    "the remote size changed to {total} bytes"
                )))
            }
            Ok((start, _, _)) if start != self.next => {
                Err(Report::Stop(CoreError::MalformedHeader(format!(
                    "content-range starts at {start}, expected {}",
                    self.next
                ))))
            }
            Ok(_) => Ok(()),
            Err(error) => Err(Report::Stop(error)),
        }
    }

    /// Write one clamped chunk, publishing `next` only after the bytes are on the file.
    async fn write(&mut self, bytes: &[u8]) -> bool {
        let env = Arc::clone(&self.shared.env);
        let mut done = 0usize;
        while done < bytes.len() {
            if self.stopping() {
                return false;
            }
            let take = env.chunk_size.min(bytes.len() - done);
            env.bucket.acquire(take as u64).await;
            if let Err(error) = env
                .file
                .pwrite_all(&bytes[done..done + take], self.next + done as u64)
            {
                let resumable = error.pauses_task();
                self.stop(error, resumable);
                return false;
            }
            done += take;
            self.seg
                .next
                .store(self.next + done as u64, Ordering::SeqCst);
            self.shared
                .uncheckpointed
                .fetch_add(take as u64, Ordering::SeqCst);
        }
        self.next += done as u64;
        self.failures = 0;
        true
    }

    /// A sequential stream cannot be resumed at an offset: the server ignores `Range`,
    /// so a retry re-requests the body from byte zero. The bytes already written are
    /// dropped and the offset restarts at zero rather than mixing two prefixes
    /// (OpenSpec 06 §5.1). Segmentation has no such problem: its offsets are stable, so
    /// this is a no-op there.
    async fn rewind(&mut self) -> bool {
        if !self.shared.env.sequential || self.next == 0 {
            return true;
        }
        if let Err(error) = self.shared.env.file.truncate() {
            self.stop(error, false);
            return false;
        }
        self.next = 0;
        self.seg.next.store(0, Ordering::SeqCst);
        self.shared
            .emit(EngineEvent::Restarted {
                reason: "the sequential stream restarted from byte zero".to_string(),
            })
            .await;
        true
    }

    /// Count one consecutive failure and wait before retrying the same range.
    ///
    /// `false` means "do not retry": either the budget is exhausted or the task is
    /// stopping (the caller distinguishes the two with [`Self::stopping`]).
    async fn retry(
        &mut self,
        control: &mut mpsc::Receiver<SplitRequest>,
        hint: Option<Duration>,
    ) -> bool {
        self.failures += 1;
        if self.failures > self.shared.env.max_retries {
            return false;
        }
        let delay = hint.unwrap_or_else(|| retry::backoff_delay(self.failures - 1));
        self.wait(control, delay).await
    }

    /// Sleep without going deaf: splits are answered and cancellation is honoured.
    async fn wait(&mut self, control: &mut mpsc::Receiver<SplitRequest>, delay: Duration) -> bool {
        let deadline = Instant::now() + delay;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return false,
                _ = self.shared.env.halt.cancelled() => return false,
                request = control.recv() => {
                    match request {
                        Some(request) => apply_split(&self.shared, &self.seg, self.next, request),
                        None => return true,
                    }
                }
                _ = tokio::time::sleep(left) => return true,
            }
        }
    }

    fn stopping(&self) -> bool {
        self.cancel.is_cancelled() || self.shared.env.halt.is_cancelled()
    }

    fn complete(&self) {
        let _ = self.msg_tx.send(WorkerMsg::Completed { id: self.seg.id });
    }

    fn stop(&self, error: CoreError, resumable: bool) {
        let _ = self.msg_tx.send(WorkerMsg::Stopped {
            id: self.seg.id,
            error,
            resumable,
        });
    }

    fn restart(&self, reason: String) {
        let _ = self.msg_tx.send(WorkerMsg::Restart { reason });
    }
}

/// Why a response was rejected.
enum Report {
    Restart(String),
    Stop(CoreError),
}

/// The donor side of the split handshake.
///
/// The donor checks that the requested boundary is still meaningful, clamps its own
/// `end`, registers the new range in the shared table and only then acknowledges. Both
/// changes happen under the table lock, so an observer of the table (a progress
/// snapshot, the donor selector) can never see a hole between the donor's new `end`
/// and the new segment's `start`.
fn apply_split(shared: &Shared, seg: &SegHandle, next: u64, request: SplitRequest) {
    let end = seg.end();
    let mut table = shared
        .segments
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if end != request.original_end {
        // A stale request: the donor changed since the coordinator looked at it.
        drop(table);
        let _ = request.reply.send(SplitAck {
            accepted: false,
            clamped_next: next,
        });
        return;
    }
    match planner::split_at(
        SegmentRange {
            start: seg.start,
            end,
        },
        next,
        request.midpoint,
    ) {
        SplitOutcome::Accepted {
            donor,
            clamped_next,
            ..
        } => {
            seg.end.store(donor.end, Ordering::SeqCst);
            table.push(SegHandle::new(&SegmentStateRecord {
                id: request.new_id,
                start: clamped_next,
                end: request.original_end,
                next: clamped_next,
            }));
            drop(table);
            let _ = request.reply.send(SplitAck {
                accepted: true,
                clamped_next,
            });
        }
        SplitOutcome::Rejected(_) => {
            drop(table);
            let _ = request.reply.send(SplitAck {
                accepted: false,
                clamped_next: next,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::TransferMode;

    #[test]
    fn canceller_is_observable_before_and_after_cancel() {
        let cancel = Canceller::new();
        assert!(!cancel.is_cancelled());
        cancel.cancel();
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn a_full_range_200_with_content_range_is_not_a_remote_change() {
        // The exact shape iCloud returns for a one-segment PDF.
        assert!(content_range_covers(
            "bytes 0-564063/564064",
            0,
            564063,
            Some(564064),
            Some(564064),
        ));
        // A partial response that starts at zero cannot satisfy a later worker.
        assert!(!content_range_covers(
            "bytes 0-564063/564064",
            4096,
            564063,
            Some(564064),
            Some(564064),
        ));
        // A different generation or a truncated body keeps the restart path.
        assert!(!content_range_covers(
            "bytes 0-564063/564065",
            0,
            564063,
            Some(564064),
            Some(564064),
        ));
        assert!(!content_range_covers(
            "bytes 0-564063/564064",
            0,
            564063,
            Some(564064),
            Some(2),
        ));
        assert!(!content_range_covers(
            "bytes 0-100/564064",
            0,
            564063,
            Some(564064),
            Some(564064),
        ));
    }

    #[test]
    fn a_resume_with_a_new_size_is_stale() {
        let state = ResumeState {
            total_bytes: 10,
            supports_ranges: true,
            etag: Some("\"v1\"".to_string()),
            last_modified: None,
            segments: Vec::new(),
            single_stream_written: 0,
            last_error: None,
        };
        let info = ProbeInfo {
            mode: TransferMode::Segmented { total: 11 },
            etag: Some("\"v1\"".to_string()),
            last_modified: None,
            content_type: None,
            content_disposition: None,
            final_url: Url::parse("http://127.0.0.1/file.bin").expect("url"),
        };
        assert!(stale_reason(&state, &info).is_some());
        let info = ProbeInfo {
            mode: TransferMode::Segmented { total: 10 },
            etag: Some("\"v2\"".to_string()),
            ..info
        };
        assert!(stale_reason(&state, &info).is_some());
        let info = ProbeInfo {
            mode: TransferMode::Segmented { total: 10 },
            etag: Some("\"v1\"".to_string()),
            ..info
        };
        assert!(stale_reason(&state, &info).is_none());
    }
}
