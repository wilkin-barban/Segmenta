//! System tray integration (ksni StatusNotifierItem).
//!
//! Provides a pure menu model testable without display or D-Bus (Part A) and
//! the live StatusNotifierItem tray service with background event polling (Part B).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::client::{Client, ClientEvent};
use crate::model::{format_bytes, format_eta, format_speed, DownloadItem, DownloadStatus};
use segmenta_i18n::{tr, Locale, Msg};

/// Actions offered by the system tray menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    /// Restore and present the main window.
    Open,
    /// Pause the active downloading task.
    Pause,
    /// Resume the active paused task.
    Resume,
    /// Cancel the active task.
    Cancel,
    /// Open the preferences dialog.
    Preferences,
    /// Terminate the application.
    Quit,
}

/// One item in the pure tray menu model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayMenuItem {
    /// Informational header line (e.g. filename or idle message).
    Header(String),
    /// Informational progress line (percentage and downloaded/total).
    Progress(String),
    /// Informational speed and ETA line.
    SpeedEta(String),
    /// Actionable menu entry.
    Action(TrayAction, String),
    /// Visual separator between action sections.
    Separator,
}

impl TrayMenuItem {
    /// The visible label of this menu item (empty for separators).
    pub fn label(&self) -> &str {
        match self {
            TrayMenuItem::Header(text)
            | TrayMenuItem::Progress(text)
            | TrayMenuItem::SpeedEta(text)
            | TrayMenuItem::Action(_, text) => text.as_str(),
            TrayMenuItem::Separator => "",
        }
    }

    /// Whether this item is a visual separator.
    pub fn is_separator(&self) -> bool {
        matches!(self, TrayMenuItem::Separator)
    }

    /// Associated action, if actionable.
    pub fn action(&self) -> Option<TrayAction> {
        match self {
            TrayMenuItem::Action(action, _) => Some(*action),
            _ => None,
        }
    }
}

/// Pure model of the tray menu contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayModel {
    pub items: Vec<TrayMenuItem>,
}

impl TrayModel {
    /// Builds the pure menu model from the active task snapshot.
    pub fn build(active_task: Option<&DownloadItem>, _active_count: usize, locale: Locale) -> Self {
        let mut items = Vec::new();
        match active_task {
            Some(task) => {
                // Header row with the file name
                let header = if task.filename.is_empty() {
                    task.task_id.clone()
                } else {
                    task.filename.clone()
                };
                items.push(TrayMenuItem::Header(header));

                // Row with percentage and downloaded/total
                items.push(TrayMenuItem::Progress(format_tray_progress(task, locale)));

                // Row with speed and ETA (reuses crate::model::format_speed)
                items.push(TrayMenuItem::SpeedEta(format_tray_speed_eta(
                    task.speed_bps,
                    task.eta_seconds,
                    locale,
                )));

                // Abrir Segmenta always
                items.push(TrayMenuItem::Action(
                    TrayAction::Open,
                    tr(locale, Msg::TrayOpenSegmenta).to_string(),
                ));

                // Pausar when downloading, Reanudar when paused
                if task.status == DownloadStatus::Downloading {
                    items.push(TrayMenuItem::Action(
                        TrayAction::Pause,
                        tr(locale, Msg::Pause).to_string(),
                    ));
                } else if task.status == DownloadStatus::Paused {
                    items.push(TrayMenuItem::Action(
                        TrayAction::Resume,
                        tr(locale, Msg::Resume).to_string(),
                    ));
                }

                // Cancelar when there is an active task
                items.push(TrayMenuItem::Action(
                    TrayAction::Cancel,
                    tr(locale, Msg::Cancel).to_string(),
                ));

                // Preferencias
                items.push(TrayMenuItem::Action(
                    TrayAction::Preferences,
                    tr(locale, Msg::Preferences).to_string(),
                ));

                // Separator
                items.push(TrayMenuItem::Separator);

                // Salir de la aplicación
                items.push(TrayMenuItem::Action(
                    TrayAction::Quit,
                    tr(locale, Msg::TrayQuit).to_string(),
                ));
            }
            None => {
                // Header says the idle text
                items.push(TrayMenuItem::Header(tr(locale, Msg::TrayIdle).to_string()));

                // Only Abrir/Preferencias/Salir are present
                items.push(TrayMenuItem::Action(
                    TrayAction::Open,
                    tr(locale, Msg::TrayOpenSegmenta).to_string(),
                ));
                items.push(TrayMenuItem::Action(
                    TrayAction::Preferences,
                    tr(locale, Msg::Preferences).to_string(),
                ));
                items.push(TrayMenuItem::Separator);
                items.push(TrayMenuItem::Action(
                    TrayAction::Quit,
                    tr(locale, Msg::TrayQuit).to_string(),
                ));
            }
        }
        Self { items }
    }
}

/// Selects the primary active task from `download.list` and counts active tasks.
///
/// Priority is given to transferring states: downloading, probing, retrying,
/// verifying, finalizing. If no task is currently transferring, returns the first
/// paused task so the user can resume it from the tray.
pub fn select_active_task(items: &[DownloadItem]) -> (Option<&DownloadItem>, usize) {
    let transferring: Vec<&DownloadItem> = items
        .iter()
        .filter(|item| is_transferring(item.status))
        .collect();
    let count = transferring.len();
    if let Some(first) = transferring.first() {
        return (Some(*first), count);
    }
    let paused = items
        .iter()
        .find(|item| item.status == DownloadStatus::Paused);
    (paused, 0)
}

/// Whether the task status represents an active transferring state.
pub fn is_transferring(status: DownloadStatus) -> bool {
    matches!(
        status,
        DownloadStatus::Downloading
            | DownloadStatus::Probing
            | DownloadStatus::Retrying
            | DownloadStatus::Verifying
            | DownloadStatus::Finalizing
    )
}

/// Percentage formatted as a string: `42 %`, or `—` when the total is unknown.
pub fn format_tray_percentage(item: &DownloadItem) -> String {
    if item.has_known_total() {
        format!("{:.0} %", (item.progress() * 100.0).clamp(0.0, 100.0))
    } else {
        "—".to_string()
    }
}

/// Row with percentage and downloaded/total.
///
/// When total is known: `25 % · 1.0 GB de 4.0 GB`
/// When total is unknown: `— · 500.0 MB` (indeterminate percentage rather than 0%)
pub fn format_tray_progress(item: &DownloadItem, locale: Locale) -> String {
    let size = if item.has_known_total() {
        format!(
            "{} {} {}",
            format_bytes(item.downloaded_bytes),
            tr(locale, Msg::Of),
            format_bytes(item.total_bytes.unwrap_or(0)),
        )
    } else {
        format_bytes(item.downloaded_bytes)
    };

    let percent = format_tray_percentage(item);
    format!("{percent} · {size}")
}

/// Row with speed and ETA (reuses `format_speed` and `format_eta`).
pub fn format_tray_speed_eta(speed_bps: u64, eta_seconds: Option<u64>, locale: Locale) -> String {
    let speed = format_speed(speed_bps);
    let eta = match eta_seconds {
        Some(seconds) if seconds > 0 => format!(
            "{} {}",
            format_eta(Some(seconds)),
            tr(locale, Msg::Eta).to_lowercase(),
        ),
        _ => format_eta(eta_seconds),
    };
    format!("{speed} · {eta}")
}

/// Commands dispatched from the tray thread to the GTK main thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayCommand {
    /// Present the main window.
    Open,
    /// Present the main window focused on a specific task.
    FocusTask(String),
    /// Pause a task.
    Pause(String),
    /// Resume a task.
    Resume(String),
    /// Cancel a task.
    Cancel(String),
    /// Open the preferences dialog.
    Preferences,
    /// Terminate the application.
    Quit,
}

/// StatusNotifierItem implementation for Segmenta.
struct SegmentaTray {
    sender: std::sync::mpsc::Sender<TrayCommand>,
    active_task: Option<DownloadItem>,
    active_count: usize,
}

impl ksni::Tray for SegmentaTray {
    fn id(&self) -> String {
        crate::window::APP_ID.to_string()
    }

    fn icon_name(&self) -> String {
        crate::window::APP_ID.to_string()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        Vec::new()
    }

    fn title(&self) -> String {
        "Segmenta".to_string()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.sender.send(TrayCommand::Open);
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        let _ = self.sender.send(TrayCommand::Open);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let locale = segmenta_i18n::current();
        let model = TrayModel::build(self.active_task.as_ref(), self.active_count, locale);
        let sender = self.sender.clone();
        let active_task_id = self.active_task.as_ref().map(|t| t.task_id.clone());

        model
            .items
            .into_iter()
            .map(|item| match item {
                TrayMenuItem::Header(text) => StandardItem {
                    label: text,
                    enabled: false,
                    disposition: Disposition::Informative,
                    ..Default::default()
                }
                .into(),
                TrayMenuItem::Progress(text) => StandardItem {
                    label: text,
                    enabled: false,
                    disposition: Disposition::Informative,
                    ..Default::default()
                }
                .into(),
                TrayMenuItem::SpeedEta(text) => StandardItem {
                    label: text,
                    enabled: false,
                    disposition: Disposition::Informative,
                    ..Default::default()
                }
                .into(),
                TrayMenuItem::Separator => MenuItem::Separator,
                TrayMenuItem::Action(action, label) => {
                    let sender = sender.clone();
                    let task_id = active_task_id.clone();
                    StandardItem {
                        label,
                        enabled: true,
                        activate: Box::new(move |_| {
                            let cmd = match action {
                                TrayAction::Open => TrayCommand::Open,
                                TrayAction::Pause => {
                                    if let Some(id) = &task_id {
                                        TrayCommand::Pause(id.clone())
                                    } else {
                                        return;
                                    }
                                }
                                TrayAction::Resume => {
                                    if let Some(id) = &task_id {
                                        TrayCommand::Resume(id.clone())
                                    } else {
                                        return;
                                    }
                                }
                                TrayAction::Cancel => {
                                    if let Some(id) = &task_id {
                                        TrayCommand::Cancel(id.clone())
                                    } else {
                                        return;
                                    }
                                }
                                TrayAction::Preferences => TrayCommand::Preferences,
                                TrayAction::Quit => TrayCommand::Quit,
                            };
                            let _ = sender.send(cmd);
                        }),
                        ..Default::default()
                    }
                    .into()
                }
            })
            .collect()
    }
}

/// Handle that owns the system tray background thread.
pub struct TrayHandle {
    stop: Arc<AtomicBool>,
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Spawns the system tray icon service and its background update thread.
///
/// Returns `None` if the StatusNotifierWatcher is absent or D-Bus registration fails.
pub fn spawn(
    client: Arc<dyn Client>,
    sender: std::sync::mpsc::Sender<TrayCommand>,
) -> Option<TrayHandle> {
    use ksni::blocking::TrayMethods;

    let tray = SegmentaTray {
        sender,
        active_task: None,
        active_count: 0,
    };

    let handle = match tray.spawn() {
        Ok(h) => h,
        Err(err) => {
            eprintln!("segmenta-gui: tray spawn failed (no StatusNotifierWatcher?): {err}");
            return None;
        }
    };

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);

    std::thread::Builder::new()
        .name("segmenta-tray".to_string())
        .spawn(move || {
            let mut last_refresh = Instant::now() - Duration::from_secs(10);
            let mut last_poll = Instant::now();
            let mut needs_refresh = true;

            while !stop_thread.load(Ordering::Relaxed) {
                // Drain client notifications
                while let Some(event) = client.poll_event() {
                    match event {
                        ClientEvent::Notification { method, .. } => {
                            if method == "task_progress"
                                || method == "task_added"
                                || method == "task_updated"
                            {
                                needs_refresh = true;
                            }
                        }
                        ClientEvent::Connected | ClientEvent::Disconnected(_) => {
                            needs_refresh = true;
                        }
                    }
                }

                // Fallback poll every 2 seconds
                let now = Instant::now();
                if now.duration_since(last_poll) >= Duration::from_secs(2) {
                    needs_refresh = true;
                    last_poll = now;
                }

                // Throttled refresh (at most once per second)
                if needs_refresh && now.duration_since(last_refresh) >= Duration::from_secs(1) {
                    if let Ok(value) = client.request(
                        "download.list",
                        serde_json::json!({ "filter": "all", "limit": 100 }),
                    ) {
                        let tasks: Vec<DownloadItem> =
                            serde_json::from_value::<Vec<DownloadItem>>(value).unwrap_or_default();
                        let (active, count) = select_active_task(&tasks);
                        let active_cloned = active.cloned();

                        handle.update(move |tray: &mut SegmentaTray| {
                            tray.active_task = active_cloned;
                            tray.active_count = count;
                        });
                    }
                    last_refresh = Instant::now();
                    needs_refresh = false;
                }

                std::thread::sleep(Duration::from_millis(100));
            }
        })
        .ok()?;

    Some(TrayHandle { stop })
}
