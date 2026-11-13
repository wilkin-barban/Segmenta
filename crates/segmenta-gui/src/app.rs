//! `AdwApplication` setup.
//!
//! The GUI is only an IPC client: it never opens the database, never speaks
//! HTTP and never touches the engine. The whole contract is
//! `$XDG_RUNTIME_DIR/segmenta/daemon.sock` (or `$SEGMENTA_SOCKET`) over NDJSON
//! JSON-RPC 2.0.

use std::sync::Arc;

use adw::prelude::*;

use crate::client::{Client, UnixClient};
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
        existing.window().present();
        return;
    }
    let client: Arc<dyn Client> = Arc::new(UnixClient::from_env());
    let window = build_window_with_client(app, client);
    if let Some(task_id) = take_pending_show_task() {
        focus_when_ready(window.clone(), task_id);
    }
    OPEN_WINDOWS.with(|windows| windows.borrow_mut().push(window));
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
