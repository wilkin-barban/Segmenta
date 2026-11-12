//! GUI tests: they need an X display, so run them under `xvfb-run -a`.
//!
//! Without a display the tests skip with a clear message instead of failing the
//! suite (`gtk::init()` reports the missing display). GTK is not thread safe and
//! `gtk::init()` refuses to run twice, so every test body is executed on one
//! dedicated harness thread that initialises the toolkit once.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use segmenta_gui::adw::prelude::*;
use segmenta_gui::client::{Client, DemoClient};
use segmenta_gui::model::{DownloadStatus, Filter};
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
