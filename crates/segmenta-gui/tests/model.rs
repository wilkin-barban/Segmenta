//! Pure model behaviour: wire parsing, row updates, filtering and formatting.
//!
//! These tests must pass without any display and without a daemon.

use serde_json::json;

use segmenta_gui::model::{
    format_bytes, format_eta, format_speed, BatchDeleteReport, CleanCompletedReport, Counts,
    DownloadItem, DownloadStatus, DownloadStore, Filter, ProgressUpdate, SegmentItem, SeqStatus,
    Settings,
};
use segmenta_i18n::{tr, Locale, Msg};

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

fn created_row(task_id: &str, status: &str, created_at: &str) -> DownloadItem {
    let mut item = row(task_id, status);
    item.created_at = Some(created_at.to_string());
    item
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
fn a_new_task_lands_at_the_top_of_the_list() {
    let mut store = DownloadStore::default();
    store.reload(vec![row("old", "completed")]);
    store.upsert(row("fresh", "downloading"));
    let order: Vec<&str> = store
        .items()
        .iter()
        .map(|item| item.task_id.as_str())
        .collect();
    assert_eq!(order, vec!["fresh", "old"]);
    // An update in place must not move the row.
    store.upsert(row("old", "paused"));
    let order: Vec<&str> = store
        .items()
        .iter()
        .map(|item| item.task_id.as_str())
        .collect();
    assert_eq!(order, vec!["fresh", "old"]);
}

#[test]
fn reload_keeps_the_daemon_newest_first_order() {
    let mut store = DownloadStore::default();
    // Exactly what `download.list` sends (`created_at DESC, rowid DESC`): newest on top.
    store.reload(vec![
        created_row("newest", "completed", "2026-04-24T12:00:00Z"),
        created_row("middle", "downloading", "2026-04-24T11:00:00Z"),
        created_row("oldest", "failed", "2026-04-24T10:00:00Z"),
    ]);
    let order: Vec<&str> = store
        .items()
        .iter()
        .map(|item| item.task_id.as_str())
        .collect();
    assert_eq!(
        order,
        vec!["newest", "middle", "oldest"],
        "the reload reversed the daemon order"
    );
    assert_eq!(store.len(), 3);

    // A task added from an event still lands on top of the same order.
    store.upsert(created_row("fresh", "downloading", "2026-04-24T13:00:00Z"));
    let order: Vec<&str> = store
        .items()
        .iter()
        .map(|item| item.task_id.as_str())
        .collect();
    assert_eq!(order, vec!["fresh", "newest", "middle", "oldest"]);

    // The next reload receives that same order and must keep it, without duplicates.
    store.reload(vec![
        created_row("fresh", "downloading", "2026-04-24T13:00:00Z"),
        created_row("newest", "completed", "2026-04-24T12:00:00Z"),
        created_row("middle", "downloading", "2026-04-24T11:00:00Z"),
        created_row("oldest", "failed", "2026-04-24T10:00:00Z"),
        // A repeated id must replace in place, never paint two rows.
        created_row("middle", "paused", "2026-04-24T11:00:00Z"),
    ]);
    let order: Vec<&str> = store
        .items()
        .iter()
        .map(|item| item.task_id.as_str())
        .collect();
    assert_eq!(order, vec!["fresh", "newest", "middle", "oldest"]);
    assert_eq!(
        store.get("middle").map(|item| item.status),
        Some(DownloadStatus::Paused),
        "the later duplicate payload must win"
    );
}

#[test]
fn a_stale_status_answer_never_moves_the_bar_back() {
    let mut store = DownloadStore::default();
    store.reload(vec![row("a", "downloading")]);
    // A `download.status` is sent now; while it is in flight a progress event lands.
    let request_mark = store.task_mark("a");
    let progress: ProgressUpdate = serde_json::from_value(json!({
        "downloaded_bytes": 3_000_000_000u64,
        "total_bytes": 4_000_000_000u64,
        "speed_bps": 30_000_000u64
    }))
    .expect("progress payload parses");
    assert!(store.apply_progress("a", &progress));

    // The answer carries the older bytes and the older status.
    let mut stale = row("a", "downloading");
    stale.downloaded_bytes = 1_200_000_000;
    store.apply_status(stale, request_mark);
    let item = store.get("a").expect("row kept");
    assert_eq!(item.downloaded_bytes, 3_000_000_000, "the bar moved back");
    assert_eq!(item.speed_bps, 30_000_000);
    assert!((item.progress() - 0.75).abs() < 1e-9);

    // An answer requested after the event is the authority: the task completed.
    let fresh_mark = store.task_mark("a");
    let mut fresh = row("a", "completed");
    fresh.downloaded_bytes = 4_000_000_000;
    store.apply_status(fresh, fresh_mark);
    let item = store.get("a").expect("row kept");
    assert_eq!(item.status, DownloadStatus::Completed);
    assert_eq!(item.downloaded_bytes, 4_000_000_000);
    assert!((item.progress() - 1.0).abs() < 1e-9);

    // A task the list does not know yet still lands on top, never dropped.
    let mut unknown = row("late", "downloading");
    unknown.downloaded_bytes = 10;
    store.apply_status(unknown, 0);
    assert_eq!(store.len(), 2);
    assert_eq!(
        store.items().first().map(|item| item.task_id.as_str()),
        Some("late")
    );
}

#[test]
fn an_unknown_total_never_fakes_a_fraction_or_a_size() {
    let mut store = DownloadStore::default();
    // HLS/DASH streams publish `total_bytes: null` (`manager.rs`, the stream path).
    let mut stream = row("stream", "downloading");
    stream.total_bytes = None;
    stream.downloaded_bytes = 5_000_000;
    // A torrent reports `0` until the metadata arrives.
    let mut torrent = row("torrent", "downloading");
    torrent.total_bytes = Some(0);
    torrent.downloaded_bytes = 12_000_000;
    store.reload(vec![stream, torrent]);

    for task_id in ["stream", "torrent"] {
        let item = store.get(task_id).expect("row listed");
        assert!(!item.has_known_total(), "{task_id} claims a known size");
        assert_eq!(item.progress(), 0.0);
        // Only the downloaded amount: never `0 B de 0 B`.
        assert!(!item.size_line().contains(" de "), "{}", item.size_line());
    }

    // A `task_progress` that brings no total keeps the one the list already had.
    let mut known = row("known", "downloading");
    known.total_bytes = Some(4_000_000_000);
    store.reload(vec![known]);
    let progress: ProgressUpdate = serde_json::from_value(json!({
        "downloaded_bytes": 2_000_000_000u64,
        "segments": []
    }))
    .expect("progress payload parses");
    assert!(store.apply_progress("known", &progress));
    let item = store.get("known").expect("row kept");
    assert!(item.has_known_total());
    assert_eq!(item.total_bytes, Some(4_000_000_000));
    assert!((item.progress() - 0.5).abs() < 1e-9);
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
fn retrying_and_finalizing_are_real_states_not_unknown() {
    // Core stores `retrying` and `finalizing` (segmenta-daemon store.rs); the GUI
    // must not fall back to `Unknown` for either of them.
    let retrying = row("a", "retrying");
    assert_ne!(retrying.status, DownloadStatus::Unknown);
    assert_eq!(retrying.status, DownloadStatus::Retrying);
    assert_eq!(retrying.status.label(Locale::Es), "Reintentando");
    assert_eq!(retrying.status.label(Locale::En), "Retrying");
    // Waiting for the next retry: in flight, so it counts as active and the daemon
    // accepts a pause (manager.rs::pause_task) and a resume (manager.rs::resume_task).
    assert!(retrying.status.is_active());
    assert!(retrying.status.can_pause());
    assert!(retrying.status.can_resume());
    assert!(retrying.status.can_cancel());
    assert!(!retrying.status.can_delete());

    let finalizing = row("a", "finalizing");
    assert_ne!(finalizing.status, DownloadStatus::Unknown);
    assert_eq!(finalizing.status, DownloadStatus::Finalizing);
    assert_eq!(finalizing.status.label(Locale::Es), "Finalizando");
    assert_eq!(finalizing.status.label(Locale::En), "Finalizing");
    // Flushing and publishing: still work in flight, so pausable and cancellable,
    // but there is nothing left to retry from.
    assert!(finalizing.status.is_active());
    assert!(finalizing.status.can_pause());
    assert!(!finalizing.status.can_resume());
    assert!(finalizing.status.can_cancel());
    assert!(!finalizing.status.can_delete());
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

// Safe download management (0.2.1): the daemon payloads that drive the clean and
// batch-delete dialogs are parsed here, without a display and without a daemon.

#[test]
fn clean_completed_answer_parses_the_contract_shape() {
    let report: CleanCompletedReport = serde_json::from_value(json!({
        "removed": 3,
        "failed": [{ "task_id": "b", "error": "database is locked" }]
    }))
    .expect("clean_completed payload parses");
    assert_eq!(report.removed, 3);
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].task_id, "b");
    // The payload has no file count at all: cleaning never deletes files.
    let text = report.summary(Locale::Es);
    assert!(text.contains('3'), "{text}");
    assert!(text.contains('b'), "{text}");
    assert!(text.contains("database is locked"), "{text}");
}

#[test]
fn batch_delete_answer_names_every_failure() {
    let report: BatchDeleteReport = serde_json::from_value(json!({
        "results": [
            { "task_id": "a", "ok": true },
            { "task_id": "b", "ok": false, "error": "no such task" }
        ],
        "removed": 1,
        "files_deleted": 1,
        "files_failed": [
            { "task_id": "a", "path": "/home/user/Descargas/a.iso", "error": "permission denied" }
        ]
    }))
    .expect("delete_many payload parses");
    assert_eq!(report.removed, 1);
    assert_eq!(report.files_deleted, 1);
    assert!(!report.is_full_success(), "a failed file is not success");
    assert_eq!(report.failed_tasks().len(), 1);
    let text = report.summary(Locale::En);
    assert!(
        text.contains('b') && text.contains("no such task"),
        "{text}"
    );
    assert!(
        text.contains("/home/user/Descargas/a.iso") && text.contains("permission denied"),
        "{text}"
    );

    let clean: BatchDeleteReport = serde_json::from_value(
        json!({ "results": [{ "task_id": "a", "ok": true }], "removed": 1 }),
    )
    .expect("minimal payload parses");
    assert_eq!(clean.files_deleted, 0);
    assert!(clean.is_full_success());
}

#[test]
fn a_selection_summary_counts_files_and_active_tasks() {
    let mut store = DownloadStore::default();
    store.reload(vec![
        row("done", "completed"),
        row("run", "downloading"),
        row("nofile", "failed"),
    ]);
    // A task without a path has nothing to delete.
    let mut no_file = store.get("nofile").cloned().expect("row");
    no_file.final_path = String::new();
    store.upsert(no_file);

    let ids = vec![
        "done".to_string(),
        "run".to_string(),
        "nofile".to_string(),
        "ghost".to_string(),
    ];
    let summary = store.selection_summary(&ids);
    assert_eq!(summary.tasks, 3, "unknown ids are not counted");
    assert_eq!(summary.files, 2);
    assert_eq!(summary.active, 1);
    let body = summary.body(Locale::Es);
    assert!(body.contains('3') && body.contains('2'), "{body}");
    assert!(body.contains(&tr(Locale::Es, Msg::DeleteSelectionActive).replace("{count}", "1")));
}

#[test]
fn remove_many_removes_only_the_named_rows() {
    let mut store = DownloadStore::default();
    store.reload(vec![
        row("a", "completed"),
        row("b", "completed"),
        row("c", "failed"),
    ]);
    let removed = store.remove_many(&["a".to_string(), "ghost".to_string()]);
    assert_eq!(removed, 1, "an unknown id removes nothing");
    assert_eq!(store.len(), 2);
    assert!(store.get("a").is_none());
    assert!(store.get("c").is_some());
    assert_eq!(store.completed_ids(), vec!["b".to_string()]);
}

/// `download.list` answers with `segments: []`; a stale status answer must not wipe the
/// segments a fresher event already painted. The smoke gate caught this as `segments=0`.
#[test]
fn a_stale_status_answer_keeps_the_segments_of_a_fresher_event() {
    let mut store = DownloadStore::default();
    store.reload(vec![row("task", "downloading")]);
    // A `download.status` goes out now; a progress event with four segments lands while
    // it is in flight.
    let request_mark = store.task_mark("task");
    let progress: ProgressUpdate = serde_json::from_value(json!({
        "downloaded_bytes": 1_000u64,
        "total_bytes": 4_096u64,
        "speed_bps": 0u64,
        "segments": [
            {"segment_id": 0, "start_offset": 0, "end_offset": 1023, "next_offset": 512, "status": "active", "speed_bps": 0},
            {"segment_id": 1, "start_offset": 1024, "end_offset": 2047, "next_offset": 1024, "status": "active", "speed_bps": 0},
            {"segment_id": 2, "start_offset": 2048, "end_offset": 3071, "next_offset": 2048, "status": "active", "speed_bps": 0},
            {"segment_id": 3, "start_offset": 3072, "end_offset": 4095, "next_offset": 3072, "status": "active", "speed_bps": 0}
        ]
    }))
    .expect("progress payload parses");
    assert!(store.apply_progress("task", &progress));
    assert_eq!(
        store.get("task").map(|item| item.segments.len()),
        Some(4),
        "the live event painted four segments"
    );

    let mut stale = row("task", "downloading");
    stale.segments = Vec::new();
    store.apply_status(stale, request_mark);
    assert_eq!(
        store.get("task").map(|item| item.segments.len()),
        Some(4),
        "the stale answer wiped the segments and the smoke gate would see 0"
    );
}
