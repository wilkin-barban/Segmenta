//! Deterministic screenshots for the release QA (S22/S25).
//!
//! The capture is produced by GTK itself (`GskRenderer` → `GdkTexture`), never by an
//! external screenshot tool, so light and dark are reproducible in CI/Xvfb.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::adw::prelude::*;
use crate::client::{Client, DemoClient};
use crate::window::SegmentaWindow;
use crate::{adw, glib, gtk};

/// Capture viewport of the QA (1440×900 desktop).
pub const CAPTURE_WIDTH: i32 = 1440;
/// Capture viewport height.
pub const CAPTURE_HEIGHT: i32 = 900;

/// Render the main window in light and dark and write two PNGs into `dir`.
///
/// The application loop owns the capture because GTK only paints a mapped window:
/// the sequence switches the style manager, waits for the restyle and renders.
pub struct CaptureOutcome {
    /// Written PNGs, light first.
    pub paths: Vec<PathBuf>,
    /// Rows the window had painted when the captures were taken.
    pub rows: usize,
    /// State label at capture time.
    pub state: String,
}

pub fn capture(dir: &Path) -> Result<CaptureOutcome, String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    let written: Rc<RefCell<Vec<PathBuf>>> = Rc::new(RefCell::new(Vec::new()));
    let failure: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let rows_seen: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let state_seen: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let rows_outer = Rc::clone(&rows_seen);
    let state_outer = Rc::clone(&state_seen);
    let app = adw::Application::builder()
        .application_id("io.github.wilkin.Segmenta.Captures")
        .build();
    {
        let written = Rc::clone(&written);
        let failure = Rc::clone(&failure);
        let dir = dir.to_path_buf();
        app.connect_activate(move |application| {
            let demo = DemoClient::with_downloads(vec![demo_row()]);
            let client: Arc<dyn Client> = Arc::new(demo);
            let window = SegmentaWindow::new(application, client);
            window
                .window()
                .set_default_size(CAPTURE_WIDTH, CAPTURE_HEIGHT);
            window.window().present();
            let phase = Rc::new(Cell::new(0u8));
            let written = Rc::clone(&written);
            let failure = Rc::clone(&failure);
            let rows_in = Rc::clone(&rows_seen);
            let state_in = Rc::clone(&state_seen);
            let started = std::time::Instant::now();
            let dir = dir.clone();
            let application = application.clone();
            gtk::glib::timeout_add_local(Duration::from_millis(250), move || {
                // Never capture the spinner: wait until the demo list is painted.
                if window.rows().is_empty() {
                    if started.elapsed() > Duration::from_secs(6) {
                        *failure.borrow_mut() = Some(
                            "the demo list never rendered; refusing to capture the loading state"
                                .to_string(),
                        );
                        application.quit();
                        return glib::ControlFlow::Break;
                    }
                    return glib::ControlFlow::Continue;
                }
                rows_in.set(window.rows().len());
                *state_in.borrow_mut() = format!("{:?}", window.view_state());
                let step = phase.get();
                phase.set(step + 1);
                match step {
                    0 => {
                        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight)
                    }
                    1 => {}
                    2 => {
                        if let Err(err) = save(&window, &dir.join("0.0.2-light.png"), &written) {
                            *failure.borrow_mut() = Some(err);
                            application.quit();
                            return glib::ControlFlow::Break;
                        }
                        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
                    }
                    3 => {}
                    4 => {
                        if let Err(err) = save(&window, &dir.join("0.0.2-dark.png"), &written) {
                            *failure.borrow_mut() = Some(err);
                        }
                        application.quit();
                        return glib::ControlFlow::Break;
                    }
                    _ => {}
                }
                glib::ControlFlow::Continue
            });
        });
    }
    let _ = app.run_with_args::<&str>(&["segmenta-gui"]);
    let failure_value = failure.borrow().clone();
    if let Some(err) = failure_value {
        return Err(err);
    }
    let paths = written.borrow().clone();
    let rows = rows_outer.get();
    let state = state_outer.borrow().clone();
    Ok(CaptureOutcome { paths, rows, state })
}

/// Snapshot the mapped window and write it as PNG.
fn save(
    window: &SegmentaWindow,
    path: &Path,
    written: &Rc<RefCell<Vec<PathBuf>>>,
) -> Result<(), String> {
    let texture = render(window)?;
    texture
        .save_to_png(path)
        .map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    written.borrow_mut().push(path.to_path_buf());
    Ok(())
}

fn render(window: &SegmentaWindow) -> Result<gtk::gdk::Texture, String> {
    let widget = window.window();
    let paintable = gtk::WidgetPaintable::new(Some(&widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(CAPTURE_WIDTH),
        f64::from(CAPTURE_HEIGHT),
    );
    let node = snapshot
        .to_node()
        .ok_or_else(|| "the window produced no render node".to_string())?;
    // `render_texture` requires a realized renderer: create a cairo renderer and
    // realize it against the window's surface before painting.
    let surface = widget
        .surface()
        .ok_or_else(|| "the window has no GdkSurface".to_string())?;
    let renderer = gtk::gsk::CairoRenderer::new();
    let _ = renderer.realize(Some(&surface));
    let texture = renderer.render_texture(&node, None::<&gtk::graphene::Rect>);
    // A realized renderer must be unrealized before it is disposed.
    renderer.unrealize();
    Ok(texture)
}

fn demo_row() -> serde_json::Value {
    serde_json::json!({
        "task_id": "qa-capture",
        "url": "https://releases.ubuntu.com/26.04/ubuntu-26.04-desktop-amd64.iso",
        "final_path": "/home/wilkin/Descargas/ubuntu-26.04-desktop-amd64.iso",
        "filename": "ubuntu-26.04-desktop-amd64.iso",
        "status": "downloading",
        "total_bytes": 4_294_967_296u64,
        "downloaded_bytes": 1_889_743_257u64,
        "speed_bps": 45_200_000u64,
        "eta_seconds": 53u64,
        "retry_count": 0u32,
        "supports_ranges": true,
        "queue_name": "main",
        "created_at": "2026-04-24T10:00:00Z",
        "completed_at": null,
        "error_code": null,
        "error_message": null,
        "segments": (0..8).map(|index| {
            serde_json::json!({
                "segment_id": index,
                "start_offset": index * 536_870_912u64,
                "end_offset": (index + 1) * 536_870_912u64 - 1,
                "next_offset": index * 536_870_912u64 + 236_223_232u64,
                "status": "active",
                "speed_bps": 5_650_000u64
            })
        }).collect::<Vec<_>>()
    })
}
