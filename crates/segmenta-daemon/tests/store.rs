//! Persistence contract: the DDL of OpenSpec 08, migrations, checkpoint state, settings
//! and the guarantee that a secret never lands in the database file.

use std::collections::{BTreeMap, HashMap};

use segmenta_daemon::secrets::{FileSecretStore, SecretStore};
use segmenta_daemon::store::{
    Checkpoint, DownloadRecord, DownloadStatus, SegmentRow, Settings, Store, SCHEMA_VERSION,
};
use serde_json::json;

fn record(id: &str, status: DownloadStatus) -> DownloadRecord {
    DownloadRecord {
        id: id.to_string(),
        url: "http://example.test/file.bin".to_string(),
        final_url: None,
        final_path: format!("/tmp/{id}.bin"),
        temp_path: None,
        total_bytes: Some(1024),
        downloaded_bytes: 0,
        status,
        supports_ranges: true,
        etag: Some("\"v1\"".to_string()),
        last_modified: None,
        content_type: Some("application/octet-stream".to_string()),
        expected_checksum: None,
        error_code: None,
        error_message: None,
        retry_count: 0,
        priority: 0,
        queue_name: "main".to_string(),
        download_type: "direct".to_string(),
        created_at: None,
        completed_at: None,
        filename: format!("{id}.bin"),
        media_kind: None,
        media_format: None,
        playlist_title: None,
        playlist_count: None,
        batch_id: None,
    }
}

fn open_temp() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("segmenta.db")).unwrap();
    (dir, store)
}

#[test]
fn migration_is_idempotent_and_records_the_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("segmenta.db");
    {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(store.journal_mode().unwrap().to_lowercase(), "wal");
    }
    // Reopening must not re-run the DDL or lose data.
    {
        let store = Store::open(&path).unwrap();
        store
            .insert_download(&record("abc", DownloadStatus::Pending))
            .unwrap();
        assert_eq!(store.list(None, 10, 0).unwrap().len(), 1);
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(store.list(None, 10, 0).unwrap().len(), 1);
}

#[test]
fn the_main_queue_exists_after_migration() {
    let (_dir, store) = open_temp();
    let queues = store.queues().unwrap();
    assert_eq!(queues.len(), 1);
    assert_eq!(queues[0].name, "main");
}

#[test]
fn downloads_round_trip_and_are_ordered_newest_first() {
    let (_dir, store) = open_temp();
    store
        .insert_download(&record("first", DownloadStatus::Pending))
        .unwrap();
    store
        .insert_download(&record("second", DownloadStatus::Downloading))
        .unwrap();
    let rows = store.list(None, 10, 0).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "second", "newest first");

    let fetched = store.get("first").unwrap().expect("row exists");
    assert_eq!(fetched.total_bytes, Some(1024));
    assert_eq!(fetched.status, DownloadStatus::Pending);

    let filtered = store
        .list(Some(DownloadStatus::Downloading), 10, 0)
        .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, "second");

    let page = store.list(None, 1, 1).unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id, "first");
}

#[test]
fn status_transitions_store_the_error_and_completion_time() {
    let (_dir, store) = open_temp();
    store
        .insert_download(&record("t", DownloadStatus::Downloading))
        .unwrap();
    store
        .set_status(
            "t",
            DownloadStatus::Failed,
            Some((1006, "disk full".into())),
        )
        .unwrap();
    let row = store.get("t").unwrap().unwrap();
    assert_eq!(row.status, DownloadStatus::Failed);
    assert_eq!(row.error_code, Some(1006));
    assert_eq!(row.error_message.as_deref(), Some("disk full"));

    store
        .set_status("t", DownloadStatus::Completed, None)
        .unwrap();
    let row = store.get("t").unwrap().unwrap();
    assert!(row.completed_at.is_some());
    assert!(row.error_code.is_none());
}

#[test]
fn checkpoints_replace_segments_and_keep_next_offset() {
    let (_dir, store) = open_temp();
    store
        .insert_download(&record("t", DownloadStatus::Downloading))
        .unwrap();
    let segments = vec![
        SegmentRow {
            id: 1,
            start: 0,
            end: 499,
            next: 200,
        },
        SegmentRow {
            id: 2,
            start: 500,
            end: 1023,
            next: 500,
        },
    ];
    store
        .save_checkpoint(
            "t",
            &Checkpoint {
                segments: &segments,
                downloaded_bytes: 200,
                total_bytes: Some(1024),
                etag: Some("\"v1\""),
                last_modified: None,
                supports_ranges: true,
            },
        )
        .unwrap();

    let rows = store.segments("t").unwrap();
    // The row id is the table-wide rowid, not the engine's per-download id; the offsets
    // are the contract.
    assert_eq!(
        rows.iter()
            .map(|row| (row.start, row.end, row.next))
            .collect::<Vec<_>>(),
        vec![(0, 499, 200), (500, 1023, 500)]
    );
    let row = store.get("t").unwrap().unwrap();
    assert_eq!(row.downloaded_bytes, 200);
    assert_eq!(row.total_bytes, Some(1024));

    // A second checkpoint replaces the previous rows instead of appending.
    let updated = vec![
        SegmentRow {
            id: 1,
            start: 0,
            end: 499,
            next: 500,
        },
        SegmentRow {
            id: 2,
            start: 500,
            end: 1023,
            next: 700,
        },
    ];
    store
        .save_checkpoint(
            "t",
            &Checkpoint {
                segments: &updated,
                downloaded_bytes: 700,
                total_bytes: None,
                etag: None,
                last_modified: None,
                supports_ranges: true,
            },
        )
        .unwrap();
    assert_eq!(
        store
            .segments("t")
            .unwrap()
            .iter()
            .map(|row| (row.start, row.end, row.next))
            .collect::<Vec<_>>(),
        vec![(0, 499, 500), (500, 1023, 700)]
    );
    assert_eq!(store.get("t").unwrap().unwrap().downloaded_bytes, 700);
}

#[test]
fn the_schema_rejects_impossible_segment_offsets() {
    let (_dir, store) = open_temp();
    store
        .insert_download(&record("t", DownloadStatus::Downloading))
        .unwrap();
    // next_offset must never exceed end_offset + 1.
    let bad = vec![SegmentRow {
        id: 1,
        start: 0,
        end: 99,
        next: 101,
    }];
    assert!(
        store
            .save_checkpoint(
                "t",
                &Checkpoint {
                    segments: &bad,
                    downloaded_bytes: 0,
                    total_bytes: None,
                    etag: None,
                    last_modified: None,
                    supports_ranges: true,
                },
            )
            .is_err(),
        "the CHECK constraint must reject next > end + 1"
    );
}

#[test]
fn segments_and_headers_cascade_on_delete() {
    let (_dir, store) = open_temp();
    store
        .insert_download(&record("t", DownloadStatus::Downloading))
        .unwrap();
    store
        .save_checkpoint(
            "t",
            &Checkpoint {
                segments: &[SegmentRow {
                    id: 1,
                    start: 0,
                    end: 1023,
                    next: 5,
                }],
                downloaded_bytes: 5,
                total_bytes: None,
                etag: None,
                last_modified: None,
                supports_ranges: true,
            },
        )
        .unwrap();
    let mut headers = BTreeMap::new();
    headers.insert("User-Agent".to_string(), "Segmenta/test".to_string());
    store.set_headers("t", &headers).unwrap();
    store.set_secret_alias("t", "segmenta/t").unwrap();

    assert!(store.delete("t").unwrap());
    assert!(store.segments("t").unwrap().is_empty());
    assert!(store.headers("t").unwrap().is_empty());
    assert_eq!(store.secret_alias("t").unwrap(), None);
    assert!(store.get("t").unwrap().is_none());
}

#[test]
fn crash_recovery_pauses_every_in_flight_state() {
    let (_dir, store) = open_temp();
    for (id, status) in [
        ("a", DownloadStatus::Downloading),
        ("b", DownloadStatus::Probing),
        ("c", DownloadStatus::Verifying),
        ("d", DownloadStatus::Finalizing),
        ("e", DownloadStatus::Completed),
        ("f", DownloadStatus::Paused),
    ] {
        store.insert_download(&record(id, status)).unwrap();
    }
    let recovered = store.recover_crashed().unwrap();
    assert_eq!(recovered.len(), 4, "only in-flight tasks are reconciled");
    for id in ["a", "b", "c", "d"] {
        assert_eq!(
            store.get(id).unwrap().unwrap().status,
            DownloadStatus::Paused
        );
    }
    assert_eq!(
        store.get("e").unwrap().unwrap().status,
        DownloadStatus::Completed
    );
    assert_eq!(
        store.get("f").unwrap().unwrap().status,
        DownloadStatus::Paused
    );
}

#[test]
fn settings_start_from_the_catalog_defaults() {
    let (_dir, store) = open_temp();
    let settings = store.settings().unwrap();
    assert_eq!(settings.max_connections, 16);
    assert_eq!(settings.max_retries, 5);
    assert_eq!(settings.proxy_mode, "system");
    assert_eq!(settings.action_on_complete, "none");
    assert!(!settings.allow_local_downloads);
    assert!(settings.capture_extensions.contains(&"iso".to_string()));
}

#[test]
fn settings_update_validates_types_ranges_and_unknown_keys() {
    let (_dir, store) = open_temp();
    let patch = json!({ "max_connections": 8, "global_speed_limit_bps": 1_000_000 });
    let updated = store.update_settings(patch.as_object().unwrap()).unwrap();
    assert_eq!(updated.max_connections, 8);
    assert_eq!(updated.global_speed_limit_bps, Some(1_000_000));
    // Persisted: a new handle sees them too.
    assert_eq!(store.settings().unwrap().max_connections, 8);

    for bad in [
        json!({ "max_connections": 0 }),
        json!({ "max_connections": 33 }),
        json!({ "max_retries": 21 }),
        json!({ "unknown_key": true }),
        json!({ "proxy_mode": "often" }),
        json!({ "action_on_complete": "reboot" }),
        json!({ "schedule_start": "25:00" }),
        json!({ "download_dir": "relative/path" }),
        json!({ "max_connections": "many" }),
        json!({ "appearance": "sepia" }),
    ] {
        assert!(
            store.update_settings(bad.as_object().unwrap()).is_err(),
            "must reject {bad}"
        );
    }
    // A rejected patch changes nothing.
    assert_eq!(store.settings().unwrap().max_connections, 8);
}

#[test]
fn queue_lifecycle_enforces_the_contract() {
    let (_dir, store) = open_temp();
    store.create_queue("noche", 1).unwrap();
    assert!(store.create_queue("noche", 1).is_err(), "names are unique");
    assert!(store.create_queue("Bad Name", 1).is_err());
    assert!(store.create_queue("ok", 9).is_err());

    store
        .insert_download(&record("task", DownloadStatus::Downloading))
        .unwrap();
    assert!(
        store.remove_queue("main").is_err(),
        "main cannot be removed"
    );
    let mut named = record("task2", DownloadStatus::Paused);
    named.queue_name = "noche".to_string();
    store.insert_download(&named).unwrap();
    store.remove_queue("noche").unwrap();
    let moved = store.get("task2").unwrap().unwrap();
    assert_eq!(moved.queue_name, "main");
    assert_eq!(store.queues().unwrap().len(), 1);
}

#[test]
fn queue_schedule_is_stored() {
    let (_dir, store) = open_temp();
    store
        .schedule_queue(
            "main",
            &json!({"start_time": "02:00", "stop_time": "06:00", "action_on_complete": "power_off"}),
        )
        .unwrap();
    assert!(store.schedule_queue("nope", &json!({})).is_err());
}

#[test]
fn migration_from_v2_to_v3_preserves_data_and_creates_torrent_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("segmenta-v2.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        conn.execute_batch(r#"
            CREATE TABLE IF NOT EXISTS queues (name TEXT PRIMARY KEY, max_active INTEGER NOT NULL DEFAULT 1);
            INSERT OR IGNORE INTO queues(name, max_active) VALUES ('main', 3);
            CREATE TABLE downloads (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                final_url TEXT,
                final_path TEXT NOT NULL,
                temp_path TEXT,
                total_bytes INTEGER,
                downloaded_bytes INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL,
                supports_ranges BOOLEAN NOT NULL DEFAULT 0,
                etag TEXT,
                last_modified TEXT,
                content_type TEXT,
                expected_checksum TEXT,
                error_code INTEGER,
                error_message TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                priority INTEGER NOT NULL DEFAULT 0,
                queue_name TEXT NOT NULL DEFAULT 'main',
                download_type TEXT NOT NULL DEFAULT 'direct',
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                completed_at TIMESTAMP,
                media_kind TEXT,
                media_format TEXT,
                playlist_title TEXT,
                playlist_count INTEGER,
                batch_id TEXT
            );
            INSERT INTO downloads(id, url, final_path, status) VALUES ('old-1', 'http://example.test', '/tmp/old.bin', 'completed');
        "#).unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(SCHEMA_VERSION, 3);

    let rows = store.list(None, 10, 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "old-1");
    assert_eq!(rows[0].status, DownloadStatus::Completed);

    store
        .insert_torrent(
            "old-1",
            "0123456789abcdef0123456789abcdef01234567",
            "magnet:?",
            false,
            1,
        )
        .unwrap();
    let torrent = store.get_torrent("old-1").unwrap().expect("torrent exists");
    assert_eq!(
        torrent.info_hash,
        "0123456789abcdef0123456789abcdef01234567"
    );

    store
        .insert_torrent_file("old-1", 0, "file.txt", 100, true)
        .unwrap();
    let files = store.torrent_files("old-1").unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "file.txt");
}

#[test]
fn a_secret_never_appears_in_the_database_file() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("segmenta.db");
    let store = Store::open(&db_path).unwrap();
    store
        .insert_download(&record("t", DownloadStatus::Downloading))
        .unwrap();

    let secret = "Bearer super-secret-token-value";
    let secrets = FileSecretStore::open(&dir.path().join("secrets.json")).unwrap();
    secrets.store("segmenta/t", secret).unwrap();
    store.set_secret_alias("t", "segmenta/t").unwrap();
    assert_eq!(
        store.secret_alias("t").unwrap().as_deref(),
        Some("segmenta/t")
    );
    // A different task must not inherit the alias.
    assert_eq!(store.secret_alias("other").unwrap(), None);
    drop(store);

    // Force a checkpoint so the WAL contents land in the main file as well.
    let store = Store::open(&db_path).unwrap();
    store
        .save_checkpoint(
            "t",
            &Checkpoint {
                segments: &[SegmentRow {
                    id: 1,
                    start: 0,
                    end: 1023,
                    next: 10,
                }],
                downloaded_bytes: 10,
                total_bytes: None,
                etag: None,
                last_modified: None,
                supports_ranges: true,
            },
        )
        .unwrap();
    store.journal_mode().unwrap();
    drop(store);

    // Only the SQLite files are scanned: the fallback secret file is *supposed* to hold
    // the secret, with 0600 permissions, and is asserted separately.
    let mut haystack = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let path = dir.path().join(format!("segmenta.db{suffix}"));
        if let Ok(bytes) = std::fs::read(&path) {
            haystack.extend(bytes);
        }
    }
    let needle = secret.as_bytes();
    assert!(
        !haystack
            .windows(needle.len())
            .any(|window| window == needle),
        "the secret must never be stored in SQLite or its WAL"
    );
}

#[test]
fn the_fallback_secret_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secrets.json");
    let store = FileSecretStore::open(&path).unwrap();
    store.store("alias", "value").unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(store.backend(), "file");
    assert_eq!(store.lookup("alias").unwrap().as_deref(), Some("value"));
    store.delete("alias").unwrap();
    assert_eq!(store.lookup("alias").unwrap(), None);
}

#[test]
fn headers_are_stored_as_json_and_never_duplicated() {
    let (_dir, store) = open_temp();
    store
        .insert_download(&record("t", DownloadStatus::Pending))
        .unwrap();
    let mut headers = HashMap::new();
    headers.insert("Referer".to_string(), "https://example.test/".to_string());
    let owned: BTreeMap<String, String> = headers.into_iter().collect();
    store.set_headers("t", &owned).unwrap();
    store.set_headers("t", &owned).unwrap();
    assert_eq!(store.headers("t").unwrap(), owned);
}

#[test]
fn settings_validation_accepts_the_documented_edges() {
    let settings = Settings {
        max_connections: 32,
        max_retries: 0,
        schedule_start: Some("00:00".to_string()),
        proxy_mode: "manual".to_string(),
        proxy_url: Some("socks5://127.0.0.1:1080".to_string()),
        ..Settings::default()
    };
    assert!(settings.validate().is_ok());

    let bad_appearance = Settings {
        appearance: "sepia".to_string(),
        ..Settings::default()
    };
    assert!(bad_appearance.validate().is_err());

    let bad = Settings {
        proxy_mode: "manual".to_string(),
        ..Settings::default()
    };
    assert!(bad.validate().is_err(), "manual proxy needs a URL");
}

#[test]
fn two_downloads_can_share_engine_segment_ids() {
    // Regression: `segments.id` is the table rowid, so two tasks whose engine both
    // numbers its segments from 0 must not collide.
    let (_dir, store) = open_temp();
    for id in ["first", "second"] {
        store
            .insert_download(&record(id, DownloadStatus::Downloading))
            .unwrap();
        let segments = vec![
            SegmentRow {
                id: 0,
                start: 0,
                end: 499,
                next: 100,
            },
            SegmentRow {
                id: 1,
                start: 500,
                end: 999,
                next: 500,
            },
        ];
        store
            .save_checkpoint(
                id,
                &Checkpoint {
                    segments: &segments,
                    downloaded_bytes: 600,
                    total_bytes: Some(1000),
                    etag: None,
                    last_modified: None,
                    supports_ranges: true,
                },
            )
            .expect("a second download must be able to checkpoint its own segments");
    }
    assert_eq!(store.segments("first").unwrap().len(), 2);
    assert_eq!(store.segments("second").unwrap().len(), 2);
}

/// The extension list the previous release stored as its own default.
const PREVIOUS_DEFAULT_CAPTURE_EXTENSIONS: &[&str] = &[
    "3gp", "7z", "aac", "ace", "aif", "apk", "arj", "asf", "avi", "bin", "bz2", "exe", "gz",
    "gzip", "img", "iso", "lzh", "m4a", "m4v", "mkv", "mov", "mp3", "mp4", "mpa", "mpe", "mpeg",
    "mpg", "msi", "msu", "ogg", "ogv", "pdf", "plj", "pps", "ppt", "qt", "r0*", "r1*", "ra", "rar",
    "rm", "rmvb", "sea", "sit", "sitx", "tar", "tif", "tiff", "wav", "wma", "wmv", "z", "zip",
    "pkg",
];

fn write_raw_setting(path: &std::path::Path, key: &str, value: serde_json::Value) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute(
        "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
        rusqlite::params![key, value.to_string()],
    )
    .unwrap();
}

/// S11: a settings row written before `capture_confirm` existed still loads, keeps its
/// values and gains the new default plus the widened extension list.
#[test]
fn a_settings_row_written_before_the_new_keys_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("segmenta.db");
    {
        let store = Store::open(&path).unwrap();
        store
            .update_settings(
                json!({ "max_connections": 8, "download_dir": "/mnt/descargas", "capture_enabled": false })
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
    }
    // The exact rows 0.1.1 wrote: its default token list and no `capture_confirm` row.
    write_raw_setting(
        &path,
        "capture_extensions",
        json!(PREVIOUS_DEFAULT_CAPTURE_EXTENSIONS),
    );

    let store = Store::open(&path).unwrap();
    let settings = store.settings().unwrap();
    assert!(settings.capture_confirm, "the new key takes its default");
    assert_eq!(
        settings.capture_extensions,
        Settings::default().capture_extensions,
        "the old default list is upgraded in memory"
    );
    assert!(settings
        .capture_extensions
        .contains(&"appimage".to_string()));
    // Stored values are preserved.
    assert_eq!(settings.max_connections, 8);
    assert_eq!(settings.download_dir, "/mnt/descargas");
    assert!(!settings.capture_enabled);

    // A list the user customized is never rewritten.
    let other = tempfile::tempdir().unwrap();
    let other_path = other.path().join("segmenta.db");
    {
        let store = Store::open(&other_path).unwrap();
        store
            .update_settings(
                json!({ "capture_extensions": ["zip", "iso"] })
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
    }
    let store = Store::open(&other_path).unwrap();
    assert_eq!(
        store.settings().unwrap().capture_extensions,
        vec!["zip".to_string(), "iso".to_string()]
    );
}

/// S11: the catalog defaults carry `capture_confirm` next to the other capture keys.
#[test]
fn capture_confirm_defaults_to_true_in_the_catalog() {
    let (_dir, store) = open_temp();
    let settings = store.settings().unwrap();
    assert!(settings.capture_confirm);
    for token in [
        "deb", "rpm", "appimage", "xz", "whl", "jar", "epub", "vsix", "xpi", "msix",
    ] {
        assert!(
            settings.capture_extensions.contains(&token.to_string()),
            "missing {token}"
        );
    }
}

/// The daemon default and the shared capture list must stay the same tokens in the
/// same order, or the UI would promise a format the daemon never captures.
#[test]
fn the_capture_default_is_the_shared_core_list() {
    let expected: Vec<String> = segmenta_core::rules::CAPTURE_EXTENSIONS
        .iter()
        .map(|token| token.to_string())
        .collect();
    assert_eq!(Settings::default().capture_extensions, expected);
}
