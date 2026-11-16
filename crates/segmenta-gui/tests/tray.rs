//! Tests for the pure tray menu model (Part A).
//!
//! No display or D-Bus connection is needed because the model is pure data.

use segmenta_gui::model::{DownloadItem, DownloadStatus};
use segmenta_gui::tray::{
    format_tray_progress, format_tray_speed_eta, select_active_task, TrayAction, TrayMenuItem,
    TrayModel,
};
use segmenta_i18n::{tr, Locale, Msg};

#[test]
fn tray_menu_idle_state_when_no_active_downloads() {
    let locale = Locale::Es;
    let model = TrayModel::build(None, 0, locale);

    // Header says the idle text
    assert_eq!(
        model.items.first(),
        Some(&TrayMenuItem::Header(tr(locale, Msg::TrayIdle).to_string()))
    );

    // Only Abrir, Preferencias, Separator, and Salir are present
    let actions: Vec<TrayAction> = model.items.iter().filter_map(|it| it.action()).collect();
    assert_eq!(
        actions,
        vec![TrayAction::Open, TrayAction::Preferences, TrayAction::Quit]
    );

    // Verify labels
    assert_eq!(
        model.items[1],
        TrayMenuItem::Action(
            TrayAction::Open,
            tr(locale, Msg::TrayOpenSegmenta).to_string()
        )
    );
    assert_eq!(
        model.items[2],
        TrayMenuItem::Action(
            TrayAction::Preferences,
            tr(locale, Msg::Preferences).to_string()
        )
    );
    assert!(model.items[3].is_separator());
    assert_eq!(
        model.items[4],
        TrayMenuItem::Action(TrayAction::Quit, tr(locale, Msg::TrayQuit).to_string())
    );
    assert_eq!(model.items.len(), 5);
}

#[test]
fn tray_menu_one_downloading_task_with_speed_and_eta() {
    let locale = Locale::Es;
    let task = DownloadItem {
        task_id: "task-1".to_string(),
        filename: "ubuntu-26.04-desktop-amd64.iso".to_string(),
        status: DownloadStatus::Downloading,
        total_bytes: Some(4_000_000_000),
        downloaded_bytes: 1_000_000_000,
        speed_bps: 45_200_000,
        eta_seconds: Some(60),
        ..Default::default()
    };

    let model = TrayModel::build(Some(&task), 1, locale);

    // Row 1: Header row with the file name
    assert_eq!(
        model.items[0],
        TrayMenuItem::Header("ubuntu-26.04-desktop-amd64.iso".to_string())
    );

    // Row 2: Percentage and downloaded/total
    let progress_label = model.items[1].label();
    assert!(
        progress_label.contains("25 %"),
        "expected 25 %, got: {progress_label}"
    );
    assert!(
        progress_label.contains("1.0 GB"),
        "expected downloaded size, got: {progress_label}"
    );
    assert!(
        progress_label.contains("4.0 GB"),
        "expected total size, got: {progress_label}"
    );

    // Row 3: Speed and ETA (reuses crate::model::format_speed)
    let speed_eta_label = model.items[2].label();
    assert!(
        speed_eta_label.contains("45.2 MB/s"),
        "expected formatted speed, got: {speed_eta_label}"
    );
    assert!(
        speed_eta_label.contains("1 min"),
        "expected formatted eta, got: {speed_eta_label}"
    );

    // Actions that apply to downloading state:
    // Abrir Segmenta always, Pausar when downloading, Cancelar when active, Preferencias, Separator, Salir
    let actions: Vec<TrayAction> = model.items.iter().filter_map(|it| it.action()).collect();
    assert_eq!(
        actions,
        vec![
            TrayAction::Open,
            TrayAction::Pause,
            TrayAction::Cancel,
            TrayAction::Preferences,
            TrayAction::Quit,
        ]
    );

    assert_eq!(
        model.items[3],
        TrayMenuItem::Action(
            TrayAction::Open,
            tr(locale, Msg::TrayOpenSegmenta).to_string()
        )
    );
    assert_eq!(
        model.items[4],
        TrayMenuItem::Action(TrayAction::Pause, tr(locale, Msg::Pause).to_string())
    );
    assert_eq!(
        model.items[5],
        TrayMenuItem::Action(TrayAction::Cancel, tr(locale, Msg::Cancel).to_string())
    );
    assert_eq!(
        model.items[6],
        TrayMenuItem::Action(
            TrayAction::Preferences,
            tr(locale, Msg::Preferences).to_string()
        )
    );
    assert!(model.items[7].is_separator());
    assert_eq!(
        model.items[8],
        TrayMenuItem::Action(TrayAction::Quit, tr(locale, Msg::TrayQuit).to_string())
    );
}

#[test]
fn tray_menu_paused_task_offers_reanudar_instead_of_pausar() {
    let locale = Locale::Es;
    let task = DownloadItem {
        task_id: "task-paused".to_string(),
        filename: "video.mp4".to_string(),
        status: DownloadStatus::Paused,
        total_bytes: Some(200_000_000),
        downloaded_bytes: 50_000_000,
        speed_bps: 0,
        eta_seconds: None,
        ..Default::default()
    };

    let model = TrayModel::build(Some(&task), 0, locale);

    // Actions must contain Reanudar, not Pausar
    let actions: Vec<TrayAction> = model.items.iter().filter_map(|it| it.action()).collect();
    assert_eq!(
        actions,
        vec![
            TrayAction::Open,
            TrayAction::Resume,
            TrayAction::Cancel,
            TrayAction::Preferences,
            TrayAction::Quit,
        ]
    );

    assert_eq!(
        model.items[4],
        TrayMenuItem::Action(TrayAction::Resume, tr(locale, Msg::Resume).to_string())
    );

    // Speed/ETA should show em dash
    let speed_eta = model.items[2].label();
    assert!(speed_eta.contains('—'));
}

#[test]
fn tray_menu_task_without_known_total_shows_indeterminate_percentage() {
    let locale = Locale::Es;
    let task = DownloadItem {
        task_id: "task-stream".to_string(),
        filename: "live-stream.ts".to_string(),
        status: DownloadStatus::Downloading,
        total_bytes: None, // Unknown total (stream / torrent before metadata)
        downloaded_bytes: 500_000_000,
        speed_bps: 12_000_000,
        eta_seconds: None,
        ..Default::default()
    };

    let model = TrayModel::build(Some(&task), 1, locale);

    let progress_label = model.items[1].label();
    // Must show indeterminate percentage (em dash) rather than a bogus 0 %
    assert!(
        progress_label.contains('—'),
        "expected indeterminate em dash, got: {progress_label}"
    );
    assert!(
        !progress_label.contains("0 %"),
        "bogus 0 % found in: {progress_label}"
    );
    assert!(
        progress_label.contains("500.0 MB"),
        "expected downloaded bytes, got: {progress_label}"
    );
}

#[test]
fn select_active_task_prefers_transferring_task_or_paused() {
    let task_downloading = DownloadItem {
        task_id: "task-dl".to_string(),
        filename: "file1.bin".to_string(),
        status: DownloadStatus::Downloading,
        ..Default::default()
    };
    let task_paused = DownloadItem {
        task_id: "task-p".to_string(),
        filename: "file2.bin".to_string(),
        status: DownloadStatus::Paused,
        ..Default::default()
    };
    let task_completed = DownloadItem {
        task_id: "task-c".to_string(),
        filename: "file3.bin".to_string(),
        status: DownloadStatus::Completed,
        ..Default::default()
    };

    // With both downloading and paused, selects the downloading task
    let tasks = vec![
        task_completed.clone(),
        task_downloading.clone(),
        task_paused.clone(),
    ];
    let (selected, count) = select_active_task(&tasks);
    assert_eq!(selected.map(|t| t.task_id.as_str()), Some("task-dl"));
    assert_eq!(count, 1);

    // With only paused, selects the paused task so it can be resumed
    let tasks_paused = vec![task_completed.clone(), task_paused.clone()];
    let (selected, count) = select_active_task(&tasks_paused);
    assert_eq!(selected.map(|t| t.task_id.as_str()), Some("task-p"));
    assert_eq!(count, 0);

    // With only completed, returns None
    let tasks_empty = vec![task_completed];
    let (selected, count) = select_active_task(&tasks_empty);
    assert_eq!(selected, None);
    assert_eq!(count, 0);
}

#[test]
fn tray_menu_zero_total_bytes_also_shows_indeterminate_percentage() {
    let locale = Locale::Es;
    let task = DownloadItem {
        task_id: "task-torrent".to_string(),
        filename: "archlinux.iso".to_string(),
        status: DownloadStatus::Downloading,
        total_bytes: Some(0), // 0 is also unknown before metadata arrives
        downloaded_bytes: 10_000_000,
        speed_bps: 1_000_000,
        eta_seconds: None,
        ..Default::default()
    };

    let model = TrayModel::build(Some(&task), 1, locale);
    let progress_label = model.items[1].label();
    assert!(
        progress_label.contains('—'),
        "expected indeterminate em dash, got: {progress_label}"
    );
    assert!(
        !progress_label.contains("0 %"),
        "bogus 0 % found in: {progress_label}"
    );
}

#[test]
fn tray_menu_localizes_in_en_es_and_pt() {
    let task = DownloadItem {
        task_id: "task-loc".to_string(),
        filename: "data.zip".to_string(),
        status: DownloadStatus::Downloading,
        total_bytes: Some(100),
        downloaded_bytes: 50,
        speed_bps: 10,
        eta_seconds: Some(5),
        ..Default::default()
    };

    for (locale, open_label, pause_label, quit_label) in [
        (Locale::En, "Open Segmenta", "Pause", "Quit application"),
        (
            Locale::Es,
            "Abrir Segmenta",
            "Pausar",
            "Salir de la aplicación",
        ),
        (
            Locale::PtBr,
            "Abrir Segmenta",
            "Pausar",
            "Sair do aplicativo",
        ),
    ] {
        let model = TrayModel::build(Some(&task), 1, locale);
        let open_item = model
            .items
            .iter()
            .find(|it| it.action() == Some(TrayAction::Open))
            .unwrap();
        let pause_item = model
            .items
            .iter()
            .find(|it| it.action() == Some(TrayAction::Pause))
            .unwrap();
        let quit_item = model
            .items
            .iter()
            .find(|it| it.action() == Some(TrayAction::Quit))
            .unwrap();

        assert_eq!(open_item.label(), open_label);
        assert_eq!(pause_item.label(), pause_label);
        assert_eq!(quit_item.label(), quit_label);
    }
}

#[test]
fn tray_speed_matches_model_format_speed_strictly() {
    let locale = Locale::Es;
    for speed in [0u64, 512, 1_000_000, 45_200_000, 100_000_000] {
        let task = DownloadItem {
            task_id: "task-spd".to_string(),
            filename: "file.iso".to_string(),
            status: DownloadStatus::Downloading,
            total_bytes: Some(1_000_000_000),
            downloaded_bytes: 100_000_000,
            speed_bps: speed,
            eta_seconds: Some(30),
            ..Default::default()
        };
        let model = TrayModel::build(Some(&task), 1, locale);
        let speed_eta = model.items[2].label();
        let expected_speed = segmenta_gui::model::format_speed(speed);
        assert!(
            speed_eta.starts_with(&expected_speed),
            "speed_eta `{speed_eta}` should start with expected speed `{expected_speed}`"
        );
    }
}

#[test]
fn format_tray_helpers_produce_expected_labels() {
    let locale = Locale::Es;
    let task = DownloadItem {
        task_id: "test-fmt".to_string(),
        filename: "test.iso".to_string(),
        total_bytes: Some(1_000),
        downloaded_bytes: 500,
        ..Default::default()
    };
    let progress = format_tray_progress(&task, locale);
    assert_eq!(progress, "50 % · 500 B de 1.0 kB");

    let speed_eta = format_tray_speed_eta(1024, Some(45), locale);
    assert!(speed_eta.contains("1.0 kB/s"));
    assert!(speed_eta.contains("45 s"));
}
