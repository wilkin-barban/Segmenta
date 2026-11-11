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

/// Creates the main window for the default (socket) client.
pub fn build_window_for_client(app: &adw::Application) {
    let client: Arc<dyn Client> = Arc::new(UnixClient::from_env());
    build_window_with_client(app, client);
}

/// Creates the main window with an injected client (tests and `--smoke`).
pub fn build_window_with_client(app: &adw::Application, client: Arc<dyn Client>) -> SegmentaWindow {
    SegmentaWindow::new(app, client)
}
