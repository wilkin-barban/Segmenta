//! Pure model layer: wire types, row updates, filters and formatting.
//!
//! Everything here is toolkit free so it can be unit tested without a display
//! and so the daemon payloads have a single parsing point.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Lifecycle state of one download as reported by the daemon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Pending,
    Probing,
    Downloading,
    Paused,
    /// Waiting for the next retry (core `retrying`).
    Retrying,
    Verifying,
    /// Flushing and publishing the file (core `finalizing`).
    Finalizing,
    Completed,
    Failed,
    Cancelled,
    #[default]
    #[serde(other)]
    Unknown,
}

impl DownloadStatus {
    /// Spanish label shown in the list and the detail page.
    /// Localized status label (S21).
    pub fn label(&self, locale: segmenta_i18n::Locale) -> &'static str {
        use segmenta_i18n::{tr, Msg};
        tr(
            locale,
            match self {
                DownloadStatus::Pending => Msg::StatusPending,
                DownloadStatus::Probing => Msg::StatusProbing,
                DownloadStatus::Downloading => Msg::StatusDownloading,
                DownloadStatus::Paused => Msg::StatusPaused,
                DownloadStatus::Retrying => Msg::StatusRetrying,
                DownloadStatus::Verifying => Msg::StatusVerifying,
                DownloadStatus::Finalizing => Msg::StatusFinalizing,
                DownloadStatus::Completed => Msg::StatusCompleted,
                DownloadStatus::Failed => Msg::StatusFailed,
                DownloadStatus::Cancelled => Msg::StatusCancelled,
                DownloadStatus::Unknown => Msg::StatusUnknown,
            },
        )
    }

    /// Icon name for the sidebar and the row.
    pub fn icon_name(&self) -> &'static str {
        match self {
            DownloadStatus::Pending => "content-loading-symbolic",
            DownloadStatus::Probing => "system-search-symbolic",
            DownloadStatus::Downloading => "go-down-symbolic",
            DownloadStatus::Paused => "media-playback-pause-symbolic",
            DownloadStatus::Retrying => "view-refresh-symbolic",
            DownloadStatus::Verifying => "emblem-ok-symbolic",
            DownloadStatus::Finalizing => "document-save-symbolic",
            DownloadStatus::Completed => "emblem-ok-symbolic",
            DownloadStatus::Failed => "dialog-warning-symbolic",
            DownloadStatus::Cancelled => "process-stop-symbolic",
            DownloadStatus::Unknown => "dialog-question-symbolic",
        }
    }

    /// Work is in flight: bytes are moving, or the task is about to move them
    /// (`retrying`, `finalizing`).
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            DownloadStatus::Downloading
                | DownloadStatus::Probing
                | DownloadStatus::Retrying
                | DownloadStatus::Finalizing
        )
    }

    /// A task that is running and can be paused.
    ///
    /// Mirrors `manager.rs::pause_task`, which accepts `pending`, `probing`,
    /// `retrying`, `verifying` and `finalizing` as well as `downloading`.
    pub fn can_pause(&self) -> bool {
        self.is_active()
    }

    /// A task that stopped and can be started again without the file being lost.
    ///
    /// `retrying` is accepted too: `manager.rs::resume_task` takes `paused` and
    /// `retrying` and re-queues the task, which is how a retry is forced now.
    pub fn can_resume(&self) -> bool {
        matches!(
            self,
            DownloadStatus::Paused | DownloadStatus::Failed | DownloadStatus::Retrying
        )
    }

    /// A task with work in flight that can be cancelled.
    pub fn can_cancel(&self) -> bool {
        matches!(
            self,
            DownloadStatus::Pending
                | DownloadStatus::Probing
                | DownloadStatus::Downloading
                | DownloadStatus::Paused
                | DownloadStatus::Retrying
                | DownloadStatus::Finalizing
        )
    }

    /// A finished task whose database row can be deleted.
    pub fn can_delete(&self) -> bool {
        matches!(
            self,
            DownloadStatus::Completed | DownloadStatus::Cancelled | DownloadStatus::Failed
        )
    }
}

/// One segment as reported by `download.status` and `task_progress`.
///
/// `download.status` uses `id` and the `task_progress` payload uses
/// `segment_id`; both shapes parse into this type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SegmentItem {
    #[serde(alias = "segment_id")]
    pub id: u32,
    pub start_offset: u64,
    pub end_offset: u64,
    pub next_offset: u64,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub speed_bps: u64,
}

impl SegmentItem {
    /// Number of bytes this segment owns (`end_offset` is inclusive).
    pub fn span(&self) -> u64 {
        self.end_offset
            .saturating_sub(self.start_offset)
            .saturating_add(1)
    }

    /// Bytes of this segment already written to the file.
    pub fn downloaded(&self) -> u64 {
        self.next_offset
            .min(self.end_offset.saturating_add(1))
            .saturating_sub(self.start_offset)
    }

    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    pub fn is_complete(&self) -> bool {
        self.next_offset > self.end_offset
    }
}

/// One download row of `download.list` / `download.status`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct DownloadItem {
    pub task_id: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub final_path: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub status: DownloadStatus,
    #[serde(default)]
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub downloaded_bytes: u64,
    #[serde(default)]
    pub speed_bps: u64,
    #[serde(default)]
    pub eta_seconds: Option<u64>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub supports_ranges: bool,
    #[serde(default)]
    pub queue_name: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub error_code: Option<i64>,
    #[serde(default)]
    pub error_message: Option<String>,
    /// Where the daemon moved an infected file (`ERR_ANTIVIRUS_FOUND`).
    #[serde(default)]
    pub quarantine_path: Option<String>,
    #[serde(default)]
    pub segments: Vec<SegmentItem>,
    /// Media kind for 0.1.0 tasks (`video`, `audio`, `playlist`).
    #[serde(default)]
    pub media_kind: Option<String>,
    /// Playlist title when the task is a playlist.
    #[serde(default)]
    pub playlist_title: Option<String>,
}

/// The daemon quarantined the file: it is no longer in the downloads folder.
pub const ERR_ANTIVIRUS_FOUND: i64 = 1046;
/// The scan was requested but neither `clamdscan` nor `clamscan` is usable.
pub const ERR_ANTIVIRUS_UNAVAILABLE: i64 = 1047;

impl DownloadItem {
    /// Whether the daemon moved this file to quarantine.
    ///
    /// The quarantine path is the evidence: `1046` alone would still be a code, and the
    /// UI must never claim a quarantined file is in the downloads folder.
    pub fn is_quarantined(&self) -> bool {
        self.error_code == Some(ERR_ANTIVIRUS_FOUND)
            && self
                .quarantine_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
    }

    /// Path to show for this task: the quarantine copy when there is one.
    pub fn display_path(&self) -> &str {
        match self.quarantine_path.as_deref() {
            Some(path) if self.is_quarantined() => path,
            _ => &self.final_path,
        }
    }

    /// `0.0..=1.0`, or `0.0` while the total size is unknown.
    pub fn progress(&self) -> f64 {
        match self.total_bytes {
            Some(total) if total > 0 => {
                (self.downloaded_bytes as f64 / total as f64).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    pub fn active_segments(&self) -> usize {
        self.segments
            .iter()
            .filter(|segment| segment.is_active())
            .count()
    }

    /// `1.2 GB de 4.0 GB`, or just the downloaded amount when the size is unknown.
    pub fn size_line(&self) -> String {
        match self.total_bytes {
            Some(total) => format!(
                "{} {} {}",
                format_bytes(self.downloaded_bytes),
                segmenta_i18n::tr(segmenta_i18n::current(), segmenta_i18n::Msg::Of),
                format_bytes(total)
            ),
            None => format_bytes(self.downloaded_bytes),
        }
    }

    /// Localized media badge (`Video`, `Audio`, `Playlist`), empty for direct downloads.
    pub fn media_badge(&self) -> String {
        let Some(kind) = self.media_kind.as_deref() else {
            return String::new();
        };
        let message = match kind {
            "audio" => segmenta_i18n::Msg::MediaAudioBadge,
            "playlist" => segmenta_i18n::Msg::MediaPlaylistBadge,
            "torrent" | "magnet" => segmenta_i18n::Msg::MediaTorrentBadge,
            _ => segmenta_i18n::Msg::MediaVideoBadge,
        };
        segmenta_i18n::tr(segmenta_i18n::current(), message).to_string()
    }

    /// `80.0 MB/s · 25 s left · 2.0 GB of 4.0 GB` (labels localized).
    pub fn progress_line(&self) -> String {
        let locale = segmenta_i18n::current();
        let eta = match self.eta_seconds {
            Some(seconds) if seconds > 0 => format!(
                "{} {}",
                format_eta(Some(seconds)),
                segmenta_i18n::tr(locale, segmenta_i18n::Msg::Eta).to_lowercase()
            ),
            _ => format_eta(self.eta_seconds),
        };
        let mut parts = vec![format_speed(self.speed_bps), eta];
        let size = self.size_line();
        if !size.is_empty() {
            parts.push(size);
        }
        parts.join(" · ")
    }

    /// Matches `query` against the filename and the URL, case insensitively.
    pub fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let needle = query.to_lowercase();
        self.filename.to_lowercase().contains(&needle) || self.url.to_lowercase().contains(&needle)
    }
}

/// Payload of a `task_progress` notification.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ProgressUpdate {
    #[serde(default)]
    pub downloaded_bytes: u64,
    #[serde(default)]
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub speed_bps: u64,
    #[serde(default)]
    pub eta_seconds: Option<u64>,
    #[serde(default)]
    pub segments: Vec<SegmentItem>,
    /// Media kind for 0.1.0 tasks (`video`, `audio`, `playlist`).
    #[serde(default)]
    pub media_kind: Option<String>,
    /// Playlist title when the task is a playlist.
    #[serde(default)]
    pub playlist_title: Option<String>,
}

/// Sidebar filter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Active,
    Paused,
    Completed,
    Failed,
}

impl Filter {
    pub const ALL: [Filter; 5] = [
        Filter::All,
        Filter::Active,
        Filter::Paused,
        Filter::Completed,
        Filter::Failed,
    ];

    /// Localized sidebar label (S21).
    pub fn label(&self, locale: segmenta_i18n::Locale) -> &'static str {
        use segmenta_i18n::{tr, Msg};
        tr(
            locale,
            match self {
                Filter::All => Msg::FilterAll,
                Filter::Active => Msg::FilterActive,
                Filter::Paused => Msg::FilterPaused,
                Filter::Completed => Msg::FilterCompleted,
                Filter::Failed => Msg::FilterFailed,
            },
        )
    }

    pub fn icon_name(&self) -> &'static str {
        match self {
            Filter::All => "view-list-symbolic",
            Filter::Active => "go-down-symbolic",
            Filter::Paused => "media-playback-pause-symbolic",
            Filter::Completed => "emblem-ok-symbolic",
            Filter::Failed => "dialog-warning-symbolic",
        }
    }

    /// Human description of the empty state for this filter.
    pub fn empty_title(&self) -> &'static str {
        match self {
            Filter::All => "Todavía no hay descargas",
            Filter::Active => "No hay descargas activas",
            Filter::Paused => "No hay descargas pausadas",
            Filter::Completed => "No hay descargas completadas",
            Filter::Failed => "No hay descargas fallidas",
        }
    }

    pub fn matches(&self, status: DownloadStatus) -> bool {
        match self {
            Filter::All => true,
            Filter::Active => status.is_active(),
            Filter::Paused => status == DownloadStatus::Paused,
            Filter::Completed => status == DownloadStatus::Completed,
            Filter::Failed => status == DownloadStatus::Failed,
        }
    }
}

/// Sidebar counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub all: usize,
    pub active: usize,
    pub paused: usize,
    pub completed: usize,
    pub failed: usize,
}

impl Counts {
    pub fn of(&self, filter: Filter) -> usize {
        match filter {
            Filter::All => self.all,
            Filter::Active => self.active,
            Filter::Paused => self.paused,
            Filter::Completed => self.completed,
            Filter::Failed => self.failed,
        }
    }
}

/// Result of sequencing a push notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqStatus {
    /// Next `seq_id` in order.
    InOrder,
    /// Something was lost: the caller must reload with `download.list`.
    Gap,
    /// Already seen (or older): ignore it.
    Duplicate,
}

/// Pure in-memory view of the daemon's download table.
#[derive(Debug, Default)]
pub struct DownloadStore {
    items: Vec<DownloadItem>,
    last_seq: Option<u64>,
}

impl DownloadStore {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> &[DownloadItem] {
        &self.items
    }

    pub fn get(&self, task_id: &str) -> Option<&DownloadItem> {
        self.items.iter().find(|item| item.task_id == task_id)
    }

    fn get_mut(&mut self, task_id: &str) -> Option<&mut DownloadItem> {
        self.items.iter_mut().find(|item| item.task_id == task_id)
    }

    /// Replaces the whole table: a reload must never duplicate rows.
    pub fn reload(&mut self, items: Vec<DownloadItem>) {
        self.items.clear();
        for item in items {
            self.upsert(item);
        }
    }

    /// Inserts or replaces one row, keeping the current order.
    pub fn upsert(&mut self, item: DownloadItem) {
        match self.get_mut(&item.task_id) {
            Some(existing) => *existing = item,
            // Newest first: a task added from the extension, the CLI or the GUI must
            // appear where the daemon lists it (created_at DESC), not at the tail.
            None => self.items.insert(0, item),
        }
    }

    pub fn remove(&mut self, task_id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.task_id != task_id);
        before != self.items.len()
    }

    /// Applies a `task_progress` payload. Unknown tasks are ignored so a stale
    /// notification can never resurrect a deleted row.
    pub fn apply_progress(&mut self, task_id: &str, progress: &ProgressUpdate) -> bool {
        let Some(item) = self.get_mut(task_id) else {
            return false;
        };
        item.downloaded_bytes = progress.downloaded_bytes;
        item.speed_bps = progress.speed_bps;
        item.eta_seconds = progress.eta_seconds;
        if progress.total_bytes.is_some() {
            item.total_bytes = progress.total_bytes;
        }
        if !progress.segments.is_empty() {
            item.segments = progress.segments.clone();
        }
        true
    }

    /// Applies a `state_changed`/`task_completed` transition.
    pub fn apply_state(&mut self, task_id: &str, status: DownloadStatus) -> bool {
        let Some(item) = self.get_mut(task_id) else {
            return false;
        };
        item.status = status;
        // `retrying` and `finalizing` are in flight but no bytes move, so the last
        // speed and ETA belong to the previous attempt and must not be painted as live.
        if !matches!(
            status,
            DownloadStatus::Downloading | DownloadStatus::Probing
        ) {
            item.speed_bps = 0;
            if status != DownloadStatus::Failed {
                item.eta_seconds = None;
            }
        }
        if status.is_active() {
            item.error_code = None;
            item.error_message = None;
        }
        true
    }

    /// Applies a `task_failed` transition.
    pub fn apply_error(&mut self, task_id: &str, code: i64, message: &str) -> bool {
        let Some(item) = self.get_mut(task_id) else {
            return false;
        };
        item.status = DownloadStatus::Failed;
        item.error_code = Some(code);
        item.error_message = Some(message.to_string());
        item.speed_bps = 0;
        item.eta_seconds = None;
        true
    }

    pub fn counts(&self) -> Counts {
        let mut counts = Counts {
            all: self.items.len(),
            ..Counts::default()
        };
        for item in &self.items {
            match item.status {
                status if status.is_active() => counts.active += 1,
                DownloadStatus::Paused => counts.paused += 1,
                DownloadStatus::Completed => counts.completed += 1,
                DownloadStatus::Failed => counts.failed += 1,
                _ => {}
            }
        }
        counts
    }

    pub fn filtered(&self, filter: Filter, query: &str) -> Vec<DownloadItem> {
        let query = query.trim();
        self.items
            .iter()
            .filter(|item| filter.matches(item.status) && item.matches(query))
            .cloned()
            .collect()
    }

    /// Tracks the `seq_id` of `download.event` notifications.
    pub fn observe_seq(&mut self, seq: u64) -> SeqStatus {
        match self.last_seq {
            None => {
                self.last_seq = Some(seq);
                SeqStatus::InOrder
            }
            Some(last) if seq == last + 1 => {
                self.last_seq = Some(seq);
                SeqStatus::InOrder
            }
            Some(last) if seq > last => {
                self.last_seq = Some(seq);
                SeqStatus::Gap
            }
            Some(_) => SeqStatus::Duplicate,
        }
    }

    pub fn last_seq(&self) -> Option<u64> {
        self.last_seq
    }
}

/// `settings.get` / `settings.update` payload.
fn default_antivirus_timeout() -> u64 {
    120
}

fn default_true() -> bool {
    true
}

/// Settings payload as reported by the daemon.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Settings {
    #[serde(default)]
    pub max_connections: u32,
    #[serde(default)]
    pub global_speed_limit_bps: Option<u64>,
    #[serde(default)]
    pub clip_monitor_enabled: bool,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub download_dir: String,
    #[serde(default)]
    pub category_dirs: BTreeMap<String, String>,
    #[serde(default = "default_true")]
    pub capture_enabled: bool,
    #[serde(default = "default_true")]
    pub capture_torrents: bool,
    #[serde(default = "default_true")]
    pub capture_magnet: bool,
    #[serde(default)]
    pub capture_extensions: Vec<String>,
    #[serde(default)]
    pub domain_denylist: Vec<String>,
    /// Run ClamAV before a download is marked completed (A2).
    #[serde(default)]
    pub antivirus_enabled: bool,
    /// Scan timeout in seconds, 1..=3600.
    #[serde(default = "default_antivirus_timeout")]
    pub antivirus_timeout_secs: u64,
    /// Where an infected file is moved.
    #[serde(default)]
    pub antivirus_quarantine_dir: String,
    #[serde(default)]
    pub schedule_start: Option<String>,
    #[serde(default)]
    pub schedule_stop: Option<String>,
    #[serde(default)]
    pub action_on_complete: String,
    #[serde(default)]
    pub allow_local_downloads: bool,
    #[serde(default)]
    pub max_retries: u32,
    #[serde(default)]
    pub proxy_mode: String,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub postprocess_command: Vec<String>,
}

impl Settings {
    /// Speed limit in KiB/s; `0` means unlimited (the UI convention).
    pub fn speed_limit_kib(&self) -> u64 {
        self.global_speed_limit_bps.unwrap_or(0) / 1024
    }

    /// Bytes per second for a KiB/s value coming from the UI.
    pub fn speed_limit_bps(kib: u64) -> Option<u64> {
        if kib == 0 {
            None
        } else {
            Some(kib * 1024)
        }
    }
}

const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];

/// `45.2 MB`, `512 B`.
pub fn format_bytes(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// `45.2 MB/s`, or an em dash when nothing is moving.
pub fn format_speed(bytes_per_second: u64) -> String {
    if bytes_per_second == 0 {
        "—".to_string()
    } else {
        format!("{}/s", format_bytes(bytes_per_second))
    }
}

/// `Quedan 45 s`, `Quedan 1 min`, `Quedan 1 h 1 min`, or an em dash.
pub fn format_eta(seconds: Option<u64>) -> String {
    match seconds {
        None | Some(0) => "—".to_string(),
        Some(seconds) if seconds < 60 => format!("{seconds} s"),
        Some(seconds) if seconds < 3600 => format!("{} min", seconds / 60),
        Some(seconds) => format!("{} h {} min", seconds / 3600, (seconds % 3600) / 60),
    }
}

/// Extracts a plausible filename from a URL, used to prefill the add dialog.
pub fn filename_from_url(url: &str) -> Option<String> {
    let without_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let path = without_scheme
        .split(['#', '?'])
        .next()
        .unwrap_or(without_scheme);
    let last = path.rsplit('/').next().unwrap_or_default();
    let without_query = last.split(['#', '?']).next().unwrap_or(last);
    if without_query.is_empty() || !without_query.contains('.') {
        return None;
    }
    Some(without_query.to_string())
}

/// `true` for `http://`/`https://` URLs, the only schemes this release handles.
pub fn looks_like_http_url(text: &str) -> bool {
    let trimmed = text.trim();
    (trimmed.starts_with("http://") || trimmed.starts_with("https://")) && trimmed.len() > 8
}
