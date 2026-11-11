//! SQLite persistence: the normative DDL of OpenSpec 08, idempotent migrations,
//! checkpoint updates and settings.
//!
//! The database never stores a secret: `download_secrets` and `site_logins` only keep
//! an alias into the secret store (keyring or the 0600 fallback file).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current schema version, stored in `PRAGMA user_version`.
pub const SCHEMA_VERSION: i64 = 1;

const SCHEMA_SQL: &str = include_str!("schema.sql");

/// Errors raised by the persistence layer.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite failure.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Filesystem failure while preparing the data directory.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// A settings patch was rejected; the string names the offending key.
    #[error("{0}")]
    Invalid(String),
    /// A queue operation the contract forbids.
    #[error("{0}")]
    Queue(String),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Download lifecycle, exactly the CHECK list of the DDL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    /// Queued, not started.
    Pending,
    /// Probing the remote resource.
    Probing,
    /// Workers are writing bytes.
    Downloading,
    /// Stopped by the user or after a recoverable error.
    Paused,
    /// Waiting for the next retry.
    Retrying,
    /// Computing the final checksum.
    Verifying,
    /// Flushing and publishing the file.
    Finalizing,
    /// Finished and validated.
    Completed,
    /// Terminal error.
    Failed,
    /// Cancelled by the user.
    Cancelled,
}

impl DownloadStatus {
    /// The literal stored in the `status` column.
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadStatus::Pending => "pending",
            DownloadStatus::Probing => "probing",
            DownloadStatus::Downloading => "downloading",
            DownloadStatus::Paused => "paused",
            DownloadStatus::Retrying => "retrying",
            DownloadStatus::Verifying => "verifying",
            DownloadStatus::Finalizing => "finalizing",
            DownloadStatus::Completed => "completed",
            DownloadStatus::Failed => "failed",
            DownloadStatus::Cancelled => "cancelled",
        }
    }

    /// Parse the stored literal.
    pub fn parse(value: &str) -> Option<DownloadStatus> {
        Some(match value {
            "pending" => DownloadStatus::Pending,
            "probing" => DownloadStatus::Probing,
            "downloading" => DownloadStatus::Downloading,
            "paused" => DownloadStatus::Paused,
            "retrying" => DownloadStatus::Retrying,
            "verifying" => DownloadStatus::Verifying,
            "finalizing" => DownloadStatus::Finalizing,
            "completed" => DownloadStatus::Completed,
            "failed" => DownloadStatus::Failed,
            "cancelled" => DownloadStatus::Cancelled,
            _ => return None,
        })
    }

    /// Whether the daemon has to reconcile this state on startup.
    pub fn is_in_flight(self) -> bool {
        matches!(
            self,
            DownloadStatus::Downloading
                | DownloadStatus::Probing
                | DownloadStatus::Verifying
                | DownloadStatus::Finalizing
                | DownloadStatus::Retrying
        )
    }
}

/// One row of `downloads`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRecord {
    /// UUIDv4 task identifier.
    pub id: String,
    /// Original URL as given by the client.
    pub url: String,
    /// URL after redirects, once known.
    pub final_url: Option<String>,
    /// Destination path on disk.
    pub final_path: String,
    /// Temporary path when the writer needs a rename.
    pub temp_path: Option<String>,
    /// Total size, `None` while unknown.
    pub total_bytes: Option<u64>,
    /// Bytes confirmed by checkpoints.
    pub downloaded_bytes: u64,
    /// Lifecycle state.
    pub status: DownloadStatus,
    /// Whether the server honoured ranges.
    pub supports_ranges: bool,
    /// Strong validator.
    pub etag: Option<String>,
    /// Weak validator.
    pub last_modified: Option<String>,
    /// `Content-Type` as reported.
    pub content_type: Option<String>,
    /// Optional expected SHA-256.
    pub expected_checksum: Option<String>,
    /// Numeric error code of the last failure.
    pub error_code: Option<i64>,
    /// Human message of the last failure.
    pub error_message: Option<String>,
    /// Consecutive retry counter.
    pub retry_count: u32,
    /// Scheduling priority, higher first.
    pub priority: i64,
    /// Queue name.
    pub queue_name: String,
    /// `direct` for 0.0.1; `hls`/`dash` are reserved.
    pub download_type: String,
    /// Creation timestamp (SQLite `CURRENT_TIMESTAMP`).
    pub created_at: Option<String>,
    /// Completion timestamp.
    pub completed_at: Option<String>,
    /// Suggested name captured at add time.
    pub filename: String,
}

/// One row of `segments`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentRow {
    /// Segment identifier.
    pub id: u32,
    /// First byte, inclusive.
    pub start: u64,
    /// Last byte, inclusive.
    pub end: u64,
    /// Next byte to write.
    pub next: u64,
}

/// One row of `queues`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueRow {
    /// Queue name.
    pub name: String,
    /// Concurrent tasks allowed.
    pub max_active: u32,
}

/// Settings catalog of OpenSpec 07 §3.9.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Maximum connections per download (1-32).
    pub max_connections: u32,
    /// Global speed limit in bytes per second; `None` is unlimited.
    pub global_speed_limit_bps: Option<u64>,
    /// Clipboard monitoring (extension feature, stored for 0.1.0).
    pub clip_monitor_enabled: bool,
    /// Start the daemon with the session.
    pub autostart: bool,
    /// Default download directory.
    pub download_dir: String,
    /// Category to directory overrides.
    pub category_dirs: BTreeMap<String, String>,
    /// Extensions captured by the browser extension.
    pub capture_extensions: Vec<String>,
    /// Domains that must never be captured.
    pub domain_denylist: Vec<String>,
    /// Scheduler start time `HH:MM`.
    pub schedule_start: Option<String>,
    /// Scheduler stop time `HH:MM`.
    pub schedule_stop: Option<String>,
    /// Action when the queue empties.
    pub action_on_complete: String,
    /// Allow loopback/private destinations.
    pub allow_local_downloads: bool,
    /// Retries before a task pauses.
    pub max_retries: u32,
    /// `system`, `none` or `manual`.
    pub proxy_mode: String,
    /// Proxy URL for manual mode.
    pub proxy_url: Option<String>,
    /// Post-processing command without a shell.
    pub postprocess_command: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            max_connections: 16,
            global_speed_limit_bps: None,
            clip_monitor_enabled: true,
            autostart: false,
            download_dir: default_download_dir(),
            category_dirs: BTreeMap::new(),
            capture_extensions: vec![
                "7z", "avi", "bin", "bz2", "deb", "dmg", "flac", "gz", "iso", "m4a", "mkv", "mp3",
                "mp4", "mpeg", "msi", "ogg", "pdf", "rar", "tar", "tgz", "webm", "wav", "xz",
                "zip",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            domain_denylist: Vec::new(),
            schedule_start: None,
            schedule_stop: None,
            action_on_complete: "none".to_string(),
            allow_local_downloads: false,
            max_retries: 5,
            proxy_mode: "system".to_string(),
            proxy_url: None,
            postprocess_command: Vec::new(),
        }
    }
}

fn default_download_dir() -> String {
    if let Ok(dir) = std::env::var("XDG_DOWNLOAD_DIR") {
        if !dir.is_empty() {
            return dir;
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    Path::new(&home)
        .join("Downloads")
        .to_string_lossy()
        .into_owned()
}

/// Keys accepted by `settings.update`, in catalog order.
pub const SETTING_KEYS: &[&str] = &[
    "max_connections",
    "global_speed_limit_bps",
    "clip_monitor_enabled",
    "autostart",
    "download_dir",
    "category_dirs",
    "capture_extensions",
    "domain_denylist",
    "schedule_start",
    "schedule_stop",
    "action_on_complete",
    "allow_local_downloads",
    "max_retries",
    "proxy_mode",
    "proxy_url",
    "postprocess_command",
];

fn is_hh_mm(value: &str) -> bool {
    let Some((hh, mm)) = value.split_once(':') else {
        return false;
    };
    if hh.len() != 2 || mm.len() != 2 {
        return false;
    }
    match (hh.parse::<u32>(), mm.parse::<u32>()) {
        (Ok(h), Ok(m)) => h < 24 && m < 60,
        _ => false,
    }
}

impl Settings {
    /// Apply a JSON patch, rejecting unknown keys and out-of-range values.
    pub fn apply_patch(&mut self, patch: &serde_json::Map<String, Value>) -> Result<()> {
        let mut next = self.clone();
        for (key, value) in patch {
            if !SETTING_KEYS.contains(&key.as_str()) {
                return Err(StoreError::Invalid(format!("unknown setting `{key}`")));
            }
            let merged = serde_json::to_value(&next).unwrap_or(Value::Null);
            let mut object = match merged {
                Value::Object(object) => object,
                _ => serde_json::Map::new(),
            };
            object.insert(key.clone(), value.clone());
            next = serde_json::from_value(Value::Object(object))
                .map_err(|err| StoreError::Invalid(format!("invalid setting `{key}`: {err}")))?;
        }
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Range and format validation shared by `settings.update`.
    pub fn validate(&self) -> Result<()> {
        if !(1..=32).contains(&self.max_connections) {
            return Err(StoreError::Invalid(
                "max_connections must be between 1 and 32".to_string(),
            ));
        }
        if let Some(limit) = self.global_speed_limit_bps {
            if limit > 1024 * 1024 * 1024 {
                return Err(StoreError::Invalid(
                    "global_speed_limit_bps must not exceed 1 GiB/s".to_string(),
                ));
            }
        }
        if self.max_retries > 20 {
            return Err(StoreError::Invalid(
                "max_retries must be between 0 and 20".to_string(),
            ));
        }
        if !Path::new(&self.download_dir).is_absolute() {
            return Err(StoreError::Invalid(
                "download_dir must be an absolute path".to_string(),
            ));
        }
        for (category, dir) in &self.category_dirs {
            if !Path::new(dir).is_absolute() {
                return Err(StoreError::Invalid(format!(
                    "category_dirs[{category}] must be an absolute path"
                )));
            }
        }
        for (name, value) in [
            ("schedule_start", &self.schedule_start),
            ("schedule_stop", &self.schedule_stop),
        ] {
            if let Some(value) = value {
                if !is_hh_mm(value) {
                    return Err(StoreError::Invalid(format!("{name} must be HH:MM")));
                }
            }
        }
        if !matches!(
            self.action_on_complete.as_str(),
            "none" | "suspend" | "power_off" | "exit"
        ) {
            return Err(StoreError::Invalid(
                "action_on_complete must be none, suspend, power_off or exit".to_string(),
            ));
        }
        if !matches!(self.proxy_mode.as_str(), "system" | "none" | "manual") {
            return Err(StoreError::Invalid(
                "proxy_mode must be system, none or manual".to_string(),
            ));
        }
        if self.proxy_mode == "manual" {
            let url = self.proxy_url.as_deref().unwrap_or_default();
            if !(url.starts_with("http://")
                || url.starts_with("https://")
                || url.starts_with("socks5://"))
            {
                return Err(StoreError::Invalid(
                    "proxy_url must be an http(s) or socks5 URL".to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// The database handle. One connection guarded by a mutex: the daemon issues short
/// statements, and WAL keeps readers from blocking the writer.
pub struct Store {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Store {
    /// Open (creating if needed) the database and apply migrations.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            set_mode(parent, 0o700)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        conn.pragma_update(None, "wal_autocheckpoint", 1000)?;
        let store = Store {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
        };
        store.migrate()?;
        set_mode(path, 0o600)?;
        Ok(store)
    }

    /// Path of the database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Apply every pending migration inside one transaction.
    pub fn migrate(&self) -> Result<i64> {
        let conn = self.lock();
        let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current >= SCHEMA_VERSION {
            return Ok(current);
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(SCHEMA_SQL)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
        Ok(SCHEMA_VERSION)
    }

    /// Schema version of the file on disk.
    pub fn schema_version(&self) -> Result<i64> {
        let conn = self.lock();
        Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    /// Journal mode, asserted by the tests to be `wal`.
    pub fn journal_mode(&self) -> Result<String> {
        let conn = self.lock();
        Ok(conn.pragma_query_value(None, "journal_mode", |row| row.get(0))?)
    }

    /// Insert a new task.
    pub fn insert_download(&self, record: &DownloadRecord) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO downloads (
                id, url, final_url, final_path, temp_path, total_bytes, downloaded_bytes,
                status, supports_ranges, etag, last_modified, content_type, expected_checksum,
                error_code, error_message, retry_count, priority, queue_name, download_type
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                record.id,
                record.url,
                record.final_url,
                record.final_path,
                record.temp_path,
                record.total_bytes,
                record.downloaded_bytes,
                record.status.as_str(),
                record.supports_ranges as i64,
                record.etag,
                record.last_modified,
                record.content_type,
                record.expected_checksum,
                record.error_code,
                record.error_message,
                record.retry_count,
                record.priority,
                record.queue_name,
                record.download_type,
            ],
        )?;
        Ok(())
    }

    /// Update lifecycle state and error fields.
    pub fn set_status(
        &self,
        id: &str,
        status: DownloadStatus,
        error: Option<(i64, String)>,
    ) -> Result<()> {
        let conn = self.lock();
        let (code, message) = match &error {
            Some((code, message)) => (Some(*code), Some(message.clone())),
            None => (None, None),
        };
        let completed = if status == DownloadStatus::Completed {
            "CURRENT_TIMESTAMP"
        } else {
            "completed_at"
        };
        conn.execute(
            &format!(
                "UPDATE downloads SET status = ?2, error_code = ?3, error_message = ?4,
                 completed_at = {completed} WHERE id = ?1"
            ),
            params![id, status.as_str(), code, message],
        )?;
        Ok(())
    }

    /// Persist what the probe learned: size, range support and validators.
    pub fn set_probe_result(
        &self,
        id: &str,
        total_bytes: Option<u64>,
        supports_ranges: bool,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE downloads SET total_bytes = COALESCE(?2, total_bytes),
             supports_ranges = ?3, etag = COALESCE(?4, etag),
             last_modified = COALESCE(?5, last_modified) WHERE id = ?1",
            params![
                id,
                total_bytes.map(|value| value as i64),
                supports_ranges as i64,
                etag,
                last_modified
            ],
        )?;
        Ok(())
    }

    /// Persist the offsets and validators of one task after a checkpoint.
    pub fn save_checkpoint(
        &self,
        id: &str,
        segments: &[SegmentRow],
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        etag: Option<&str>,
        last_modified: Option<&str>,
        supports_ranges: bool,
    ) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM segments WHERE download_id = ?1", params![id])?;
        for segment in segments {
            tx.execute(
                "INSERT INTO segments
                    (id, download_id, start_offset, end_offset, next_offset, status)
                 VALUES (?6, ?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    segment.start as i64,
                    segment.end as i64,
                    segment.next as i64,
                    if segment.next > segment.end {
                        "completed"
                    } else {
                        "active"
                    },
                    segment.id
                ],
            )?;
        }
        tx.execute(
            "UPDATE downloads SET downloaded_bytes = ?2, total_bytes = COALESCE(?3, total_bytes),
             etag = COALESCE(?4, etag), last_modified = COALESCE(?5, last_modified),
             supports_ranges = ?6 WHERE id = ?1",
            params![
                id,
                downloaded_bytes as i64,
                total_bytes.map(|v| v as i64),
                etag,
                last_modified,
                supports_ranges as i64
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Update only the counters, used by progress events that do not checkpoint.
    pub fn update_progress(&self, id: &str, downloaded: u64, total: Option<u64>) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE downloads SET downloaded_bytes = ?2, total_bytes = COALESCE(?3, total_bytes)
             WHERE id = ?1",
            params![id, downloaded as i64, total.map(|v| v as i64)],
        )?;
        Ok(())
    }

    /// Fetch one task by id.
    pub fn get(&self, id: &str) -> Result<Option<DownloadRecord>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT id, url, final_url, final_path, temp_path, total_bytes, downloaded_bytes,
                    status, supports_ranges, etag, last_modified, content_type, expected_checksum,
                    error_code, error_message, retry_count, priority, queue_name, download_type,
                    created_at, completed_at
             FROM downloads WHERE id = ?1",
        )?;
        let row = statement.query_row(params![id], row_to_record).optional()?;
        Ok(row)
    }

    /// List tasks, newest first, filtered and paginated.
    pub fn list(
        &self,
        filter: Option<DownloadStatus>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<DownloadRecord>> {
        let conn = self.lock();
        let limit = limit.clamp(1, 1000) as i64;
        let offset = offset as i64;
        let mut rows = Vec::new();
        match filter {
            Some(status) => {
                let mut statement = conn.prepare(
                    "SELECT id, url, final_url, final_path, temp_path, total_bytes, downloaded_bytes,
                            status, supports_ranges, etag, last_modified, content_type, expected_checksum,
                            error_code, error_message, retry_count, priority, queue_name, download_type,
                            created_at, completed_at
                     FROM downloads WHERE status = ?3 ORDER BY created_at DESC, rowid DESC LIMIT ?1 OFFSET ?2",
                )?;
                let mapped =
                    statement.query_map(params![limit, offset, status.as_str()], row_to_record)?;
                for row in mapped {
                    rows.push(row?);
                }
            }
            None => {
                let mut statement = conn.prepare(
                    "SELECT id, url, final_url, final_path, temp_path, total_bytes, downloaded_bytes,
                            status, supports_ranges, etag, last_modified, content_type, expected_checksum,
                            error_code, error_message, retry_count, priority, queue_name, download_type,
                            created_at, completed_at
                     FROM downloads ORDER BY created_at DESC, rowid DESC LIMIT ?1 OFFSET ?2",
                )?;
                let mapped = statement.query_map(params![limit, offset], row_to_record)?;
                for row in mapped {
                    rows.push(row?);
                }
            }
        }
        Ok(rows)
    }

    /// Segments of one task, ordered by start offset.
    pub fn segments(&self, id: &str) -> Result<Vec<SegmentRow>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT id, start_offset, end_offset, next_offset FROM segments
             WHERE download_id = ?1 ORDER BY start_offset ASC",
        )?;
        let mapped = statement.query_map(params![id], |row| {
            Ok(SegmentRow {
                id: row.get::<_, i64>(0)? as u32,
                start: row.get::<_, i64>(1)? as u64,
                end: row.get::<_, i64>(2)? as u64,
                next: row.get::<_, i64>(3)? as u64,
            })
        })?;
        let mut rows = Vec::new();
        for row in mapped {
            rows.push(row?);
        }
        Ok(rows)
    }

    /// Delete a task and everything that cascades from it.
    pub fn delete(&self, id: &str) -> Result<bool> {
        let conn = self.lock();
        let affected = conn.execute("DELETE FROM downloads WHERE id = ?1", params![id])?;
        Ok(affected > 0)
    }

    /// Non-sensitive headers of a task.
    pub fn headers(&self, id: &str) -> Result<BTreeMap<String, String>> {
        let conn = self.lock();
        let json: Option<String> = conn
            .query_row(
                "SELECT headers_json FROM download_headers WHERE download_id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(json
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default())
    }

    /// Replace the non-sensitive headers of a task.
    pub fn set_headers(&self, id: &str, headers: &BTreeMap<String, String>) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO download_headers (download_id, headers_json) VALUES (?1, ?2)
             ON CONFLICT(download_id) DO UPDATE SET headers_json = excluded.headers_json",
            params![
                id,
                serde_json::to_string(headers).unwrap_or_else(|_| "{}".into())
            ],
        )?;
        Ok(())
    }

    /// Secret alias of a task, if any.
    pub fn secret_alias(&self, id: &str) -> Result<Option<String>> {
        let conn = self.lock();
        Ok(conn
            .query_row(
                "SELECT keyring_secret_id FROM download_secrets WHERE download_id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Point a task at a secret alias.
    pub fn set_secret_alias(&self, id: &str, alias: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO download_secrets (download_id, keyring_secret_id) VALUES (?1, ?2)
             ON CONFLICT(download_id) DO UPDATE SET keyring_secret_id = excluded.keyring_secret_id",
            params![id, alias],
        )?;
        Ok(())
    }

    /// Mark every in-flight task as paused after a restart.
    pub fn recover_crashed(&self) -> Result<Vec<String>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT id FROM downloads WHERE status IN
             ('downloading', 'probing', 'verifying', 'finalizing', 'retrying')",
        )?;
        let ids: Vec<String> = statement
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        drop(statement);
        conn.execute(
            "UPDATE downloads SET status = 'paused' WHERE status IN
             ('downloading', 'probing', 'verifying', 'finalizing', 'retrying')",
            [],
        )?;
        Ok(ids)
    }

    /// Effective settings, with defaults for missing keys.
    pub fn settings(&self) -> Result<Settings> {
        let conn = self.lock();
        let mut statement = conn.prepare("SELECT key, value_json FROM settings")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut settings = Settings::default();
        let mut object =
            serde_json::to_value(&settings).unwrap_or(Value::Object(Default::default()));
        for row in rows {
            let (key, raw) = row?;
            if let Value::Object(map) = &mut object {
                if let Ok(value) = serde_json::from_str::<Value>(&raw) {
                    map.insert(key, value);
                }
            }
        }
        settings = serde_json::from_value(object)
            .map_err(|err| StoreError::Invalid(format!("stored settings are invalid: {err}")))?;
        Ok(settings)
    }

    /// Apply a partial update and persist only the keys the caller sent.
    pub fn update_settings(&self, patch: &serde_json::Map<String, Value>) -> Result<Settings> {
        let mut settings = self.settings()?;
        settings.apply_patch(patch)?;
        let object =
            serde_json::to_value(&settings).map_err(|err| StoreError::Invalid(err.to_string()))?;
        let Value::Object(map) = object else {
            return Err(StoreError::Invalid(
                "settings are not an object".to_string(),
            ));
        };
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for key in patch.keys() {
            let Some(value) = map.get(key) else { continue };
            tx.execute(
                "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
                params![key, value.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(settings)
    }

    /// Queues with their live counters.
    pub fn queues(&self) -> Result<Vec<QueueRow>> {
        let conn = self.lock();
        let mut statement =
            conn.prepare("SELECT name, max_active FROM queues ORDER BY name ASC")?;
        let mapped = statement.query_map([], |row| {
            Ok(QueueRow {
                name: row.get(0)?,
                max_active: row.get::<_, i64>(1)? as u32,
            })
        })?;
        let mut rows = Vec::new();
        for row in mapped {
            rows.push(row?);
        }
        Ok(rows)
    }

    /// Create a queue with validated name and concurrency.
    pub fn create_queue(&self, name: &str, max_active: u32) -> Result<()> {
        if name.is_empty() || name.len() > 64 {
            return Err(StoreError::Queue(
                "queue name must be 1-64 characters".into(),
            ));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(StoreError::Queue("queue name must match [a-z0-9_-]".into()));
        }
        if !(1..=8).contains(&max_active) {
            return Err(StoreError::Queue(
                "max_active must be between 1 and 8".into(),
            ));
        }
        let conn = self.lock();
        conn.execute(
            "INSERT INTO queues (name, max_active) VALUES (?1, ?2)",
            params![name, max_active as i64],
        )
        .map_err(|err| match err {
            rusqlite::Error::SqliteFailure(failure, _)
                if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                StoreError::Queue(format!("queue `{name}` already exists"))
            }
            other => StoreError::Sqlite(other),
        })?;
        Ok(())
    }

    /// Remove a queue; its pending tasks move to `main`.
    pub fn remove_queue(&self, name: &str) -> Result<()> {
        if name == "main" {
            return Err(StoreError::Queue("the main queue cannot be removed".into()));
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let active: i64 = tx.query_row(
            "SELECT COUNT(*) FROM downloads WHERE queue_name = ?1 AND status IN
             ('downloading', 'probing', 'retrying', 'verifying', 'finalizing')",
            params![name],
            |row| row.get(0),
        )?;
        if active > 0 {
            return Err(StoreError::Queue(format!(
                "queue `{name}` still has {active} active task(s)"
            )));
        }
        tx.execute(
            "UPDATE downloads SET queue_name = 'main' WHERE queue_name = ?1",
            params![name],
        )?;
        tx.execute("DELETE FROM queues WHERE name = ?1", params![name])?;
        tx.commit()?;
        Ok(())
    }

    /// Store the scheduler of a queue.
    pub fn schedule_queue(&self, name: &str, schedule: &Value) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let exists: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM queues WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(StoreError::Queue(format!("queue `{name}` does not exist")));
        }
        tx.execute(
            "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
            params![format!("queue_schedule:{name}"), schedule.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<DownloadRecord> {
    let status: String = row.get(7)?;
    let final_path: String = row.get(3)?;
    let filename = Path::new(&final_path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(DownloadRecord {
        id: row.get(0)?,
        url: row.get(1)?,
        final_url: row.get(2)?,
        final_path,
        temp_path: row.get(4)?,
        total_bytes: row.get::<_, Option<i64>>(5)?.map(|value| value as u64),
        downloaded_bytes: row.get::<_, i64>(6)? as u64,
        status: DownloadStatus::parse(&status).unwrap_or(DownloadStatus::Failed),
        supports_ranges: row.get::<_, i64>(8)? != 0,
        etag: row.get(9)?,
        last_modified: row.get(10)?,
        content_type: row.get(11)?,
        expected_checksum: row.get(12)?,
        error_code: row.get(13)?,
        error_message: row.get(14)?,
        retry_count: row.get::<_, i64>(15)? as u32,
        priority: row.get(16)?,
        queue_name: row.get(17)?,
        download_type: row.get(18)?,
        created_at: row.get(19)?,
        completed_at: row.get(20)?,
        filename,
    })
}

fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
}
