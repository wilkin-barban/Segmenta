//! Pure model behaviour: wire parsing, row updates, filtering and formatting.
//!
//! These tests must pass without any display and without a daemon.

use serde_json::json;

use segmenta_gui::model::{
    format_bytes, format_eta, format_speed, Counts, DownloadItem, DownloadStatus, DownloadStore,
    Filter, ProgressUpdate, SegmentItem, SeqStatus, Settings,
};
use segmenta_i18n::Locale;

fn row(task_id: &str, status: &str) -> DownloadItem {
    serde_json::from_value(json!({
        "task_id": task_id,
        "url": "https://releases.ubuntu.com/26.04/ubuntu-26.04-desktop-amd64.iso",
        "final_path": "/home/user/Descargas/ubuntu-26.04-desktop-amd64.iso",
        "filename": "ubuntu-26.04-desktop-amd64.iso",
        "status": status,
        "total_bytes": 4_000_000_000u64,
        "downloaded_bytes": 1_200_000_000u64,
        "speed_bps": 45_200_000u64,
        "eta_seconds": 60u64,
        "retry_count": 0u32,
        "supports_ranges": true,
        "queue_name": "main",
        "created_at": "2026-04-24T10:00:00Z",
        "completed_at": null,
        "error_code": null,
        "error_message": null,
        "segments": []
    }))
    .expect("contract row parses")
}

fn segment(id: u32, start: u64, end: u64, next: u64) -> SegmentItem {
    serde_json::from_value(json!({
        "id": id,
        "start_offset": start,
        "end_offset": end,
        "next_offset": next,
        "status": if next > end { "completed" } else { "active" },
        "speed_bps": 0
    }))
    .expect("contract segment parses")
}

#[test]
fn wire_row_parses_every_contract_field() {
    let item = row("task-1", "downloading");
    assert_eq!(item.task_id, "task-1");
    assert_eq!(item.status, DownloadStatus::Downloading);
    assert_eq!(item.filename, "ubuntu-26.04-desktop-amd64.iso");
    assert_eq!(item.total_bytes, Some(4_000_000_000));
    assert_eq!(item.downloaded_bytes, 1_200_000_000);
    assert_eq!(item.speed_bps, 45_200_000);
    assert_eq!(item.eta_seconds, Some(60));
    assert_eq!(item.retry_count, 0);
    assert!(item.supports_ranges);
    assert_eq!(item.queue_name, "main");
    assert_eq!(item.created_at.as_deref(), Some("2026-04-24T10:00:00Z"));
    assert_eq!(item.completed_at, None);
    assert_eq!(item.error_code, None);
    assert_eq!(item.error_message, None);
    assert!(item.segments.is_empty());
    // 1.2 GB of 4.0 GB.
    assert!((item.progress() - 0.3).abs() < 1e-9, "{}", item.progress());
}

#[test]
fn nullable_wire_fields_are_tolerated() {
    let item: DownloadItem = serde_json::from_value(json!({
        "task_id": "t",
        "url": "https://example.com/x",
        "final_path": "",
        "filename": "x",
        "status": "pending",
        "total_bytes": null,
        "downloaded_bytes": 0u64,
        "speed_bps": 0u64,
        "eta_seconds": null,
        "retry_count": 0u32,
        "supports_ranges": false,
        "queue_name": "main",
        "created_at": null,
        "completed_at": null,
        "error_code": null,
        "error_message": null
    }))
    .expect("minimal row parses");
    assert_eq!(item.total_bytes, None);
    assert_eq!(item.progress(), 0.0);
    assert_eq!(item.status, DownloadStatus::Pending);
}

#[test]
fn segments_accept_both_id_shapes() {
    let listed = segment(3, 100, 199, 150);
    let pushed: SegmentItem = serde_json::from_value(json!({
        "segment_id": 3,
        "start_offset": 100,
        "end_offset": 199,
        "next_offset": 150,
        "status": "active",
        "speed_bps": 1024
    }))
    .expect("task_progress segment parses");
    assert_eq!(listed.id, pushed.id);
    assert_eq!(listed.span(), 100);
    assert_eq!(listed.downloaded(), 50);
    assert!(!listed.is_complete());
    assert!(listed.is_active());
}

#[test]
fn reload_replaces_rows_without_duplicates() {
    let mut store = DownloadStore::default();
    store.reload(vec![row("a", "downloading"), row("b", "completed")]);
    assert_eq!(store.len(), 2);
    store.reload(vec![row("a", "paused")]);
    assert_eq!(store.len(), 1);
    assert_eq!(
        store.get("a").map(|item| item.status),
        Some(DownloadStatus::Paused)
    );
}

#[test]
fn task_progress_updates_bytes_speed_eta_and_segments() {
    let mut store = DownloadStore::default();
    store.reload(vec![row("a", "downloading")]);
    let progress: ProgressUpdate = serde_json::from_value(json!({
        "downloaded_bytes": 2_000_000_000u64,
        "total_bytes": 4_000_000_000u64,
        "speed_bps": 80_000_000u64,
        "eta_seconds": 25u64,
        "segments": [
            {"segment_id": 0, "start_offset": 0u64, "end_offset": 999_999_999u64, "next_offset": 500_000_000u64, "status": "active", "speed_bps": 0u64},
            {"segment_id": 1, "start_offset": 1_000_000_000u64, "end_offset": 1_999_999_999u64, "next_offset": 1_500_000_000u64, "status": "active", "speed_bps": 0u64},
            {"segment_id": 2, "start_offset": 2_000_000_000u64, "end_offset": 2_999_999_999u64, "next_offset": 2_500_000_000u64, "status": "active", "speed_bps": 0u64},
            {"segment_id": 3, "start_offset": 3_000_000_000u64, "end_offset": 3_999_999_999u64, "next_offset": 3_100_000_000u64, "status": "active", "speed_bps": 0u64}
        ]
    }))
    .expect("progress payload parses");

    assert!(store.apply_progress("a", &progress));
    let updated = store.get("a").expect("row kept");
    assert_eq!(updated.downloaded_bytes, 2_000_000_000);
    assert_eq!(updated.speed_bps, 80_000_000);
    assert_eq!(updated.eta_seconds, Some(25));
    assert_eq!(updated.segments.len(), 4);
    assert_eq!(updated.active_segments(), 4);
    assert!((updated.progress() - 0.5).abs() < 1e-9);

    // A progress notification never invents a row.
    assert!(!store.apply_progress("does-not-exist", &progress));
    assert_eq!(store.len(), 1);
}

#[test]
fn state_change_and_removal_follow_events() {
    let mut store = DownloadStore::default();
    store.reload(vec![row("a", "downloading")]);
    assert!(store.apply_state("a", DownloadStatus::Completed));
    assert_eq!(
        store.get("a").map(|item| item.status),
        Some(DownloadStatus::Completed)
    );
    assert!(store.get("a").map(|item| item.speed_bps) == Some(0));
    assert!(store.apply_error("a", 503, "server exploded"));
    assert_eq!(
        store
            .get("a")
            .and_then(|item| item.error_message.as_deref()),
        Some("server exploded")
    );
    assert!(store.remove("a"));
    assert_eq!(store.len(), 0);
}

#[test]
fn seq_gaps_and_duplicates_are_detected() {
    let mut store = DownloadStore::default();
    assert_eq!(store.observe_seq(1), SeqStatus::InOrder);
    assert_eq!(store.observe_seq(2), SeqStatus::InOrder);
    assert_eq!(store.observe_seq(5), SeqStatus::Gap);
    assert_eq!(store.observe_seq(5), SeqStatus::Duplicate);
    assert_eq!(store.observe_seq(3), SeqStatus::Duplicate);
    assert_eq!(store.observe_seq(6), SeqStatus::InOrder);
}

#[test]
fn counts_and_filters_feed_the_sidebar() {
    let mut store = DownloadStore::default();
    store.reload(vec![
        row("a", "downloading"),
        row("b", "downloading"),
        row("c", "paused"),
        row("d", "completed"),
        row("e", "failed"),
        row("f", "cancelled"),
        row("g", "pending"),
    ]);
    assert_eq!(
        store.counts(),
        Counts {
            all: 7,
            active: 2,
            paused: 1,
            completed: 1,
            failed: 1
        }
    );
    assert_eq!(store.filtered(Filter::Active, "").len(), 2);
    assert_eq!(store.filtered(Filter::Paused, "").len(), 1);
    assert_eq!(store.filtered(Filter::Completed, "").len(), 1);
    assert_eq!(store.filtered(Filter::Failed, "").len(), 1);
    assert_eq!(store.filtered(Filter::All, "").len(), 7);
    // Search is case insensitive over filename and URL.
    assert_eq!(store.filtered(Filter::All, "UBUNTU-26.04").len(), 7);
    assert_eq!(store.filtered(Filter::All, "releases.ubuntu.com").len(), 7);
    assert_eq!(store.filtered(Filter::All, "no-such-file").len(), 0);
}

#[test]
fn destructive_actions_are_gated_by_state() {
    let downloading = row("a", "downloading");
    assert!(downloading.status.can_pause());
    assert!(!downloading.status.can_resume());
    assert!(downloading.status.can_cancel());
    assert!(!downloading.status.can_delete());

    let paused = row("a", "paused");
    assert!(!paused.status.can_pause());
    assert!(paused.status.can_resume());
    assert!(paused.status.can_cancel());
    assert!(!paused.status.can_delete());

    let completed = row("a", "completed");
    assert!(!completed.status.can_pause());
    assert!(!completed.status.can_resume());
    assert!(!completed.status.can_cancel());
    assert!(completed.status.can_delete());

    let failed = row("a", "failed");
    assert!(failed.status.can_resume());
    assert!(failed.status.can_delete());

    let cancelled = row("a", "cancelled");
    assert!(!cancelled.status.can_cancel());
    assert!(cancelled.status.can_delete());

    let pending = row("a", "pending");
    assert!(!pending.status.can_pause());
    assert!(pending.status.can_cancel());
}

#[test]
fn spanish_units_and_labels() {
    assert_eq!(format_bytes(0), "0 B");
    assert_eq!(format_bytes(512), "512 B");
    assert_eq!(format_bytes(45_200_000), "45.2 MB");
    assert_eq!(format_bytes(4_000_000_000), "4.0 GB");
    assert_eq!(format_speed(45_200_000), "45.2 MB/s");
    assert_eq!(format_speed(0), "—");
    assert_eq!(format_eta(None), "—");
    assert_eq!(format_eta(Some(0)), "—");
    // `format_eta` returns the duration only; the label is localized by the caller.
    assert_eq!(format_eta(Some(45)), "45 s");
    assert_eq!(format_eta(Some(90)), "1 min");
    assert_eq!(format_eta(Some(3700)), "1 h 1 min");
    assert_eq!(DownloadStatus::Downloading.label(Locale::Es), "Descargando");
    assert_eq!(DownloadStatus::Paused.label(Locale::Es), "Pausada");
    assert_eq!(DownloadStatus::Completed.label(Locale::Es), "Completada");
    assert_eq!(DownloadStatus::Failed.label(Locale::Es), "Fallida");
    assert_eq!(DownloadStatus::Pending.label(Locale::Es), "En espera");
    // The progress line must not keep Spanish words in other locales.
    let mut item = row("eta", "downloading");
    item.eta_seconds = Some(53);
    let line = item.progress_line();
    assert!(!line.contains("Quedan"), "{line}");
    assert!(
        line.contains("remaining") || line.contains("restante"),
        "{line}"
    );
    assert_eq!(Filter::All.label(Locale::Es), "Todas");
    assert_eq!(Filter::Active.label(Locale::Es), "Activas");
}

#[test]
fn settings_round_trip_and_speed_limit_conversion() {
    let settings: Settings = serde_json::from_value(json!({
        "max_connections": 16,
        "global_speed_limit_bps": null,
        "clip_monitor_enabled": true,
        "autostart": false,
        "download_dir": "/home/user/Descargas",
        "category_dirs": {"Vídeos": "/home/user/Vídeos"},
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
    }))
    .expect("settings payload parses");
    assert_eq!(settings.max_connections, 16);
    assert_eq!(settings.global_speed_limit_bps, None);
    assert_eq!(
        settings.category_dirs.get("Vídeos").map(String::as_str),
        Some("/home/user/Vídeos")
    );
    // 0 means unlimited in the UI.
    assert_eq!(settings.speed_limit_kib(), 0);
    let limited = Settings {
        global_speed_limit_bps: Some(5 * 1024 * 1024),
        ..settings.clone()
    };
    assert_eq!(limited.speed_limit_kib(), 5120);
    // Files are not downloaded from loopback by default in this release either.
    assert!(!settings.allow_local_downloads);
}
