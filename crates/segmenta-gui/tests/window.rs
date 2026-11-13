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
            manifest_only.contains("NO está instalada"),
            "a browser without the extension is not stated as such: {manifest_only}"
        );

        // A sandboxed browser offers no action that could only fail.
        let snap = view("chromium");
        assert!(snap.prepare.is_none(), "a snap browser offers prepare");
        assert!(snap.open.is_none(), "a snap browser offers open");
        assert!(snap.copy_path.is_none(), "a snap browser offers copy");

        // The usable browser offers the three actions of the contract.
        let chrome = view("chrome");
        chrome
            .prepare
            .as_ref()
            .expect("no prepare button")
            .emit_clicked();
        chrome.open.as_ref().expect("no open button").emit_clicked();
        chrome
            .copy_path
            .as_ref()
            .expect("no copy button")
            .emit_clicked();
        assert_eq!(
            *log.borrow(),
            vec![
                ("prepare", "chromium".to_string()),
                ("open", "chrome".to_string()),
                ("copy", PREPARED_EXTENSION.to_string()),
            ]
        );

        // Firefox has no prepared folder yet: nothing to copy.
        assert!(view("firefox").copy_path.is_none());

        // The numbered how-to names the real pages of both families.
        assert!(
            page.steps
                .iter()
                .any(|step| step.title().contains("chrome://extensions")),
            "the Chromium page is never named"
        );
        assert!(
            page.steps
                .iter()
                .any(|step| step.title().contains("about:debugging")),
            "the Firefox page is never named"
        );

        // Both limits are on the page: a manifest is not an install, and there is
        // no store and no signature.
        assert!(
            page.hints
                .iter()
                .any(|hint| hint.label().contains("no es una instalación")),
            "the page does not say a manifest is not an installation"
        );
        assert!(
            page.hints
                .iter()
                .any(|hint| hint.label().contains("Sin tienda y sin firma")),
            "the page does not say there is no store and no signature"
        );
    });
}
