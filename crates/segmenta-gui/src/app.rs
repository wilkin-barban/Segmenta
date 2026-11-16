//! `AdwApplication` setup.
//!
//! The GUI is only an IPC client: it never opens the database, never speaks
//! HTTP and never touches the engine. The whole contract is
//! `$XDG_RUNTIME_DIR/segmenta/daemon.sock` (or `$SEGMENTA_SOCKET`) over NDJSON
//! JSON-RPC 2.0.

use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;

use crate::client::{Client, UnixClient};
use crate::tray::{self, TrayCommand, TrayHandle};
use crate::window::{SegmentaWindow, APP_ID};
use crate::{adw, gtk};

/// Builds the application object (one per process).
pub fn build() -> adw::Application {
    adw::Application::builder().application_id(APP_ID).build()
}

/// Builds the application and attaches the window factory.
pub fn with_window() -> adw::Application {
    let app = build();
    app.connect_startup(|_| {
        // The desktop file and the AppStream metadata ship the icon; adding the
        // development path keeps `--smoke` from showing a missing icon when the
        // crate runs from the build tree.
        if let Some(display) = gtk::gdk::Display::default() {
            let theme = gtk::IconTheme::for_display(&display);
            for candidate in ["data", "crates/segmenta-gui/data"] {
                let path = std::path::Path::new(candidate);
                if path.join(format!("{APP_ID}.svg")).exists() {
                    theme.add_search_path(path);
                    break;
                }
            }
        }
    });
    app.connect_activate(build_window_for_client);
    app
}

/// Task the window must focus once its first `download.list` answer lands.
///
/// `--show-task` is parsed in `main` before GLib sees the argv, so the id travels
/// through this slot instead of the GApplication options.
static SHOW_TASK: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Removes `--show-task <id>` (or `--show-task=<id>`) from `argv` and returns the id.
///
/// The pair is consumed so the GLib option parser never rejects an exported flag.
pub fn take_show_task(argv: &mut Vec<String>) -> Option<String> {
    let mut task_id = None;
    let mut index = 0;
    while index < argv.len() {
        if argv[index] == "--show-task" {
            let value = argv.get(index + 1).cloned();
            let last = if value.is_some() { index + 1 } else { index };
            argv.drain(index..=last);
            if let Some(value) = value {
                task_id = Some(value);
            }
            continue;
        }
        if let Some(rest) = argv[index].strip_prefix("--show-task=") {
            task_id = Some(rest.to_string());
            argv.remove(index);
            continue;
        }
        index += 1;
    }
    task_id
}

/// Remembers the task the freshly built window must focus.
pub fn set_show_task(task_id: Option<String>) {
    *SHOW_TASK.lock().unwrap_or_else(|err| err.into_inner()) = task_id;
}

/// Takes the remembered task id, if any.
fn take_pending_show_task() -> Option<String> {
    SHOW_TASK
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
}

/// Ticks the focus retry waits between attempts on the first `download.list`.
const FOCUS_TICKS: u32 = 50;

/// Focuses `task_id` as soon as its row exists.
///
/// The window reloads asynchronously, so the row is not there on the first tick.
/// The retry is bounded: a task that never appears must not keep a timer alive.
fn focus_when_ready(window: SegmentaWindow, task_id: String) {
    let ticks = std::rc::Rc::new(std::cell::Cell::new(0u32));
    crate::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        if window.focus_task(&task_id) {
            return crate::glib::ControlFlow::Break;
        }
        ticks.set(ticks.get() + 1);
        if ticks.get() >= FOCUS_TICKS {
            return crate::glib::ControlFlow::Break;
        }
        crate::glib::ControlFlow::Continue
    });
}

thread_local! {
    /// Windows opened by the application.
    ///
    /// `SegmentaWindow` is a thin wrapper over `Rc<Inner>`; GTK keeps the widget alive
    /// but not that state, so the tick loop and the model died as soon as `activate`
    /// returned and the view stayed on "Conectando…" forever. The registry owns the
    /// Rust side of every open window.
    static OPEN_WINDOWS: std::cell::RefCell<Vec<SegmentaWindow>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static TRAY_HANDLE: std::cell::RefCell<Option<TrayHandle>> =
        const { std::cell::RefCell::new(None) };
    static APP_HOLD: std::cell::RefCell<Option<gtk::gio::ApplicationHoldGuard>> =
        const { std::cell::RefCell::new(None) };
}

/// Creates the main window for the default (socket) client and keeps it alive.
pub fn build_window_for_client(app: &adw::Application) {
    // `activate` can fire more than once (a desktop launch plus a second request). A new
    // window per activation leaves a second, still-loading window stacked on the first:
    // its spinner paints over the list of the one the user is looking at.
    let reused = OPEN_WINDOWS.with(|windows| {
        windows
            .borrow()
            .iter()
            .find(|window| {
                window
                    .window()
                    .application()
                    .map(|candidate| candidate.application_id() == app.application_id())
                    .unwrap_or(false)
            })
            .cloned()
    });
    if let Some(existing) = reused {
        existing.window().set_visible(true);
        existing.window().present();
        return;
    }
    let client: Arc<dyn Client> = Arc::new(UnixClient::from_env());
    let window = build_window_with_client(app, client);
    if let Some(task_id) = take_pending_show_task() {
        focus_when_ready(window.clone(), task_id);
    }
    OPEN_WINDOWS.with(|windows| windows.borrow_mut().push(window.clone()));

    // Start the system tray integration once the window exists (S2, S6).
    let tray_started = TRAY_HANDLE.with(|tray| tray.borrow().is_some());
    if !tray_started {
        let (sender, receiver) = std::sync::mpsc::channel();
        let window_handle = window.clone();
        let app_handle = app.clone();

        // The tray thread cannot touch GTK objects, so commands cross through a plain
        // channel drained by the main loop. 250 ms is imperceptible for a click and
        // avoids a 20 Hz wakeup for the whole life of the process.
        crate::glib::timeout_add_local(Duration::from_millis(250), move || {
            loop {
                match receiver.try_recv() {
                    Ok(cmd) => match cmd {
                        TrayCommand::Quit => {
                            window_handle.set_tray_active(false);
                            APP_HOLD.with(|slot| *slot.borrow_mut() = None);
                            app_handle.quit();
                            return crate::glib::ControlFlow::Break;
                        }
                        cmd => {
                            window_handle.handle_tray_command(cmd);
                        }
                    },
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        return crate::glib::ControlFlow::Break;
                    }
                }
            }
            crate::glib::ControlFlow::Continue
        });

        // The StatusNotifierWatcher belongs to the shell: it is absent on a headless
        // session, while the AppIndicator extension is still loading, or after a shell
        // restart. Giving up on the first attempt left Segmenta without an icon for the
        // rest of its life, so a missing watcher is retried until it appears.
        if start_tray(app, &window, sender.clone()) {
            return;
        }
        let retry_app = app.clone();
        let retry_window = window.clone();
        crate::glib::timeout_add_local(Duration::from_secs(5), move || {
            if start_tray(&retry_app, &retry_window, sender.clone()) {
                crate::glib::ControlFlow::Break
            } else {
                crate::glib::ControlFlow::Continue
            }
        });
    }
}

/// Publishes the tray item, twice-guarded against a second registration.
///
/// Returns whether the item is now registered. Never fails the caller: a session
/// without a StatusNotifierWatcher keeps working exactly as before the tray existed.
fn start_tray(
    app: &adw::Application,
    window: &SegmentaWindow,
    sender: std::sync::mpsc::Sender<TrayCommand>,
) -> bool {
    if TRAY_HANDLE.with(|slot| slot.borrow().is_some()) {
        return true;
    }
    let client: Arc<dyn Client> = Arc::new(UnixClient::from_env());
    let Some(handle) = tray::spawn(client, sender) else {
        return false;
    };
    let guard = app.hold();
    APP_HOLD.with(|slot| *slot.borrow_mut() = Some(guard));
    window.set_tray_active(true);
    TRAY_HANDLE.with(|slot| *slot.borrow_mut() = Some(handle));
    true
}

/// How many windows the application is keeping alive (regression guard for the
/// normal startup path, which used to drop the window state).
pub fn open_window_count() -> usize {
    OPEN_WINDOWS.with(|windows| windows.borrow().len())
}

/// Creates the main window with an injected client (tests and `--smoke`).
pub fn build_window_with_client(app: &adw::Application, client: Arc<dyn Client>) -> SegmentaWindow {
    SegmentaWindow::new(app, client)
}

#[cfg(test)]
mod tests {
    use super::take_show_task;

    #[test]
    fn show_task_is_consumed_from_the_argv_in_both_spellings() {
        let mut argv = vec![
            "segmenta-gui".to_string(),
            "--show-task".to_string(),
            "task-a".to_string(),
        ];
        assert_eq!(take_show_task(&mut argv).as_deref(), Some("task-a"));
        assert_eq!(argv, vec!["segmenta-gui".to_string()]);

        let mut argv = vec!["segmenta-gui".to_string(), "--show-task=task-b".to_string()];
        assert_eq!(take_show_task(&mut argv).as_deref(), Some("task-b"));
        assert_eq!(argv, vec!["segmenta-gui".to_string()]);
    }

    #[test]
    fn show_task_without_a_value_leaves_no_dangling_flag() {
        let mut argv = vec!["segmenta-gui".to_string(), "--show-task".to_string()];
        assert_eq!(take_show_task(&mut argv), None);
        assert_eq!(argv, vec!["segmenta-gui".to_string()]);
    }
}
