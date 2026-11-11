//! `segmenta-gui` entry point.
//!
//! Without arguments it opens the GTK window backed by the daemon socket.
//! `--smoke` builds the window with a synthetic download plus a progress event
//! and exits as soon as the model reflects them, which is the headless
//! acceptance check of S19 (`xvfb-run -a segmenta-gui --smoke`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use segmenta_gui::adw::prelude::*;
use segmenta_gui::client::{Client, DemoClient};
use segmenta_gui::{app, glib};

/// Task id used by the synthetic download.
const SMOKE_TASK: &str = "smoke-task";
/// Segments the smoke download is split into.
const SMOKE_SEGMENTS: u32 = 4;
/// Total size of the synthetic file.
const SMOKE_TOTAL: u64 = 4_000_000_000;

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("segmenta-gui {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "segmenta-gui {}\n\n\
             Usage: segmenta-gui [--smoke] [--screenshots DIR] [--version]\n\n\
             --smoke             build the window with a synthetic download and exit\n\
             --screenshots DIR   write the light and dark 1440x900 QA captures\n\
             --version           print the version and exit",
            env!("CARGO_PKG_VERSION")
        );
        return glib::ExitCode::SUCCESS;
    }
    if let Some(position) = args.iter().position(|arg| arg == "--screenshots") {
        let Some(dir) = args.get(position + 1) else {
            eprintln!("segmenta-gui: --screenshots needs a directory");
            return glib::ExitCode::FAILURE;
        };
        // The toolkit must exist before any widget is built.
        if let Err(err) = gtk::init() {
            eprintln!("segmenta-gui: cannot open the display: {err}");
            return glib::ExitCode::FAILURE;
        }
        return match segmenta_gui::screenshots::capture(std::path::Path::new(dir)) {
            Ok(captures) => {
                for path in captures.paths {
                    println!("wrote {}", path.display());
                }
                println!("captured rows={} state={}", captures.rows, captures.state);
                glib::ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("segmenta-gui: {err}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if args.iter().any(|arg| arg == "--smoke") {
        return run_smoke();
    }
    // `--smoke` is consumed here so GApplication does not reject it.
    let argv: Vec<String> = args
        .iter()
        .skip(1)
        .filter(|arg| *arg != "--smoke")
        .cloned()
        .collect();
    let mut forwarded = vec![args
        .first()
        .cloned()
        .unwrap_or_else(|| "segmenta-gui".to_string())];
    forwarded.extend(argv);
    let application = app::with_window();
    application.run_with_args(&forwarded)
}

fn smoke_row() -> Value {
    json!({
        "task_id": SMOKE_TASK,
        "url": "https://releases.ubuntu.com/26.04/ubuntu-26.04-desktop-amd64.iso",
        "final_path": "/tmp/segmenta-smoke/ubuntu-26.04-desktop-amd64.iso",
        "filename": "ubuntu-26.04-desktop-amd64.iso",
        "status": "downloading",
        "total_bytes": SMOKE_TOTAL,
        "downloaded_bytes": 1_000_000_000u64,
        "speed_bps": 45_200_000u64,
        "eta_seconds": 60u64,
        "retry_count": 0u32,
        "supports_ranges": true,
        "queue_name": "main",
        "created_at": "2026-04-24T10:00:00Z",
        "completed_at": null,
        "error_code": null,
        "error_message": null,
        "segments": smoke_segments(0)
    })
}

fn smoke_segments(downloaded: u64) -> Value {
    let span = SMOKE_TOTAL / u64::from(SMOKE_SEGMENTS);
    let list: Vec<Value> = (0..SMOKE_SEGMENTS)
        .map(|index| {
            let start = u64::from(index) * span;
            let end = start + span - 1;
            let next = if index == 0 {
                start + downloaded
            } else {
                start
            };
            json!({
                "segment_id": index,
                "start_offset": start,
                "end_offset": end,
                "next_offset": next,
                "status": "active",
                "speed_bps": 0u64
            })
        })
        .collect();
    Value::Array(list)
}

fn run_smoke() -> glib::ExitCode {
    // Hard watchdog: the smoke run must never hang a CI job.
    let finished = Arc::new(AtomicBool::new(false));
    let watchdog = Arc::clone(&finished);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(9));
        if !watchdog.load(Ordering::SeqCst) {
            eprintln!("SMOKE FAILED: no window state after 9 s");
            std::process::exit(1);
        }
    });

    let application = app::build();
    let debug = std::env::var_os("SEGMENTA_SMOKE_DEBUG").is_some();
    application.connect_activate(move |application| {
        if debug {
            eprintln!("SMOKE DEBUG activate");
        }
        let fake = DemoClient::with_downloads(vec![smoke_row()]);
        let client: Arc<dyn Client> = Arc::new(fake.clone());
        let window = app::build_window_with_client(application, client);
        fake.push_download_event(
            1,
            "task_progress",
            SMOKE_TASK,
            json!({
                "downloaded_bytes": 2_000_000_000u64,
                "total_bytes": SMOKE_TOTAL,
                "speed_bps": 80_000_000u64,
                "eta_seconds": 25u64,
                "segments": smoke_segments(1_000_000_000)
            }),
        );

        if debug {
            eprintln!("SMOKE DEBUG window built rows={}", window.rows().len());
        }
        let application = application.clone();
        let mut ticks = 0u32;
        let mut selected = false;
        glib::timeout_add_local(Duration::from_millis(100), move || {
            ticks += 1;
            // The row only exists after the first `download.list` answer, so the
            // selection is retried until it sticks.
            if !selected {
                selected = window.click_task(SMOKE_TASK);
            }
            let rows = window.rows().len();
            let segments = window.detail_segment_count();
            if debug && ticks <= 5 {
                eprintln!(
                    "SMOKE DEBUG tick={ticks} rows={rows} segments={segments:?} state={:?}",
                    window.view_state()
                );
            }
            if rows == 1 && segments == Some(SMOKE_SEGMENTS as usize) {
                println!(
                    "SMOKE OK rows={rows} segments={}",
                    segments.unwrap_or_default()
                );
                application.quit();
                return glib::ControlFlow::Break;
            }
            if ticks > 80 {
                eprintln!(
                    "SMOKE FAILED rows={rows} segments={segments:?} state={:?}",
                    window.view_state()
                );
                std::process::exit(1);
            }
            glib::ControlFlow::Continue
        });
    });

    // `--smoke` must not reach the GLib option parser.
    let code = application.run_with_args::<&str>(&["segmenta-gui"]);
    finished.store(true, Ordering::SeqCst);
    code
}
