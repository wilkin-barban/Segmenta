//! Remote torrent jobs via Debrid providers.
//!
//! Contract: S3 of `odd/tasks/segmenta-hito3-debrid-archivos-torrents.md` (T5 b).
//!
//! A magnet is sent to a Debrid provider (Real-Debrid, TorBox, or Premiumize),
//! polled until completed remotely, enqueued into the download sink for local transfer,
//! and cleaned up remotely once local transfer completes.

use std::time::Duration;

use segmenta_core::debrid::{next_interval, DebridError, DebridHttp, Provider};
use segmenta_core::debrid_torrent::{
    add_request, delete_request, info_request, parse_add, parse_info, TorrentJob, TorrentState,
};
use segmenta_core::engine::Canceller;

/// Hard ceiling for remote job polling: 6 hours.
pub const HARD_DEADLINE: Duration = Duration::from_secs(6 * 3600);

/// Initial wait between polls before the provider backs off.
pub const DEFAULT_INITIAL_INTERVAL_SECS: u64 = 5;

/// Injectable sink for downloading files produced by a remote job.
#[async_trait::async_trait]
pub trait DownloadSink: Send + Sync {
    /// Enqueue a resolved direct URL for download.
    async fn enqueue(&self, url: &str) -> Result<(), String>;
}

/// Injectable clock and sleeper so tests never sleep real time.
#[async_trait::async_trait]
pub trait JobClock: Send + Sync {
    /// Current point in time.
    fn now(&self) -> std::time::Instant;

    /// Wait for the specified duration or until yield.
    async fn sleep(&self, duration: Duration);
}

/// Production wall-clock and tokio sleeper.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealClock;

#[async_trait::async_trait]
impl JobClock for RealClock {
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// Hand-advanced clock for deterministic tests.
pub struct ManualClock {
    base: std::time::Instant,
    offset: std::sync::Mutex<Duration>,
    auto_advance: std::sync::Mutex<Duration>,
}

impl ManualClock {
    /// New manual clock starting at current process instant.
    pub fn new() -> Self {
        Self {
            base: std::time::Instant::now(),
            offset: std::sync::Mutex::new(Duration::ZERO),
            auto_advance: std::sync::Mutex::new(Duration::ZERO),
        }
    }

    /// Automatically advance clock by `duration` on every sleep call.
    pub fn set_auto_advance(&self, duration: Duration) {
        *self.auto_advance.lock().unwrap() = duration;
    }

    /// Advance the clock by `duration`.
    pub fn advance(&self, duration: Duration) {
        *self.offset.lock().unwrap() += duration;
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl JobClock for ManualClock {
    fn now(&self) -> std::time::Instant {
        self.base + *self.offset.lock().unwrap()
    }

    async fn sleep(&self, _duration: Duration) {
        let step = *self.auto_advance.lock().unwrap();
        if !step.is_zero() {
            *self.offset.lock().unwrap() += step;
        }
        tokio::task::yield_now().await;
    }
}

/// Failures that can occur during a debrid torrent job.
#[derive(Debug)]
pub enum DebridTorrentError {
    /// Error originating from the core debrid provider or protocol layer.
    Core(DebridError),
    /// The download sink refused or failed to enqueue a link.
    Sink(String),
    /// Local files failed to complete.
    LocalCompletion(String),
    /// Polling exceeded the hard deadline.
    DeadlineExceeded,
    /// Cancellation requested by user or shutdown.
    Cancelled,
    /// The provider reported the job expired, deleted, or removed.
    Expired,
    /// The provider reported the remote download failed.
    Failed,
    /// No supported debrid account is enabled.
    NoAccount,
    /// The job reached ready state but provider listed no downloadable links.
    EmptyLinks,
}

impl std::fmt::Display for DebridTorrentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DebridTorrentError::Core(err) => write!(f, "{}", err.message()),
            DebridTorrentError::Sink(err) => write!(f, "download sink error: {err}"),
            DebridTorrentError::LocalCompletion(err) => write!(f, "local completion error: {err}"),
            DebridTorrentError::DeadlineExceeded => write!(f, "deadline exceeded (6 hours)"),
            DebridTorrentError::Cancelled => write!(f, "cancelled"),
            DebridTorrentError::Expired => write!(f, "remote torrent expired"),
            DebridTorrentError::Failed => write!(f, "remote torrent failed"),
            DebridTorrentError::NoAccount => write!(f, "no supported debrid account enabled"),
            DebridTorrentError::EmptyLinks => {
                write!(f, "remote torrent ready but reported no links")
            }
        }
    }
}

impl std::error::Error for DebridTorrentError {}

impl From<DebridError> for DebridTorrentError {
    fn from(err: DebridError) -> Self {
        DebridTorrentError::Core(err)
    }
}

/// Select the highest priority account whose provider supports remote torrents.
pub fn pick_best_account(candidates: &[(Provider, String)]) -> Option<(Provider, String)> {
    candidates
        .iter()
        .find(|(provider, _)| {
            matches!(
                provider,
                Provider::RealDebrid | Provider::TorBox | Provider::Premiumize
            )
        })
        .cloned()
}

/// Run a remote debrid torrent job end to end.
///
/// 1. Adds the magnet to the provider.
/// 2. Bounded polling until completion, error, expiration, or deadline.
/// 3. Hands ready links to `sink.enqueue`.
/// 4. When all links are accepted AND `local_completion` reports success, deletes the remote job.
///    If anything fails, leaves the remote job alone and logs why.
#[allow(clippy::too_many_arguments)]
pub async fn run_debrid_torrent_job<F, P>(
    http: &dyn DebridHttp,
    provider: Provider,
    token: &str,
    magnet: &str,
    sink: &dyn DownloadSink,
    clock: &dyn JobClock,
    cancel: &Canceller,
    local_completion: F,
    mut on_progress: P,
) -> Result<(), DebridTorrentError>
where
    F: FnOnce() -> Result<(), String> + Send,
    P: FnMut(&TorrentJob) + Send,
{
    if cancel.is_cancelled() {
        return Err(DebridTorrentError::Cancelled);
    }

    let add_req = add_request(provider, token, magnet)?;
    let (add_status, add_body) = http.send(&add_req).await?;
    let job_id = parse_add(provider, add_status, &add_body)?;
    tracing::info!(provider = provider.as_str(), id = %job_id, "remote torrent job added");

    let start = clock.now();
    let deadline = start + HARD_DEADLINE;
    let mut interval_secs = DEFAULT_INITIAL_INTERVAL_SECS;

    // Strict iteration bound to ensure no loop can run unbounded.
    const MAX_POLLS: usize = 100_000;
    let mut polls = 0;

    let ready_job: TorrentJob = loop {
        polls += 1;
        if polls > MAX_POLLS {
            tracing::warn!(id = %job_id, "max poll iterations exceeded");
            return Err(DebridTorrentError::DeadlineExceeded);
        }

        if cancel.is_cancelled() {
            return Err(DebridTorrentError::Cancelled);
        }

        let now = clock.now();
        if now >= deadline {
            tracing::warn!(id = %job_id, "poll loop reached 6-hour deadline");
            return Err(DebridTorrentError::DeadlineExceeded);
        }

        let info_req = info_request(provider, token, &job_id)?;
        let (info_status, info_body) = match http.send(&info_req).await {
            Ok(res) => res,
            Err(DebridError::QuotaExceeded) => {
                interval_secs = next_interval(interval_secs, true);
                let remaining = deadline.saturating_duration_since(clock.now());
                if remaining.is_zero() {
                    return Err(DebridTorrentError::DeadlineExceeded);
                }
                let wait = Duration::from_secs(interval_secs).min(remaining);
                tokio::select! {
                    _ = cancel.cancelled() => return Err(DebridTorrentError::Cancelled),
                    _ = clock.sleep(wait) => continue,
                }
            }
            Err(err) => return Err(DebridTorrentError::Core(err)),
        };

        let job = match parse_info(provider, info_status, &info_body) {
            Ok(job) => job,
            Err(DebridError::QuotaExceeded) => {
                interval_secs = next_interval(interval_secs, true);
                let remaining = deadline.saturating_duration_since(clock.now());
                if remaining.is_zero() {
                    return Err(DebridTorrentError::DeadlineExceeded);
                }
                let wait = Duration::from_secs(interval_secs).min(remaining);
                tokio::select! {
                    _ = cancel.cancelled() => return Err(DebridTorrentError::Cancelled),
                    _ = clock.sleep(wait) => continue,
                }
            }
            // UnknownState, Malformed, Unauthorized, etc. surface immediately.
            Err(err) => return Err(DebridTorrentError::Core(err)),
        };

        on_progress(&job);

        match job.state {
            TorrentState::Queued | TorrentState::Downloading => {
                interval_secs = next_interval(interval_secs, false);
                let remaining = deadline.saturating_duration_since(clock.now());
                if remaining.is_zero() {
                    return Err(DebridTorrentError::DeadlineExceeded);
                }
                let wait = Duration::from_secs(interval_secs).min(remaining);
                tokio::select! {
                    _ = cancel.cancelled() => return Err(DebridTorrentError::Cancelled),
                    _ = clock.sleep(wait) => continue,
                }
            }
            TorrentState::Ready => {
                break job;
            }
            TorrentState::Expired => {
                tracing::warn!(id = %job_id, "remote torrent expired or removed by provider");
                return Err(DebridTorrentError::Expired);
            }
            TorrentState::Failed => {
                tracing::warn!(id = %job_id, "remote torrent failed on provider");
                return Err(DebridTorrentError::Failed);
            }
        }
    };

    if ready_job.links.is_empty() {
        tracing::warn!(
            id = %job_id,
            "remote torrent ready but reported no links; leaving remote job untouched"
        );
        return Err(DebridTorrentError::EmptyLinks);
    }

    // Enqueue every link into the download sink.
    for link in &ready_job.links {
        if cancel.is_cancelled() {
            return Err(DebridTorrentError::Cancelled);
        }
        if let Err(err) = sink.enqueue(link).await {
            tracing::warn!(
                id = %job_id,
                link = %link,
                error = %err,
                "download sink refused link; leaving remote job untouched"
            );
            return Err(DebridTorrentError::Sink(err));
        }
    }

    // All links accepted by sink; check local completion.
    match local_completion() {
        Ok(()) => {
            tracing::info!(
                id = %job_id,
                "all links accepted and local completion successful; deleting remote job"
            );
            let del_req = delete_request(provider, token, &job_id)?;
            if let Err(err) = http.send(&del_req).await {
                tracing::warn!(
                    id = %job_id,
                    error = ?err,
                    "delete request failed on provider; leaving local files intact"
                );
            }
            Ok(())
        }
        Err(err) => {
            // The links are queued, so the task succeeded from the user's point of view.
            // What is not confirmed is the local download, and that is the only thing that
            // authorizes deleting the remote copy: keep it and say so.
            tracing::warn!(
                id = %job_id,
                reason = %err,
                "local completion not confirmed; the remote job is kept"
            );
            Ok(())
        }
    }
}
