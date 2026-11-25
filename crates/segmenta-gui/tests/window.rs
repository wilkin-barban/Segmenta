//! GUI tests: they need an X display, so run them under `xvfb-run -a`.
//!
//! Without a display the tests skip with a clear message instead of failing the
//! suite (`gtk::init()` reports the missing display). GTK is not thread safe and
//! `gtk::init()` refuses to run twice, so every test body is executed on one
//! dedicated harness thread that initialises the toolkit once.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use segmenta_gui::adw::prelude::*;
use segmenta_gui::client::{Client, DemoClient};
use segmenta_gui::model::{DownloadStatus, Filter, Settings, ERR_ANTIVIRUS_UNAVAILABLE};
use segmenta_gui::prefs;
use segmenta_gui::tray::TrayCommand;
use segmenta_gui::window::{SegmentaWindow, ViewState};
use segmenta_gui::{adw, glib, gtk};

/// One test body, executed on the toolkit thread.
type Job = Box<dyn FnOnce() + Send + 'static>;

/// Outcome of one test body, carried back to the test thread.
type Outcome = Result<(), String>;

struct Harness {
    jobs: Sender<Job>,
    init: Result<(), String>,
}

fn harness() -> &'static Harness {
    static HARNESS: OnceLock<Harness> = OnceLock::new();
    HARNESS.get_or_init(|| {
        let (jobs, queue) = channel::<Job>();
        let (ready, initialised) = channel::<Result<(), String>>();
        std::thread::Builder::new()
            .name("gtk-harness".to_string())
            .spawn(move || {
                let init = gtk::init().map_err(|err| err.to_string());
                let usable = init.is_ok();
                let _ = ready.send(init);
                if !usable {
                    return;
                }
                for job in queue {
                    job();
                }
            })
            .expect("spawn the GTK harness thread");
        let init = initialised.recv().expect("GTK harness never answered");
        Harness { jobs, init }
    })
}

/// Runs `body` on the toolkit thread, propagating panics to the test thread.
fn run_gui<F>(name: &str, body: F)
where
    F: FnOnce() + Send + 'static,
{
    let harness = harness();
    if let Err(err) = &harness.init {
        eprintln!(
            "skipping GUI test `{name}`: gtk::init() failed ({err}); \
             run `xvfb-run -a cargo test -p segmenta-gui`"
        );
        return;
    }
    let (done, outcome): (Sender<Outcome>, Receiver<Outcome>) = channel();
    let job: Job = Box::new(move || {
        // Capture the formatted panic so GTK assertions keep their message.
        let captured: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
        let sink = Arc::clone(&captured);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            *sink.lock().unwrap_or_else(|err| err.into_inner()) = Some(info.to_string());
        }));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        std::panic::set_hook(previous);
        let result = match result {
            Ok(()) => Ok(()),
            Err(payload) => {
                let fallback = captured
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .clone()
                    .unwrap_or_else(|| "the GUI test panicked".to_string());
                Err(panic_message(&payload).unwrap_or(fallback))
            }
        };
        let _ = done.send(result);
    });
    if harness.jobs.send(job).is_err() {
        panic!("the GTK harness thread is gone");
    }
    match outcome.recv_timeout(Duration::from_secs(60)) {
        Ok(Ok(())) => {}
        Ok(Err(message)) => panic!("{message}"),
        Err(err) => panic!("the GUI test did not finish: {err}"),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> Option<String> {
    if let Some(message) = payload.downcast_ref::<&str>() {
        Some((*message).to_string())
    } else {
        payload.downcast_ref::<String>().cloned()
    }
}

fn app(id: &str) -> adw::Application {
    let app = adw::Application::builder().application_id(id).build();
    let _ = app.register(None::<&gtk::gio::Cancellable>);
    app
}

fn pump_until(mut predicate: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    predicate()
}

fn segments(count: u32, total: u64) -> Value {
    let span = total / u64::from(count);
    let list: Vec<Value> = (0..count)
        .map(|index| {
            let start = u64::from(index) * span;
            let end = start + span - 1;
            json!({
                "segment_id": index,
                "start_offset": start,
                "end_offset": end,
                "next_offset": start,
                "status": "active",
                "speed_bps": 0u64
            })
        })
        .collect();
    Value::Array(list)
}

/// A `download.list`/`download.status` row as the daemon sends it.
fn download_row(task_id: &str, status: &str, segment_count: u32) -> Value {
    json!({
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
        "segments": segments(segment_count, 4_000_000_000u64)
    })
}

fn window_with(app: &adw::Application, fake: &DemoClient) -> SegmentaWindow {
    let client: Arc<dyn Client> = Arc::new(fake.clone());
    SegmentaWindow::new(app, client)
}

/// Empty application window used as the parent of the preferences dialog.
fn prefs_parent(app: &adw::Application) -> adw::ApplicationWindow {
    adw::ApplicationWindow::builder()
        .application(app)
        .title("Segmenta")
        .build()
}

/// `settings.get` answer with the antivirus keys A2 persists (OpenSpec 12 §5).
fn antivirus_settings() -> Settings {
    serde_json::from_value(json!({
        "max_connections": 8,
        "download_dir": "/home/user/Descargas",
        "antivirus_enabled": true,
        "antivirus_timeout_secs": 300,
        "antivirus_quarantine_dir": ANTIVIRUS_QUARANTINE_DIR
    }))
    .expect("the daemon settings payload must deserialize")
}

/// Records every `settings.update` patch the page enqueues, in order.
/// Records the patches a preferences control enqueues.
type PatchLog = (Rc<RefCell<Vec<Value>>>, Rc<dyn Fn(Value)>);

fn recorded_patches() -> PatchLog {
    let log: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&log);
    (
        log,
        Rc::new(move |patch: Value| sink.borrow_mut().push(patch)),
    )
}

const ANTIVIRUS_QUARANTINE_DIR: &str = "/home/user/.local/share/segmenta/quarantine";

/// Path the daemon reports for the file it moved out of Downloads.
const QUARANTINED_FILE: &str = "/home/user/.local/share/segmenta/quarantine/eicar.com.txt";

/// The three catalog spellings of the quarantine state, so the assertion does not
/// depend on the locale the suite runs with.
const QUARANTINE_LABELS: [&str; 3] = ["En cuarentena", "In quarantine", "Em quarentena"];

/// A failed task the antivirus quarantined: `ERR_ANTIVIRUS_FOUND` (1046).
fn quarantined_row() -> Value {
    let mut row = download_row("infected", "failed", 1);
    row["final_path"] = json!("/home/user/Descargas/eicar.com.txt");
    row["filename"] = json!("eicar.com.txt");
    row["error_code"] = json!(1046);
    row["error_message"] = json!("ERR_ANTIVIRUS_FOUND: Eicar-Test-Signature FOUND");
    row["quarantine_path"] = json!(QUARANTINED_FILE);
    row
}

/// A row with no segments, cheap enough to build a snapshot of hundreds.
fn bulk_row(index: usize) -> Value {
    let mut row = download_row(&format!("task-{index}"), "completed", 1);
    row["segments"] = json!([]);
    row
}

#[test]
fn queue_names_come_from_queue_list_not_from_the_downloads() {
    run_gui("queue_names_come_from_queue_list", || {
        let app = app("io.github.wilkin.Segmenta.TestQueueNames");
        // The daemon knows `main` and `noche`; the snapshot only carries `main` and
        // a stale `legacy` queue that no longer exists in `queue.list`.
        let mut legacy = download_row("task-2", "completed", 1);
        legacy["queue_name"] = json!("legacy");
        let fake =
            DemoClient::with_downloads(vec![download_row("task-1", "downloading", 1), legacy]);
        fake.set_queues(vec!["main", "noche"]);
        let window = window_with(&app, &fake);

        assert!(
            pump_until(
                || window.queue_names() == vec!["main".to_string(), "noche".to_string()],
                Duration::from_secs(5)
            ),
            "the queue sidebar painted {:?}",
            window.queue_names()
        );
        assert_eq!(window.rows().len(), 2, "the snapshot rows were lost");
        assert!(window.queues_visible());
    });
}

#[test]
fn empty_queue_list_paints_no_phantom_queue() {
    run_gui("empty_queue_list_paints_no_phantom_queue", || {
        let app = app("io.github.wilkin.Segmenta.TestNoQueues");
        // The download still names `main`: the sidebar must not derive a queue from it.
        let fake = DemoClient::with_downloads(vec![download_row("task-1", "downloading", 1)]);
        fake.set_queues(vec![]);
        let window = window_with(&app, &fake);

        assert!(
            pump_until(|| window.rows().len() == 1, Duration::from_secs(5)),
            "the listed download never became a row (state {:?})",
            window.view_state()
        );
        // Give the queue answer time to arrive before asserting the emptiness.
        let _ = pump_until(|| !window.queues_visible(), Duration::from_millis(500));
        assert!(
            window.queue_names().is_empty(),
            "phantom queues painted: {:?}",
            window.queue_names()
        );
        assert!(
            !window.queues_visible(),
            "the empty queue sidebar is still painted"
        );
    });
}

#[test]
fn capped_snapshot_paints_the_truncation_notice_with_the_real_total() {
    run_gui("capped_snapshot_paints_the_truncation_notice", || {
        let app = app("io.github.wilkin.Segmenta.TestTruncated");
        // One row past the snapshot cap: the list is capped, the total is not 500.
        let rows: Vec<Value> = (0..501).map(bulk_row).collect();
        let fake = DemoClient::with_downloads(rows);
        let window = window_with(&app, &fake);

        assert!(
            pump_until(
                || window
                    .truncation_notice()
                    .is_some_and(|notice| notice.contains("501")),
                Duration::from_secs(10)
            ),
            "the capped snapshot painted no notice (state {:?}, rows {})",
            window.view_state(),
            window.rows().len()
        );
        let notice = window.truncation_notice().expect("notice");
        assert!(notice.contains("500"), "the notice hides the cap: {notice}");
        assert!(
            notice.contains("501"),
            "the notice hides the total: {notice}"
        );
        assert_eq!(window.rows().len(), 500, "the snapshot cap changed");
    });
}

#[test]
fn snapshot_of_exactly_the_page_size_paints_no_notice() {
    run_gui("snapshot_of_exactly_the_page_size_paints_no_notice", || {
        let app = app("io.github.wilkin.Segmenta.TestExactPage");
        // A full page that happens to be the whole table: counting must confirm the
        // total instead of warning about a truncation that never happened.
        let rows: Vec<Value> = (0..500).map(bulk_row).collect();
        let fake = DemoClient::with_downloads(rows);
        let window = window_with(&app, &fake);

        assert!(
            pump_until(|| window.rows().len() == 500, Duration::from_secs(10)),
            "the full page never reached the list (state {:?})",
            window.view_state()
        );
        // Pump while the counting answer lands: a wrong total would paint the notice
        // and keep it painted, so waiting cannot hide the bug.
        let _ = pump_until(|| false, Duration::from_millis(500));
        assert_eq!(window.truncation_notice(), None);
        assert_eq!(window.rows().len(), 500);
    });
}

#[test]
fn uncapped_snapshot_paints_no_truncation_notice() {
    run_gui("uncapped_snapshot_paints_no_truncation_notice", || {
        let app = app("io.github.wilkin.Segmenta.TestNotTruncated");
        let rows: Vec<Value> = (0..3).map(bulk_row).collect();
        let fake = DemoClient::with_downloads(rows);
        let window = window_with(&app, &fake);

        assert!(
            pump_until(|| window.rows().len() == 3, Duration::from_secs(5)),
            "the uncapped snapshot never listed its rows (state {:?})",
            window.view_state()
        );
        assert_eq!(window.truncation_notice(), None);
    });
}

#[test]
fn listed_download_becomes_a_row() {
    run_gui("listed_download_becomes_a_row", || {
        let app = app("io.github.wilkin.Segmenta.TestRow");
        let fake = DemoClient::with_downloads(vec![download_row("task-1", "downloading", 4)]);
        let window = window_with(&app, &fake);

        assert!(
            pump_until(|| window.rows().len() == 1, Duration::from_secs(5)),
            "the listed download never became a row (state {:?})",
            window.view_state()
        );
        let rows = window.rows();
        assert_eq!(rows[0].task_id, "task-1");
        assert_eq!(rows[0].filename, "ubuntu-26.04-desktop-amd64.iso");
        assert_eq!(rows[0].status, DownloadStatus::Downloading);
        assert_eq!(window.view_state(), ViewState::Ready);
        assert_eq!(window.counts().all, 1);
        assert_eq!(window.counts().active, 1);
    });
}

#[test]
fn task_progress_updates_the_row_model() {
    run_gui("task_progress_updates_the_row_model", || {
        let app = app("io.github.wilkin.Segmenta.TestProgress");
        let fake = DemoClient::with_downloads(vec![download_row("task-1", "downloading", 4)]);
        let window = window_with(&app, &fake);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));

        fake.push_download_event(
            1,
            "task_progress",
            "task-1",
            json!({
                "downloaded_bytes": 2_000_000_000u64,
                "total_bytes": 4_000_000_000u64,
                "speed_bps": 80_000_000u64,
                "eta_seconds": 25u64,
                "segments": segments(4, 4_000_000_000u64)
            }),
        );

        assert!(
            pump_until(
                || !window.rows().is_empty() && window.rows()[0].speed_bps == 80_000_000,
                Duration::from_secs(5)
            ),
            "the progress event never reached the row model"
        );
        let row = &window.rows()[0];
        assert_eq!(row.downloaded_bytes, 2_000_000_000);
        assert_eq!(row.speed_bps, 80_000_000);
        assert_eq!(row.eta_seconds, Some(25));
        assert_eq!(row.segments.len(), 4);
        // The progress notification must not duplicate the row.
        assert_eq!(window.rows().len(), 1);
    });
}

#[test]
fn row_exposes_dedicated_speed_label_while_downloading_and_clears_on_completion() {
    run_gui(
        "row_exposes_dedicated_speed_label_while_downloading_and_clears_on_completion",
        || {
            let app = app("io.github.wilkin.Segmenta.TestSpeedLabel");
            let fake =
                DemoClient::with_downloads(vec![download_row("task-speed", "downloading", 4)]);
            let window = window_with(&app, &fake);
            assert!(pump_until(
                || window.rows().len() == 1,
                Duration::from_secs(5)
            ));

            fake.push_download_event(
                1,
                "task_progress",
                "task-speed",
                json!({
                    "downloaded_bytes": 2_000_000_000u64,
                    "total_bytes": 4_000_000_000u64,
                    "speed_bps": 45_200_000u64,
                    "eta_seconds": 44u64,
                    "segments": []
                }),
            );

            let expected_speed = segmenta_gui::model::format_speed(45_200_000);
            assert!(
                pump_until(
                    || window.row_speed_labels().first().map(|s| s.as_str())
                        == Some(&expected_speed),
                    Duration::from_secs(5)
                ),
                "the row never exposed the formatted speed label; got {:?}",
                window.row_speed_labels()
            );

            fake.push_download_event(
                2,
                "state_changed",
                "task-speed",
                json!({
                    "old_state": "downloading",
                    "new_state": "completed"
                }),
            );

            assert!(
                pump_until(
                    || {
                        let labels = window.row_speed_labels();
                        labels.first().map(|s| s.is_empty()).unwrap_or(true)
                    },
                    Duration::from_secs(5)
                ),
                "speed label was not cleared on completion; got {:?}",
                window.row_speed_labels()
            );
        },
    );
}

/// A media/torrent task as the daemon reports it: no size known, no segment table.
fn media_row(task_id: &str, media_kind: &str) -> Value {
    let mut row = download_row(task_id, "downloading", 1);
    row["url"] = json!("https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    row["filename"] = json!("video.webm");
    row["final_path"] = json!("/home/user/Descargas/video.webm");
    row["total_bytes"] = json!(null);
    row["downloaded_bytes"] = json!(0u64);
    row["segments"] = json!([]);
    row["media_kind"] = json!(media_kind);
    row
}

#[test]
fn the_list_paints_the_daemon_newest_first_order() {
    run_gui("list_paints_daemon_order", || {
        let app = app("io.github.wilkin.Segmenta.TestListOrder");
        // `download.list` answers `created_at DESC, rowid DESC`: newest on top.
        let mut newest = download_row("task-new", "downloading", 1);
        newest["created_at"] = json!("2026-04-24T12:00:00Z");
        let mut middle = download_row("task-mid", "completed", 1);
        middle["created_at"] = json!("2026-04-24T11:00:00Z");
        let mut oldest = download_row("task-old", "completed", 1);
        oldest["created_at"] = json!("2026-04-24T10:00:00Z");
        let fake = DemoClient::with_downloads(vec![newest, middle, oldest]);
        let window = window_with(&app, &fake);

        assert!(
            pump_until(|| window.rows().len() == 3, Duration::from_secs(5)),
            "the snapshot never listed its three rows (state {:?})",
            window.view_state()
        );
        let painted: Vec<String> = window
            .rows()
            .iter()
            .map(|item| item.task_id.clone())
            .collect();
        assert_eq!(
            painted,
            vec!["task-new", "task-mid", "task-old"],
            "the list reversed the daemon order"
        );

        // A reload must repaint the same order, not flip it again.
        window.reload();
        let _ = pump_until(|| false, Duration::from_millis(300));
        let painted: Vec<String> = window
            .rows()
            .iter()
            .map(|item| item.task_id.clone())
            .collect();
        assert_eq!(painted, vec!["task-new", "task-mid", "task-old"]);
    });
}

#[test]
fn direct_download_detail_bar_changes_with_each_progress_event() {
    run_gui("direct_download_detail_bar_changes", || {
        let app = app("io.github.wilkin.Segmenta.TestDirectBar");
        let fake = DemoClient::with_downloads(vec![download_row("task-1", "downloading", 4)]);
        let window = window_with(&app, &fake);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));
        assert!(window.click_task("task-1"), "task-1 is not selectable");
        assert!(
            pump_until(|| window.detail().is_some(), Duration::from_secs(5)),
            "the detail page never opened"
        );
        // The row starts at 1.2 GB of 4.0 GB: 0.30 painted by the widget itself.
        let initial = window
            .detail_progress_fraction()
            .expect("the detail page paints a progress bar");
        assert!(
            (initial - 0.3).abs() < 1e-9,
            "the detail bar opened at {initial}"
        );

        fake.push_download_event(
            1,
            "task_progress",
            "task-1",
            json!({
                "downloaded_bytes": 1_000_000_000u64,
                "total_bytes": 4_000_000_000u64,
                "speed_bps": 10_000_000u64,
                "eta_seconds": 300u64,
                "segments": segments(4, 4_000_000_000u64)
            }),
        );
        assert!(
            pump_until(
                || window.detail_progress_fraction() == Some(0.25),
                Duration::from_secs(5)
            ),
            "DEBUG rows={:?} detail={:?} fraction={:?} text={:?} row_fractions={:?} list_calls={} state={:?}",
            window.rows().iter().map(|row| (row.task_id.clone(), row.downloaded_bytes, row.speed_bps, row.total_bytes)).collect::<Vec<_>>(),
            window.detail().map(|item| (item.downloaded_bytes, item.total_bytes)),
            window.detail_progress_fraction(),
            window.detail_progress_text(),
            window.row_progress_fractions(),
            fake.list_calls(),
            window.view_state()
        );
        assert!(
            pump_until(
                || window.row_progress_fractions().first().copied() == Some(0.25),
                Duration::from_secs(5)
            ),
            "the list row painted {:?} after the first event",
            window.row_progress_fractions()
        );

        fake.push_download_event(
            2,
            "task_progress",
            "task-1",
            json!({
                "downloaded_bytes": 3_000_000_000u64,
                "total_bytes": 4_000_000_000u64,
                "speed_bps": 20_000_000u64,
                "eta_seconds": 50u64,
                "segments": segments(4, 4_000_000_000u64)
            }),
        );
        assert!(
            pump_until(
                || window.detail_progress_fraction() == Some(0.75),
                Duration::from_secs(5)
            ),
            "the detail bar stayed at {:?} after the second event",
            window.detail_progress_fraction()
        );
        assert!(
            pump_until(
                || window.row_progress_fractions().first().copied() == Some(0.75),
                Duration::from_secs(5)
            ),
            "the list row painted {:?} after the second event",
            window.row_progress_fractions()
        );
        assert_eq!(
            window.detail_progress_indeterminate(),
            Some(false),
            "a known size must not paint a pulse"
        );
    });
}

#[test]
fn media_progress_with_an_unknown_total_pulses_and_shows_the_real_bytes() {
    run_gui("media_progress_unknown_total", || {
        let app = app("io.github.wilkin.Segmenta.TestMediaBar");
        let fake = DemoClient::with_downloads(vec![media_row("media-1", "video")]);
        let window = window_with(&app, &fake);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));
        assert!(window.click_task("media-1"), "media-1 is not selectable");
        assert!(
            pump_until(|| window.detail().is_some(), Duration::from_secs(5)),
            "the detail page never opened"
        );
        assert_eq!(
            window.detail_progress_indeterminate(),
            Some(true),
            "an unknown size painted a determinate bar"
        );
        assert_eq!(window.detail_progress_text().as_deref(), Some("0 B"));
        assert!(window.row_progress_indeterminate());

        // A yt-dlp style event: bytes moved, the total is still unknown (`null`).
        fake.push_download_event(
            1,
            "task_progress",
            "media-1",
            json!({
                "downloaded_bytes": 5_000_000u64,
                "total_bytes": null,
                "speed_bps": 1_000_000u64,
                "eta_seconds": null,
                "segments": [],
                "media_kind": "video"
            }),
        );
        assert!(
            pump_until(
                || window.detail_progress_text().as_deref() == Some("5.0 MB"),
                Duration::from_secs(5)
            ),
            "the detail bar painted {:?} as text",
            window.detail_progress_text()
        );
        assert_eq!(window.detail_progress_indeterminate(), Some(true));
        assert_eq!(
            window.detail_progress_fraction(),
            Some(0.0),
            "an unknown size must not invent a fraction"
        );
        assert_eq!(
            window.detail_percent_label().as_deref(),
            Some("5.0 MB"),
            "the percent chip still lies about a size it does not know"
        );

        fake.push_download_event(
            2,
            "task_progress",
            "media-1",
            json!({
                "downloaded_bytes": 12_000_000u64,
                "total_bytes": null,
                "speed_bps": 1_500_000u64,
                "eta_seconds": null,
                "segments": [],
                "media_kind": "video"
            }),
        );
        assert!(
            pump_until(
                || window.detail_progress_text().as_deref() == Some("12.0 MB"),
                Duration::from_secs(5)
            ),
            "the detail bar text stayed at {:?}",
            window.detail_progress_text()
        );
        assert_eq!(
            window.detail_progress_indeterminate(),
            Some(true),
            "an unknown size must stay in pulse mode"
        );
    });
}

#[test]
fn torrent_progress_is_indeterminate_at_zero_and_settles_on_a_known_total() {
    run_gui("torrent_progress_zero_total", || {
        let app = app("io.github.wilkin.Segmenta.TestTorrentBar");
        let mut row = media_row("torrent-1", "torrent");
        // A torrent before metadata reports the total as `0`, not as `null`.
        row["total_bytes"] = json!(0u64);
        let fake = DemoClient::with_downloads(vec![row]);
        let window = window_with(&app, &fake);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));
        assert!(window.click_task("torrent-1"));
        assert!(pump_until(
            || window.detail().is_some(),
            Duration::from_secs(5)
        ));
        assert_eq!(
            window.detail_progress_indeterminate(),
            Some(true),
            "total_bytes 0 painted a determinate bar"
        );

        fake.push_download_event(
            1,
            "task_progress",
            "torrent-1",
            json!({
                "downloaded_bytes": 6u64,
                "total_bytes": 12u64,
                "speed_bps": 0u64,
                "eta_seconds": null,
                "segments": []
            }),
        );
        assert!(
            pump_until(
                || window.detail_progress_fraction() == Some(0.5),
                Duration::from_secs(5)
            ),
            "the torrent bar painted {:?}",
            window.detail_progress_fraction()
        );
        assert_eq!(window.detail_progress_indeterminate(), Some(false));
        assert_eq!(window.detail_progress_text(), None);
    });
}

#[test]
fn a_progress_event_without_a_total_keeps_the_size_from_the_item() {
    run_gui("progress_event_without_total", || {
        let app = app("io.github.wilkin.Segmenta.TestEventWithoutTotal");
        let fake = DemoClient::with_downloads(vec![download_row("task-1", "downloading", 4)]);
        let window = window_with(&app, &fake);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));
        assert!(window.click_task("task-1"));
        assert!(pump_until(
            || window.detail().is_some(),
            Duration::from_secs(5)
        ));

        // `task_progress` without `total_bytes`: the size the list already knows wins.
        fake.push_download_event(
            1,
            "task_progress",
            "task-1",
            json!({
                "downloaded_bytes": 2_000_000_000u64,
                "speed_bps": 12_000_000u64,
                "eta_seconds": 160u64,
                "segments": segments(4, 4_000_000_000u64)
            }),
        );
        assert!(
            pump_until(
                || window.detail_progress_fraction() == Some(0.5),
                Duration::from_secs(5)
            ),
            "the detail bar painted {:?} instead of the item total",
            window.detail_progress_fraction()
        );
        assert_eq!(window.detail_progress_indeterminate(), Some(false));
        assert_eq!(window.detail_percent_label().as_deref(), Some("50 %"));
    });
}

#[test]
fn detail_page_shows_one_block_per_segment() {
    run_gui("detail_page_shows_one_block_per_segment", || {
        let app = app("io.github.wilkin.Segmenta.TestSegments");
        let fake = DemoClient::with_downloads(vec![download_row("task-1", "downloading", 4)]);
        let window = window_with(&app, &fake);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));

        assert!(window.click_task("task-1"), "task-1 is not selectable");
        assert!(
            pump_until(
                || window.detail_segment_count() == Some(4),
                Duration::from_secs(5)
            ),
            "the segment view reported {:?}",
            window.detail_segment_count()
        );
        let detail = window.detail().expect("detail page for the selected row");
        assert_eq!(detail.task_id, "task-1");
        assert!(detail.status.can_pause());
        assert!(!detail.status.can_resume());
    });
}

#[test]
fn seq_gap_forces_a_reload_without_duplicating_rows() {
    run_gui("seq_gap_forces_a_reload_without_duplicating_rows", || {
        let app = app("io.github.wilkin.Segmenta.TestGap");
        let fake = DemoClient::with_downloads(vec![download_row("task-1", "downloading", 4)]);
        let window = window_with(&app, &fake);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));
        let initial_calls = fake.list_calls();
        assert!(initial_calls >= 1, "the first list never happened");

        // seq 1 is in order, seq 7 skips the sequence: the client must reload.
        fake.push_download_event(
            1,
            "task_progress",
            "task-1",
            json!({
                "downloaded_bytes": 1_300_000_000u64,
                "total_bytes": 4_000_000_000u64,
                "speed_bps": 10_000_000u64,
                "eta_seconds": 100u64,
                "segments": segments(4, 4_000_000_000u64)
            }),
        );
        assert!(pump_until(
            || fake.list_calls() >= initial_calls
                && window.rows().first().map(|row| row.speed_bps) == Some(10_000_000),
            Duration::from_secs(5)
        ));

        fake.push_download_event(
            7,
            "task_progress",
            "task-1",
            json!({
                "downloaded_bytes": 3_000_000_000u64,
                "total_bytes": 4_000_000_000u64,
                "speed_bps": 20_000_000u64,
                "eta_seconds": 50u64,
                "segments": segments(4, 4_000_000_000u64)
            }),
        );
        assert!(
            pump_until(|| fake.list_calls() > initial_calls, Duration::from_secs(5)),
            "a seq_id gap did not trigger a download.list reload ({} calls)",
            fake.list_calls()
        );
        // The reload replaces the snapshot: the row is never duplicated.
        assert_eq!(window.rows().len(), 1);
        assert_eq!(window.rows()[0].task_id, "task-1");
    });
}

#[test]
fn empty_and_error_states_are_distinct() {
    run_gui("empty_and_error_states_are_distinct", || {
        let empty_app = app("io.github.wilkin.Segmenta.TestEmpty");
        let empty_fake = DemoClient::empty();
        let empty_window = window_with(&empty_app, &empty_fake);
        assert!(
            pump_until(
                || empty_window.view_state() == ViewState::Empty,
                Duration::from_secs(5)
            ),
            "empty daemon answer produced {:?}",
            empty_window.view_state()
        );

        let error_app = app("io.github.wilkin.Segmenta.TestError");
        let error_fake = DemoClient::failing("No such file or directory (os error 2)");
        let error_window = window_with(&error_app, &error_fake);
        assert!(
            pump_until(
                || matches!(error_window.view_state(), ViewState::Error(_)),
                Duration::from_secs(5)
            ),
            "unreachable daemon produced {:?}",
            error_window.view_state()
        );

        let message = match error_window.view_state() {
            ViewState::Error(message) => message,
            other => panic!("expected the error state, got {other:?}"),
        };
        assert!(!message.is_empty());
        assert!(message.to_lowercase().contains("daemon"), "{message}");
        assert_ne!(empty_window.view_state(), error_window.view_state());
        assert_eq!(empty_window.rows().len(), 0);
        assert_eq!(error_window.rows().len(), 0);

        // Filters stay usable without rows.
        empty_window.set_filter(Filter::Active);
        assert_eq!(empty_window.filter(), Filter::Active);
    });
}

#[test]
fn antivirus_page_paints_the_values_the_daemon_sent() {
    run_gui("antivirus_page_paints_the_values", || {
        let app = app("io.github.wilkin.Segmenta.TestAntivirusPrefs");
        let parent = prefs_parent(&app);
        let settings = antivirus_settings();
        let (_, save) = recorded_patches();
        let page = prefs::antivirus_page(&parent, &settings, save);

        assert!(
            page.enabled.is_active(),
            "the switch ignored `antivirus_enabled: true`"
        );
        assert_eq!(
            page.timeout.value(),
            300.0,
            "the timeout row ignored `antivirus_timeout_secs`"
        );
        assert_eq!(
            page.quarantine.subtitle().as_deref(),
            Some(ANTIVIRUS_QUARANTINE_DIR),
            "the folder row ignored `antivirus_quarantine_dir`"
        );
        assert!(
            !page.page.title().is_empty(),
            "the page has no title from the catalog"
        );
        // The explanation of the external engine rides on the group, and it must be a
        // real sentence: the caveat of the switch is not a tooltip someone can miss.
        let hint = page.group.description().unwrap_or_default().to_string();
        assert!(
            hint.len() > 60,
            "the antivirus explanation is missing: `{hint}`"
        );
        assert_eq!(page.group.title().as_str(), page.page.title().as_str());
    });
}

#[test]
fn antivirus_controls_save_the_values_they_show() {
    run_gui("antivirus_controls_save", || {
        let app = app("io.github.wilkin.Segmenta.TestAntivirusSave");
        let parent = prefs_parent(&app);
        let settings = antivirus_settings();
        let (log, save) = recorded_patches();
        let page = prefs::antivirus_page(&parent, &settings, save);

        page.enabled.set_active(false);
        let last = log
            .borrow()
            .last()
            .cloned()
            .expect("the switch saved nothing");
        assert_eq!(last, json!({ "antivirus_enabled": false }));

        page.timeout.set_value(45.0);
        let last = log
            .borrow()
            .last()
            .cloned()
            .expect("the timeout saved nothing");
        assert_eq!(last, json!({ "antivirus_timeout_secs": 45 }));

        // The row keeps the daemon's real bounds (1–3600 s), not a UI invention.
        page.timeout.set_value(0.0);
        assert_eq!(page.timeout.value(), 1.0, "the timeout accepted 0 s");
        page.timeout.set_value(7_200.0);
        assert_eq!(page.timeout.value(), 3600.0, "the timeout accepted 2 h");
        let last = log
            .borrow()
            .last()
            .cloned()
            .expect("the clamped timeout saved nothing");
        assert_eq!(last, json!({ "antivirus_timeout_secs": 3600 }));
    });
}

#[test]
fn quarantined_task_paints_the_state_and_the_quarantine_path() {
    run_gui("quarantined_task_paints_the_state", || {
        let app = app("io.github.wilkin.Segmenta.TestQuarantine");
        let fake = DemoClient::with_downloads(vec![quarantined_row()]);
        let window = window_with(&app, &fake);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));
        assert!(window.click_task("infected"), "the task is not selectable");
        assert!(
            pump_until(
                || window.detail_state().is_some() && window.detail_quarantine().is_some(),
                Duration::from_secs(5)
            ),
            "the quarantined detail painted no state (state {:?})",
            window.view_state()
        );

        let state = window.detail_state().expect("the state pill");
        assert!(
            QUARANTINE_LABELS.contains(&state.as_str()),
            "the quarantined task is painted as `{state}`"
        );
        let path = window.detail_path().expect("the path label");
        assert_eq!(path, QUARANTINED_FILE, "the detail painted `{path}`");
        assert!(
            !path.contains("Descargas"),
            "the detail still claims the file is in Downloads: {path}"
        );
        let notice = window.detail_quarantine().expect("the quarantine notice");
        assert!(
            notice.contains(QUARANTINED_FILE),
            "the notice hides the quarantine path: {notice}"
        );
        // Opening the Downloads folder would point at a file the daemon moved away.
        let actions = window.detail_actions();
        assert!(
            !actions.contains(&"open_folder".to_string()),
            "a quarantined task offers open_folder: {actions:?}"
        );
    });
}

#[test]
fn missing_engine_failure_is_not_painted_as_quarantine() {
    run_gui("missing_engine_is_not_quarantine", || {
        let app = app("io.github.wilkin.Segmenta.TestAntivirusUnavailable");
        let mut row = download_row("no-engine", "failed", 1);
        row["error_code"] = json!(ERR_ANTIVIRUS_UNAVAILABLE);
        row["error_message"] =
            json!("ERR_ANTIVIRUS_UNAVAILABLE: neither clamdscan nor clamscan is available");
        let fake = DemoClient::with_downloads(vec![row]);
        let window = window_with(&app, &fake);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));
        assert!(window.click_task("no-engine"));
        assert!(
            pump_until(|| window.detail_state().is_some(), Duration::from_secs(5)),
            "the failed detail painted no state (state {:?})",
            window.view_state()
        );

        // 1047 is the engine, not an infection: no quarantine label and no path.
        let state = window.detail_state().expect("the state pill");
        assert!(
            !QUARANTINE_LABELS.contains(&state.as_str()),
            "a missing engine is painted as `{state}`"
        );
        assert_eq!(window.detail_quarantine(), None);
        assert_eq!(
            window.detail_path().as_deref(),
            Some("/home/user/Descargas/ubuntu-26.04-desktop-amd64.iso")
        );
    });
}

// --------------------------------------------------------------- Browsers (E1)

/// Prepared folder `extension.prepare` leaves behind.
const PREPARED_EXTENSION: &str = "/home/user/.local/share/segmenta/extensions/chromium-0.2.0";

/// `extension.status` with the three states the page must tell apart: a snap
/// browser that cannot run the host, the manifest without the extension, and the
/// extension already present in a profile.
fn browser_status() -> Value {
    json!({
        "extension_id": "kdeldnifdbpemnhfpmhonlnibbhnoaan",
        "extension_version": "0.2.0",
        "browsers": [
            {
                "id": "chromium",
                "name": "Chromium",
                "family": "chromium",
                "command": "/snap/bin/chromium",
                "version": "Chromium 120.0",
                "unsupported": "snap",
                "manifest_present": true,
                "extension_installed": false,
                "prepared_path": PREPARED_EXTENSION,
                "state": "unsupported"
            },
            {
                "id": "chrome",
                "name": "Google Chrome",
                "family": "chromium",
                "command": "/usr/bin/google-chrome",
                "version": "Google Chrome 120.0",
                "unsupported": null,
                "manifest_present": true,
                "extension_installed": false,
                "prepared_path": PREPARED_EXTENSION,
                "state": "native-ready"
            },
            {
                "id": "firefox",
                "name": "Firefox",
                "family": "firefox",
                "command": "/usr/bin/firefox",
                "version": "Firefox 128.0",
                "unsupported": null,
                "manifest_present": true,
                "extension_installed": true,
                "prepared_path": null,
                "state": "extension-installed"
            }
        ]
    })
}

/// Builds the Navegadores page with recording closures instead of an IPC client.
fn browser_page(log: &Rc<RefCell<Vec<(&'static str, String)>>>) -> prefs::BrowsersPage {
    fn record(
        log: &Rc<RefCell<Vec<(&'static str, String)>>>,
        kind: &'static str,
    ) -> Rc<dyn Fn(&str)> {
        let log = Rc::clone(log);
        Rc::new(move |value: &str| log.borrow_mut().push((kind, value.to_string())))
    }
    prefs::browsers_page(
        segmenta_i18n::Locale::Es,
        &browser_status(),
        prefs::BrowserHandlers {
            prepare: record(log, "prepare"),
            open: record(log, "open"),
            copy_path: record(log, "copy"),
            install_native: {
                let log = Rc::clone(log);
                Rc::new(move || log.borrow_mut().push(("install", String::new())))
            },
        },
    )
}

#[test]
fn browsers_page_paints_the_three_states_and_wires_the_three_actions() {
    run_gui("browsers_page_states", || {
        let log: Rc<RefCell<Vec<(&'static str, String)>>> = Rc::new(RefCell::new(Vec::new()));
        let page = browser_page(&log);
        assert_eq!(page.rows.len(), 3, "the page lists the detected browsers");

        let view = |id: &str| {
            page.rows
                .iter()
                .find(|view| view.data.id == id)
                .unwrap_or_else(|| panic!("`{id}` has no row"))
        };
        let unsupported = view("chromium")
            .row
            .subtitle()
            .unwrap_or_default()
            .to_string();
        let manifest_only = view("chrome")
            .row
            .subtitle()
            .unwrap_or_default()
            .to_string();
        let installed = view("firefox")
            .row
            .subtitle()
            .unwrap_or_default()
            .to_string();
        assert_ne!(unsupported, manifest_only, "snap and manifest-only row");
        assert_ne!(manifest_only, installed, "manifest-only and installed row");
        assert_ne!(unsupported, installed, "snap and installed row");
        assert!(
            manifest_only.contains("Falta cargar la extensión"),
            "a browser without the extension is not stated as such: {manifest_only}"
        );

        // A sandboxed browser offers no action that could only fail.
        let snap = view("chromium");
        assert!(
            snap.primary_button.is_none(),
            "a snap browser offers primary button"
        );
        assert!(
            snap.menu_button.is_none(),
            "a snap browser offers menu button"
        );

        // The usable browser offers the three actions of the contract:
        // prepare is primary; open and copy_path are in the menu.
        let chrome = view("chrome");
        chrome
            .primary_button
            .as_ref()
            .expect("no prepare primary button")
            .emit_clicked();
        chrome.activate_menu_action("open");
        chrome.activate_menu_action("copy-path");
        assert_eq!(
            *log.borrow(),
            vec![
                ("prepare", "chromium".to_string()),
                ("open", "chrome".to_string()),
                ("copy", PREPARED_EXTENSION.to_string()),
            ]
        );

        // Firefox has no prepared folder yet: copy_path is not in menu.
        assert!(!view("firefox")
            .menu_labels()
            .contains(&"Copiar ruta".to_string()));

        // The how-to is one short list: no per-family manual any more.
        assert_eq!(page.steps.len(), 3, "the how-to must stay three steps");
        assert!(
            page.steps
                .iter()
                .any(|step| step.title().contains("página de extensiones")),
            "the extensions page is never named"
        );

        // One dim line points at the archives instead of two warning paragraphs.
        assert_eq!(page.hints.len(), 1, "one note, not a wall of warnings");
        assert!(
            page.hints[0]
                .label()
                .contains("/usr/share/segmenta/extensions"),
            "the note does not point at the archives"
        );
    });
}

/// A browser row must never collapse its title into a vertical column.
///
/// The X11 capture showed "Google Chrome · Manifest nativo…" one character per line with
/// two huge empty suffix buttons next to it: the suffixes were asking for more width than
/// the row had, so the title was squeezed to its minimum.
#[test]
fn browser_row_suffixes_leave_room_for_the_title() {
    run_gui("browser_rows_keep_the_title", || {
        let application = app("io.github.wilkin.Segmenta.BrowserRows");
        let status = json!({
            "extension_id": "kdeldnifdbpemnhfpmhonlnibbhnoaan",
            "firefox_extension_id": "segmenta@wilkin.github.io",
            "host_path": "/usr/bin/segmenta-host",
            "extension_version": "0.2.1",
            "browsers": [{
                "id": "chrome", "name": "Google Chrome", "family": "chromium",
                "command": "/usr/bin/google-chrome", "version": "Chrome 120.0",
                "unsupported": null, "manifest_path": "/etc/opt/chrome/x.json",
                "manifest_present": true, "user_manifest_present": false,
                "extension_installed": false, "prepared_path": null,
                "archive_path": null, "state": "native-ready"
            }]
        });
        let page = segmenta_gui::prefs::browsers_page(
            segmenta_gui::prefs::locale(),
            &status,
            segmenta_gui::prefs::BrowserHandlers {
                prepare: Rc::new(|_| {}),
                open: Rc::new(|_| {}),
                install_native: Rc::new(|| {}),
                copy_path: Rc::new(|_| {}),
            },
        );
        let window = adw::ApplicationWindow::builder()
            .application(&application)
            .build();
        window.set_default_size(960, 760);
        let page_widget: gtk::Widget = page.page.clone().upcast();
        window.set_content(Some(&page_widget));
        window.present();
        assert!(pump_until(
            || page.rows.first().map(|row| row.row.height()).unwrap_or(0) > 0,
            Duration::from_secs(5)
        ));
        let row = &page.rows[0].row;
        // The title keeps a usable width: with one character per line it was ~10 px.
        assert!(
            row.title_lines() == 1,
            "the title must not wrap into a column"
        );
        assert!(
            row.height() < 120,
            "the row collapsed into a tall vertical title: {} px",
            row.height()
        );
    });
}

/// 0.2.2: a loaded extension that is not the shipped one must be a warning with a
/// reload action, because a stale copy is exactly what kept the dead
/// `segmenta-add-torrent` menu alive in Chrome.
#[test]
fn an_out_of_sync_extension_row_warns_and_offers_reload() {
    run_gui("browsers_page_version_sync", || {
        let log: Rc<RefCell<Vec<(&'static str, String)>>> = Rc::new(RefCell::new(Vec::new()));
        let mut status = browser_status();
        status["browsers"][1]["prepared_version"] = json!("0.2.1");
        status["browsers"][1]["shipped_version"] = json!("0.2.2");
        status["browsers"][1]["version_sync"] = json!("mismatch");
        let page = prefs::browsers_page(
            segmenta_i18n::Locale::Es,
            &status,
            prefs::BrowserHandlers {
                prepare: {
                    let log = Rc::clone(&log);
                    Rc::new(move |value: &str| {
                        log.borrow_mut().push(("prepare", value.to_string()))
                    })
                },
                open: {
                    let log = Rc::clone(&log);
                    Rc::new(move |value: &str| log.borrow_mut().push(("open", value.to_string())))
                },
                copy_path: {
                    let log = Rc::clone(&log);
                    Rc::new(move |value: &str| log.borrow_mut().push(("copy", value.to_string())))
                },
                install_native: Rc::new(|| {}),
            },
        );
        let row = page
            .rows
            .iter()
            .find(|view| view.data.id == "chrome")
            .expect("chrome has no row");
        let subtitle = row.row.subtitle().unwrap_or_default().to_string();
        assert!(
            subtitle.contains("0.2.1") && subtitle.contains("0.2.2"),
            "the row does not name both versions: {subtitle}"
        );
        assert!(
            subtitle.contains("Atención"),
            "the out-of-sync row is not a warning: {subtitle}"
        );
        let reload = row.primary_button.as_ref().expect("no reload button");
        assert_eq!(
            reload.label().map(|text| text.to_string()).as_deref(),
            Some("Recargar"),
            "the out-of-sync row does not offer a reload"
        );
        reload.emit_clicked();
        assert_eq!(*log.borrow(), vec![("open", "chrome".to_string())]);

        // A matching version stays a plain prepare action as primary button.
        let matched = prefs::browsers_page(
            segmenta_i18n::Locale::Es,
            &browser_status(),
            prefs::BrowserHandlers {
                prepare: Rc::new(|_: &str| {}),
                open: Rc::new(|_: &str| {}),
                copy_path: Rc::new(|_: &str| {}),
                install_native: Rc::new(|| {}),
            },
        );
        let chrome = matched
            .rows
            .iter()
            .find(|view| view.data.id == "chrome")
            .expect("chrome has no row");
        assert_eq!(
            chrome
                .primary_button
                .as_ref()
                .unwrap()
                .label()
                .map(|t| t.to_string()),
            Some("Preparar".to_string())
        );
    });
}

#[test]
fn add_dialog_produces_request_with_scheduled_at_when_switch_on_and_without_when_off() {
    run_gui("add_dialog_schedule", || {
        let defaults = segmenta_gui::add_dialog::AddDefaults {
            destination_dir: "/home/user/Downloads".to_string(),
            connections: 8,
            clip_monitor_enabled: false,
        };
        let form = segmenta_gui::add_dialog::build_form(&defaults);
        form.url.set_text("https://example.com/test.bin");

        // When schedule switch is off: no scheduled_at
        assert!(!form.schedule.is_active());
        let params_off = segmenta_gui::add_dialog::build_add_params(&form);
        assert!(params_off.get("scheduled_at").is_none());
        assert_eq!(params_off["start_immediately"], true);

        // When schedule switch is on: scheduled_at is present
        form.schedule.set_active(true);
        assert!(form.schedule.is_active());
        let params_on = segmenta_gui::add_dialog::build_add_params(&form);
        assert!(params_on.get("scheduled_at").is_some());
        assert_eq!(params_on["start_immediately"], false);
        let scheduled_at = params_on["scheduled_at"].as_str().unwrap();
        assert!(!scheduled_at.is_empty());
    });
}

#[test]
fn browser_row_has_at_most_one_plain_button_and_one_menu_button_suffix() {
    run_gui("browser_row_suffixes_count", || {
        let log: Rc<RefCell<Vec<(&'static str, String)>>> = Rc::new(RefCell::new(Vec::new()));
        let page = browser_page(&log);
        for view in &page.rows {
            let mut plain_buttons = 0;
            let mut menu_buttons = 0;
            let mut child = view.row.first_child();
            while let Some(c) = child {
                count_button_types(&c, &mut plain_buttons, &mut menu_buttons);
                child = c.next_sibling();
            }
            assert!(
                plain_buttons <= 1,
                "browser `{}` has {} plain action buttons (must be <= 1)",
                view.data.id,
                plain_buttons
            );
            assert!(
                menu_buttons <= 1,
                "browser `{}` has {} menu buttons (must be <= 1)",
                view.data.id,
                menu_buttons
            );
        }

        // For a browser with the extension prepared (chrome in browser_page):
        let chrome = page
            .rows
            .iter()
            .find(|v| v.data.id == "chrome")
            .expect("chrome row");
        assert_eq!(chrome.menu_labels(), vec!["Abrir", "Copiar ruta"]);
    });
}

fn count_button_types(widget: &gtk::Widget, plain_buttons: &mut usize, menu_buttons: &mut usize) {
    if widget.is::<gtk::MenuButton>() {
        *menu_buttons += 1;
        return;
    } else if widget.is::<gtk::Button>() {
        *plain_buttons += 1;
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        count_button_types(&c, plain_buttons, menu_buttons);
        child = c.next_sibling();
    }
}

#[test]
fn add_dialog_schedule_progressive_picker_and_future_moment_guarantee() {
    run_gui("add_dialog_schedule_progressive", || {
        let defaults = segmenta_gui::add_dialog::AddDefaults {
            destination_dir: "/home/user/Downloads".to_string(),
            connections: 8,
            clip_monitor_enabled: false,
        };
        let form = segmenta_gui::add_dialog::build_form(&defaults);
        form.url.set_text("https://example.com/test.bin");

        // 1. Schedule off -> start_immediately: true, no scheduled_at key
        assert!(!form.schedule.is_active());
        let params_off = segmenta_gui::add_dialog::build_add_params(&form);
        assert_eq!(params_off["start_immediately"], true);
        assert!(params_off.get("scheduled_at").is_none());

        // 2. Schedule on: quick option "En 1 hora" -> scheduled_at between now+55min and now+65min
        form.schedule.set_active(true);
        assert!(form.schedule.is_active());
        assert_eq!(form.schedule_when.selected(), 1);
        let params_1h = segmenta_gui::add_dialog::build_add_params(&form);
        assert_eq!(params_1h["start_immediately"], false);
        let scheduled_at_str = params_1h["scheduled_at"].as_str().expect("scheduled_at");
        let scheduled_dt = glib::DateTime::from_iso8601(scheduled_at_str, None).expect("iso8601");
        let now_utc = glib::DateTime::now_utc().expect("now_utc");
        let diff_secs = scheduled_dt.to_unix() - now_utc.to_unix();
        assert!(
            (55 * 60..=65 * 60).contains(&diff_secs),
            "scheduled_at diff {diff_secs}s is not between 55min and 65min"
        );

        // 3. Selecting "Fecha y hora exacta…" reveals the calendar and hides it again
        form.schedule_when.set_selected(5);
        assert!(form.days_ahead.is_visible());
        assert!(form.schedule_hour.is_visible());
        assert!(form.schedule_minute.is_visible());

        form.schedule_when.set_selected(1);
        assert!(!form.days_ahead.is_visible());
        assert!(!form.schedule_hour.is_visible());
        assert!(!form.schedule_minute.is_visible());

        // 4. Summary row text changes with selection
        form.schedule_when.set_selected(0);
        let text_0 = form.schedule_summary.title().to_string();
        form.schedule_when.set_selected(1);
        let text_1 = form.schedule_summary.title().to_string();
        assert_ne!(
            text_0, text_1,
            "summary row did not change between 30min and 1h"
        );

        // 5. Exact mode with today and a time already past -> the dialog snaps it forward,
        // and there is no control that can express an earlier day any more.
        form.schedule_when.set_selected(5);
        form.days_ahead.set_value(0.0);
        form.schedule_hour.set_value(0.0);
        form.schedule_minute.set_value(0.0);
        let params_past = segmenta_gui::add_dialog::build_add_params(&form);
        let scheduled_past_str = params_past["scheduled_at"].as_str().expect("scheduled_at");
        let past_resolved =
            glib::DateTime::from_iso8601(scheduled_past_str, None).expect("iso8601");
        let now = glib::DateTime::now_utc().expect("now_utc");
        assert!(
            past_resolved.to_unix() > now.to_unix(),
            "built params must carry a future moment even with past date selected"
        );
    });
}

/// The dialog must not offer any way to schedule the past: with an offset in days the
/// earliest moment is today, and a past time today is snapped forward in the widgets
/// themselves, so what the summary says is what gets sent.
#[test]
fn the_schedule_picker_cannot_express_the_past() {
    run_gui("schedule_picker_rejects_past", || {
        let form = segmenta_gui::add_dialog::build_form(&segmenta_gui::add_dialog::AddDefaults {
            destination_dir: "/tmp".to_string(),
            connections: 8,
            clip_monitor_enabled: false,
        });
        form.schedule.set_active(true);
        form.schedule_when.set_selected(5);
        assert_eq!(form.days_ahead.value(), 0.0, "default offset is today");
        assert!(
            form.days_ahead.value() >= 0.0,
            "the offset is never negative, so the day can never be earlier than today"
        );

        // A time already gone today is clamped in the widgets, not only in the payload.
        form.schedule_hour.set_value(0.0);
        form.schedule_minute.set_value(0.0);
        let hour_after = form.schedule_hour.value() as u32;
        let minute_after = form.schedule_minute.value() as u32;
        let now = glib::DateTime::now_local().expect("now_local");
        assert!(
            (hour_after as i32, minute_after as i32) >= (now.hour(), now.minute()),
            "the widgets still show a past time: {hour_after}:{minute_after}"
        );

        let params = segmenta_gui::add_dialog::build_add_params(&form);
        let scheduled = params["scheduled_at"].as_str().expect("scheduled_at");
        let moment = glib::DateTime::from_iso8601(scheduled, None).expect("iso8601");
        assert!(
            moment.to_unix() > glib::DateTime::now_utc().expect("now").to_unix(),
            "the payload carries a past moment: {scheduled}"
        );
    });
}

#[test]
fn window_close_should_quit_policy_reflects_tray_presence() {
    // When the tray is active, closing the window hides it and keeps the process running (S6).
    assert!(!segmenta_gui::window::close_should_quit(true));
    // When the tray is not active, closing the window terminates the application as usual.
    assert!(segmenta_gui::window::close_should_quit(false));
}

#[test]
fn tray_pause_command_acts_on_current_active_task_or_noop() {
    run_gui(
        "tray_pause_command_acts_on_current_active_task_or_noop",
        || {
            let app_active = app("io.github.wilkin.Segmenta.TestTrayPause");

            // 1. With an active downloading task in the model: handler issues download.pause for task-active
            let fake =
                DemoClient::with_downloads(vec![download_row("task-active", "downloading", 1)]);
            let window = window_with(&app_active, &fake);

            assert!(
                pump_until(|| window.rows().len() == 1, Duration::from_secs(5)),
                "the listed download never became a row"
            );

            window.handle_tray_command(TrayCommand::Pause);

            assert!(
                pump_until(
                    || !fake.calls_to("download.pause").is_empty(),
                    Duration::from_secs(5)
                ),
                "download.pause was never issued for the active task"
            );
            let pause_calls = fake.calls_to("download.pause");
            assert_eq!(pause_calls.len(), 1, "expected 1 download.pause call");
            assert_eq!(pause_calls[0]["task_id"], "task-active");

            // 2. With no active task (empty model): handler issues nothing and does not error
            let app_empty = app("io.github.wilkin.Segmenta.TestTrayPauseEmpty");
            let fake_empty = DemoClient::empty();
            let window_empty = window_with(&app_empty, &fake_empty);

            assert!(
                pump_until(
                    || window_empty.view_state() == ViewState::Empty,
                    Duration::from_secs(5)
                ),
                "empty window never reached ViewState::Empty"
            );

            window_empty.handle_tray_command(TrayCommand::Pause);
            let _ = pump_until(|| false, Duration::from_millis(200));
            assert!(
                fake_empty.calls_to("download.pause").is_empty(),
                "download.pause should not be called when model is empty"
            );

            // 3. With only completed tasks (no active task): handler issues nothing and does not error
            let app_completed = app("io.github.wilkin.Segmenta.TestTrayPauseCompleted");
            let fake_completed =
                DemoClient::with_downloads(vec![download_row("task-done", "completed", 1)]);
            let window_completed = window_with(&app_completed, &fake_completed);

            assert!(
                pump_until(
                    || window_completed.rows().len() == 1,
                    Duration::from_secs(5)
                ),
                "completed download never became a row"
            );

            window_completed.handle_tray_command(TrayCommand::Pause);
            let _ = pump_until(|| false, Duration::from_millis(200));
            assert!(
                fake_completed.calls_to("download.pause").is_empty(),
                "download.pause should not be called when task is completed"
            );
        },
    );
}

#[test]
fn the_search_toggle_reveals_the_bar() {
    run_gui("the_search_toggle_reveals_the_bar", || {
        let app = app("io.github.wilkin.Segmenta.SearchToggle");
        let fake = DemoClient::with_downloads(vec![download_row("task-1", "downloading", 1)]);
        let window = window_with(&app, &fake);

        // Closed at startup: search mode is off, but the widget itself must stay
        // visible because GtkSearchBar reveals it from `search-mode-enabled`.
        assert!(!window.search_active());
        // The regression: `set_visible(false)` took the widget out of the layout, so the
        // toggle set the search mode and GTK never drew the bar.
        assert!(
            window.search_bar_visible(),
            "the search bar must not be hidden with set_visible(false)"
        );

        window.toggle_search();
        assert!(window.search_active());
        assert!(window.search_bar_visible());

        window.toggle_search();
        assert!(!window.search_active());
        assert!(window.search_bar_visible());
    });
}

#[test]
fn clicking_the_search_button_focuses_the_entry() {
    run_gui("clicking_the_search_button_focuses_the_entry", || {
        let app = app("io.github.wilkin.Segmenta.SearchFocus");
        let fake = DemoClient::with_downloads(vec![download_row("task-1", "downloading", 1)]);
        let window = window_with(&app, &fake);
        window.window().present();
        assert!(
            pump_until(|| window.window().is_mapped(), Duration::from_secs(3)),
            "the window never mapped"
        );

        // Closed at startup: nothing has asked the entry for the caret.
        assert_eq!(window.search_focus_attempts(), 0);

        // A real click on the magnifier, not a call to the action: this is the path the
        // user takes, and the one that used to leave the focus on the list.
        window.click_search_button();
        assert!(window.search_active(), "the click must open the bar");
        // The click path used to skip `focus_search` entirely; the request is the one
        // observable a headless display can prove, because GTK only grants the caret to a
        // widget of an active window and Xvfb has no window manager to activate it.
        assert!(
            pump_until(
                || window.search_focus_attempts() > 0,
                Duration::from_secs(3)
            ),
            "the click must move the caret into the search entry"
        );

        // The button and the action stay in sync: closing from the action updates the
        // toggle, and the focus goes back to the list.
        let attempts = window.search_focus_attempts();
        window.toggle_search();
        assert!(!window.search_active());
        assert!(!window.search_button_active());
        // Closing never asks for the caret again.
        assert_eq!(window.search_focus_attempts(), attempts);
    });
}

#[test]
fn the_header_shows_the_vpn_state_and_opens_the_preferences() {
    run_gui("vpn_badge", || {
        let app = app("io.github.wilkin.Segmenta.VpnBadge");
        let fake = DemoClient::with_downloads(vec![download_row("task-1", "downloading", 1)]);
        fake.set_vpn("none", "");
        let window = window_with(&app, &fake);

        // No interface configured: no badge at all, so nothing suggests protection.
        window.reload_vpn();
        assert!(
            pump_until(|| window.vpn_badge().is_none(), Duration::from_secs(3)),
            "without a VPN there must be no badge"
        );

        // A working interface says so, and names it.
        fake.set_vpn("active", "wg0");
        window.reload_vpn();
        assert!(
            pump_until(
                || window.vpn_badge().as_deref() == Some("VPN activa (wg0)"),
                Duration::from_secs(3)
            ),
            "unexpected badge: {:?}",
            window.vpn_badge()
        );

        // A dead interface is a warning, not a silent "protected".
        fake.set_vpn("down", "wg0");
        window.reload_vpn();
        assert!(
            pump_until(
                || window.vpn_badge().as_deref() == Some("VPN caída (wg0)"),
                Duration::from_secs(3)
            ),
            "unexpected badge: {:?}",
            window.vpn_badge()
        );
        assert_eq!(window.vpn_badge_level().as_deref(), Some("warning"));

        fake.set_vpn("missing", "wg0");
        window.reload_vpn();
        assert!(
            pump_until(
                || window.vpn_badge().as_deref() == Some("VPN ausente (wg0)"),
                Duration::from_secs(3)
            ),
            "unexpected badge: {:?}",
            window.vpn_badge()
        );

        // The badge is the shortcut to the preference that owns the interface.
        window.click_vpn_badge();
        assert!(
            pump_until(|| window.preferences_requested(), Duration::from_secs(3)),
            "the badge must open the preferences"
        );
    });
}
