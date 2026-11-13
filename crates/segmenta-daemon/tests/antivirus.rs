//! Antivirus and quarantine contract (A2: OpenSpec 12 §5, OpenSpec 08 §4.2.2).
//!
//! Every scanner is a real script written into a tempdir and executed by path: no test
//! goes through a shell, no test depends on the network and no test asserts on wall-clock
//! time beyond a generous upper bound. Engine detection is driven by
//! `SEGMENTA_ANTIVIRUS_PATH`, the same kind of override `SEGMENTA_YT_DLP` is for media, so
//! a test never needs the real ClamAV and never inherits one from the host.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use segmenta_daemon::postprocess::{
    detect_engine_in, quarantine, scan_with, ScanEngine, ScanOptions, ScanVerdict,
    DEFAULT_CAPTURE_LIMIT,
};
use segmenta_daemon::store::{DownloadStatus, Settings, Store, StoreError, SCHEMA_VERSION};

/// The `downloads` table exactly as schema v3 leaves it, plus the parent table and the
/// settings table, so the fixture is a database a 0.1.2 daemon really produced.
const V3_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS queues (
    name TEXT PRIMARY KEY,
    max_active INTEGER NOT NULL DEFAULT 1 CHECK(max_active BETWEEN 1 AND 8),
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
INSERT OR IGNORE INTO queues(name, max_active) VALUES ('main', 3);

CREATE TABLE downloads (
    id TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    final_url TEXT,
    final_path TEXT NOT NULL,
    temp_path TEXT,
    total_bytes INTEGER,
    downloaded_bytes INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL CHECK(status IN (
        'pending', 'probing', 'downloading', 'paused',
        'retrying', 'verifying', 'finalizing',
        'completed', 'failed', 'cancelled'
    )),
    supports_ranges BOOLEAN NOT NULL DEFAULT 0,
    etag TEXT,
    last_modified TEXT,
    content_type TEXT,
    expected_checksum TEXT,
    error_code INTEGER,
    error_message TEXT,
    retry_count INTEGER NOT NULL DEFAULT 0,
    priority INTEGER NOT NULL DEFAULT 0,
    queue_name TEXT NOT NULL DEFAULT 'main' REFERENCES queues(name),
    download_type TEXT NOT NULL DEFAULT 'direct' CHECK(download_type IN (
        'direct', 'hls', 'dash', 'youtube_video', 'youtube_playlist', 'youtube_audio', 'torrent', 'magnet'
    )),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP,
    media_kind TEXT,
    media_format TEXT,
    playlist_title TEXT,
    playlist_count INTEGER,
    batch_id TEXT
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL
);
"#;

/// Columns `downloads` must carry once schema v4 is applied, with their declared types.
const V4_COLUMNS: [(&str, &str); 3] = [
    ("quarantine_path", "TEXT"),
    ("postprocess_exit", "INTEGER"),
    ("postprocess_message", "TEXT"),
];

/// Write a real v3 database with three rows, one of them a media task.
fn seed_v3(path: &Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.pragma_update(None, "user_version", 3).unwrap();
    conn.execute_batch(V3_DDL).unwrap();
    conn.execute_batch(
        r#"
        INSERT INTO downloads
            (id, url, final_path, status, downloaded_bytes, total_bytes, media_kind, batch_id)
        VALUES
            ('old-media', 'http://example.test/video.mp4', '/tmp/video.mp4', 'completed',
             2048, 2048, 'video', 'batch-1'),
            ('old-failed', 'http://example.test/broken.bin', '/tmp/broken.bin', 'failed',
             0, 0, NULL, NULL),
            ('old-queued', 'http://example.test/later.iso', '/tmp/later.iso', 'pending',
             0, NULL, NULL, NULL);
        INSERT INTO settings (key, value_json) VALUES ('max_connections', '8');
        "#,
    )
    .unwrap();
}

/// Number of columns of `downloads` named `column`: 0 when v4 was not applied and 2 when
/// the migration ran twice.
fn column_count(path: &Path, column: &str) -> i64 {
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut statement = conn
        .prepare("SELECT COUNT(*) FROM pragma_table_info('downloads') WHERE name = ?1")
        .unwrap();
    statement
        .query_row([column], |row| row.get::<_, i64>(0))
        .unwrap()
}

fn declared_type(path: &Path, column: &str) -> String {
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut statement = conn
        .prepare("SELECT type FROM pragma_table_info('downloads') WHERE name = ?1")
        .unwrap();
    statement
        .query_row([column], |row| row.get::<_, String>(0))
        .unwrap()
}

#[test]
fn migration_from_v3_adds_the_three_v4_columns_and_keeps_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("segmenta-v3.db");
    seed_v3(&path);

    let store = Store::open(&path).unwrap();

    // A2 owns v4; v5 (site logins by origin) is already in the chain.
    assert!(
        SCHEMA_VERSION >= 4,
        "A2 owns schema v4 (OpenSpec 08 §4.2.2)"
    );
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);

    // No row is added, lost or rewritten: the migration only widens the table.
    let rows = store.list(None, 10, 0).unwrap();
    assert_eq!(rows.len(), 3, "the three pre-existing rows stay");
    let old = store.get("old-media").unwrap().expect("row survived");
    assert_eq!(old.url, "http://example.test/video.mp4");
    assert_eq!(old.final_path, "/tmp/video.mp4");
    assert_eq!(old.status, DownloadStatus::Completed);
    assert_eq!(old.downloaded_bytes, 2048);
    assert_eq!(old.media_kind.as_deref(), Some("video"));
    assert_eq!(old.batch_id.as_deref(), Some("batch-1"));
    assert_eq!(
        store.get("old-failed").unwrap().unwrap().status,
        DownloadStatus::Failed
    );
    assert_eq!(
        store.get("old-queued").unwrap().unwrap().status,
        DownloadStatus::Pending
    );

    // The three v4 columns exist, exactly once each, with the agreed types.
    for (column, kind) in V4_COLUMNS {
        assert_eq!(column_count(&path, column), 1, "{column} must exist once");
        assert_eq!(declared_type(&path, column), kind, "{column} type");
    }

    // The stored settings survive too.
    assert_eq!(store.settings().unwrap().max_connections, 8);
}

#[test]
fn the_v4_migration_is_idempotent_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("segmenta-v3.db");
    seed_v3(&path);

    {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    }
    // Reopening must not re-run the DDL: a second `ADD COLUMN` would fail outright and,
    // if it did not, would duplicate the columns.
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(store.list(None, 10, 0).unwrap().len(), 3);
    for (column, _) in V4_COLUMNS {
        assert_eq!(
            column_count(&path, column),
            1,
            "{column} must not duplicate"
        );
    }
    assert_eq!(store.settings().unwrap().max_connections, 8);
}

// ---------------------------------------------------------------------------
// Settings (OpenSpec 12 §5)
// ---------------------------------------------------------------------------

#[test]
fn settings_ship_the_documented_antivirus_defaults() {
    let settings = Settings::default();
    assert!(!settings.antivirus_enabled, "the scan is opt-in");
    assert_eq!(settings.antivirus_timeout_secs, 120);
    let quarantine = Path::new(&settings.antivirus_quarantine_dir);
    assert!(
        quarantine.is_absolute(),
        "the default quarantine is absolute: {quarantine:?}"
    );
    assert!(
        quarantine.ends_with("segmenta/quarantine"),
        "the default quarantine lives under the data dir: {quarantine:?}"
    );
}

#[test]
fn settings_update_accepts_and_persists_the_antivirus_keys() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("segmenta.db")).unwrap();
    let quarantine = dir.path().join("cuarentena");
    let patch = serde_json::json!({
        "antivirus_enabled": true,
        "antivirus_timeout_secs": 300,
        "antivirus_quarantine_dir": quarantine.display().to_string(),
    });

    let updated = store.update_settings(patch.as_object().unwrap()).unwrap();

    assert!(updated.antivirus_enabled);
    assert_eq!(updated.antivirus_timeout_secs, 300);
    assert_eq!(
        updated.antivirus_quarantine_dir,
        quarantine.display().to_string()
    );
    let reread = store.settings().unwrap();
    assert!(reread.antivirus_enabled, "the values survive a re-read");
    assert_eq!(reread.antivirus_timeout_secs, 300);
    assert_eq!(
        reread.antivirus_quarantine_dir,
        quarantine.display().to_string()
    );
}

#[test]
fn settings_reject_out_of_range_antivirus_values() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("segmenta.db")).unwrap();

    for patch in [
        serde_json::json!({ "antivirus_timeout_secs": 0 }),
        serde_json::json!({ "antivirus_timeout_secs": 3601 }),
        serde_json::json!({ "antivirus_quarantine_dir": "cuarentena/relativa" }),
        serde_json::json!({ "antivirus_enabled": "si" }),
    ] {
        let error = store
            .update_settings(patch.as_object().unwrap())
            .expect_err(&format!("{patch} must be rejected"));
        assert!(
            matches!(error, StoreError::Invalid(_)),
            "{patch} must be an invalid setting, got {error:?}"
        );
    }
    assert!(
        !store.settings().unwrap().antivirus_enabled,
        "a rejected patch leaves the stored settings untouched"
    );
}

// ---------------------------------------------------------------------------
// Engine detection and one scan (OpenSpec 12 §5)
// ---------------------------------------------------------------------------

/// Write an executable script into `dir`, renamed into place so a concurrent `fork` of
/// another test cannot make `execve` fail with ETXTBSY.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    let staging = dir.join(format!(".{name}.staging"));
    std::fs::write(&staging, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staging, &path).unwrap();
    path
}

fn target_file(dir: &Path, name: &str) -> PathBuf {
    let file = dir.join(name);
    std::fs::write(&file, b"payload").unwrap();
    file
}

fn scan_options(timeout: Duration) -> ScanOptions {
    ScanOptions {
        timeout,
        capture_limit: DEFAULT_CAPTURE_LIMIT,
    }
}

#[test]
fn engine_detection_prefers_clamdscan_over_clamscan() {
    let dir = tempfile::tempdir().unwrap();
    script(dir.path(), "clamscan", "exit 0");
    script(dir.path(), "clamdscan", "exit 0");

    let engine = detect_engine_in(&[dir.path().to_path_buf()]).expect("an engine");

    assert_eq!(engine.kind, ScanEngine::Clamdscan);
    assert_eq!(engine.binary, dir.path().join("clamdscan"));
}

#[test]
fn engine_detection_falls_back_to_clamscan() {
    let dir = tempfile::tempdir().unwrap();
    script(dir.path(), "clamscan", "exit 0");

    let engine = detect_engine_in(&[dir.path().to_path_buf()]).expect("an engine");

    assert_eq!(engine.kind, ScanEngine::Clamscan);
    assert_eq!(engine.binary, dir.path().join("clamscan"));
}

#[test]
fn engine_detection_ignores_what_cannot_be_executed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clamscan");
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert!(
        detect_engine_in(&[dir.path().to_path_buf()]).is_none(),
        "a file without the execute bit is not a usable engine"
    );
}

#[test]
fn no_engine_is_reported_as_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let file = target_file(dir.path(), "file.bin");

    let report = scan_with(None, &file, &scan_options(Duration::from_secs(5)));

    assert_eq!(report.verdict(), ScanVerdict::Unavailable);
    assert_eq!(report.exit_code, None);
    assert!(
        report.message.contains("ERR_ANTIVIRUS_UNAVAILABLE"),
        "the record must name the condition: {}",
        report.message
    );
    assert!(file.exists(), "an unavailable scan never touches the file");
}

#[test]
fn a_clean_file_is_clean() {
    let dir = tempfile::tempdir().unwrap();
    script(dir.path(), "clamscan", "exit 0");
    let file = target_file(dir.path(), "file.bin");
    let engine = detect_engine_in(&[dir.path().to_path_buf()]);

    let report = scan_with(engine, &file, &scan_options(Duration::from_secs(5)));

    assert_eq!(report.verdict(), ScanVerdict::Clean, "{}", report.message);
    assert_eq!(report.exit_code, Some(0));
    assert_eq!(
        report.engine.map(|engine| engine.kind),
        Some(ScanEngine::Clamscan)
    );
    assert!(file.exists());
}

#[test]
fn an_infected_file_is_infected_with_the_scanner_message() {
    let dir = tempfile::tempdir().unwrap();
    // ClamAV exits 1 on infection; the file travels as the last argument after
    // `--no-summary`, so a fake can echo it back without any shell interpretation.
    script(
        dir.path(),
        "clamscan",
        "printf '%s: Eicar-Test-Signature FOUND\\n' \"$2\"\nexit 1",
    );
    let file = target_file(dir.path(), "eicar.com");
    let engine = detect_engine_in(&[dir.path().to_path_buf()]);

    let report = scan_with(engine, &file, &scan_options(Duration::from_secs(5)));

    assert_eq!(
        report.verdict(),
        ScanVerdict::Infected,
        "{}",
        report.message
    );
    assert_eq!(report.exit_code, Some(1));
    assert!(
        report.message.contains("Eicar-Test-Signature"),
        "the scanner message is recorded: {}",
        report.message
    );
    assert!(
        file.exists(),
        "the scan itself never removes the file: the quarantine step does"
    );
}

#[test]
fn an_infected_file_leaves_the_downloads_folder_for_the_quarantine() {
    let dir = tempfile::tempdir().unwrap();
    let downloads = dir.path().join("Descargas");
    let quarantine_dir = dir.path().join("cuarentena");
    std::fs::create_dir_all(&downloads).unwrap();
    let file = target_file(&downloads, "eicar.com");

    let moved = quarantine(&file, &quarantine_dir).expect("an infected file must be quarantined");

    assert!(
        !file.exists(),
        "the file must not stay in the downloads folder"
    );
    assert!(moved.exists(), "the quarantined copy exists");
    assert_eq!(moved.parent(), Some(quarantine_dir.as_path()));
    assert_eq!(moved.file_name(), Some(std::ffi::OsStr::new("eicar.com")));
    assert_eq!(
        std::fs::read(&moved).unwrap_or_default(),
        std::fs::read(target_file(dir.path(), "eicar.com")).unwrap_or_default(),
        "the quarantined file carries the same bytes the scanner rejected"
    );
}

#[test]
fn a_second_infected_file_with_the_same_name_does_not_overwrite_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let quarantine_dir = dir.path().join("cuarentena");
    let first = target_file(dir.path(), "same.bin");
    let first_copy = quarantine(&first, &quarantine_dir).unwrap();
    let second = target_file(dir.path(), "same.bin");
    let second_copy = quarantine(&second, &quarantine_dir).unwrap();

    assert_ne!(
        first_copy, second_copy,
        "a repeated name gets its own quarantined file"
    );
    assert!(first_copy.exists() && second_copy.exists());
}

#[test]
fn a_scan_timeout_is_a_failure_and_never_an_infection() {
    let dir = tempfile::tempdir().unwrap();
    script(dir.path(), "clamscan", "/bin/sleep 30");
    let file = target_file(dir.path(), "file.bin");
    let engine = detect_engine_in(&[dir.path().to_path_buf()]);

    let started = Instant::now();
    let report = scan_with(engine, &file, &scan_options(Duration::from_millis(300)));
    let elapsed = started.elapsed();

    assert_eq!(report.verdict(), ScanVerdict::Failed, "{}", report.message);
    assert_eq!(report.exit_code, None);
    assert!(
        report.message.to_lowercase().contains("timeout"),
        "the record must explain the kill: {}",
        report.message
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "bounded time: {elapsed:?}"
    );
    assert!(file.exists(), "a timeout never deletes the file");
}

#[test]
fn a_scanner_error_is_a_failure_and_never_an_infection() {
    let dir = tempfile::tempdir().unwrap();
    script(
        dir.path(),
        "clamscan",
        "echo 'LibClamAV Error: can not open database' >&2\nexit 2",
    );
    let file = target_file(dir.path(), "file.bin");
    let engine = detect_engine_in(&[dir.path().to_path_buf()]);

    let report = scan_with(engine, &file, &scan_options(Duration::from_secs(5)));

    assert_eq!(report.verdict(), ScanVerdict::Failed, "{}", report.message);
    assert_eq!(report.exit_code, Some(2));
    assert!(
        report.message.contains("LibClamAV"),
        "the scanner output is recorded: {}",
        report.message
    );
    assert!(file.exists(), "a scanner error never deletes the file");
}
