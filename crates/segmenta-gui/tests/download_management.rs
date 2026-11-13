//! Safe download management (0.2.1): "Limpiar completadas" and batch delete of a
//! multi-selection.
//!
//! Every assertion is made against the daemon contract: `download.clean_completed`
//! never deletes files and `download.delete_many` always carries `delete_files`
//! explicitly. None of these tests starts a daemon.
//!
//! Run with `xvfb-run -a cargo test -p segmenta-gui --test download_management`.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use segmenta_gui::adw::prelude::*;
use segmenta_gui::client::{Client, ClientError, ClientEvent, DemoClient};
use segmenta_gui::window::SegmentaWindow;
use segmenta_gui::{adw, glib, gtk};
use segmenta_i18n::{tr, Msg};

type Job = Box<dyn FnOnce() + Send + 'static>;
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
            .name("gtk-download-management-harness".to_string())
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

fn run_gui<F>(name: &str, body: F)
where
    F: FnOnce() + Send + 'static,
{
    let harness = harness();
    if let Err(err) = &harness.init {
        eprintln!("skipping GUI test `{name}`: gtk::init() failed ({err})");
        return;
    }
    let (done, outcome): (Sender<Outcome>, Receiver<Outcome>) = channel();
    let job: Job = Box::new(move || {
        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&captured);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            *sink.lock().unwrap_or_else(|err| err.into_inner()) = Some(info.to_string());
        }));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        std::panic::set_hook(previous);
        let result = match result {
            Ok(()) => Ok(()),
            Err(_) => Err(captured
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
                .unwrap_or_else(|| "the GUI test panicked".to_string())),
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

fn pump_until_stable(mut predicate: impl FnMut() -> bool, window: Duration) -> bool {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        pump_until(|| false, Duration::from_millis(20));
    }
    predicate()
}

fn row(task_id: &str, status: &str, filename: &str) -> Value {
    json!({
        "task_id": task_id,
        "url": format!("https://example.test/{filename}"),
        "final_path": format!("/home/user/Descargas/{filename}"),
        "filename": filename,
        "status": status,
        "total_bytes": 1024u64,
        "downloaded_bytes": 512u64,
        "speed_bps": 0u64,
        "eta_seconds": null,
        "retry_count": 0u32,
        "supports_ranges": true,
        "queue_name": "main",
        "created_at": "2026-04-24T10:00:00Z",
        "completed_at": null,
        "error_code": null,
        "error_message": null,
        "segments": []
    })
}

fn settings() -> Value {
    json!({
        "max_connections": 16,
        "global_speed_limit_bps": null,
        "clip_monitor_enabled": true,
        "autostart": false,
        "download_dir": "/home/user/Descargas",
        "category_dirs": {},
        "capture_extensions": ["iso"],
        "domain_denylist": [],
        "schedule_start": null,
        "schedule_stop": null,
        "action_on_complete": "none",
        "allow_local_downloads": false,
        "max_retries": 5,
        "proxy_mode": "system",
        "proxy_url": null,
        "postprocess_command": [],
        "appearance": "system"
    })
}

fn ready_window(
    application: &adw::Application,
    fake: &DemoClient,
    expected: usize,
) -> SegmentaWindow {
    fake.set_settings(settings());
    let client: Arc<dyn Client> = Arc::new(fake.clone());
    let window = SegmentaWindow::new(application, client);
    let loaded = pump_until(|| window.rows().len() == expected, Duration::from_secs(5));
    assert!(
        loaded,
        "the window never listed the {expected} expected download(s)"
    );
    window
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

#[test]
fn clean_completed_is_offered_only_when_a_task_completed() {
    run_gui("clean_gate", || {
        let application = app("io.github.wilkin.Segmenta.Clean1");
        // No completed task: the action must be disabled instead of failing later.
        let fake = DemoClient::with_downloads(vec![row("run", "downloading", "ubuntu.iso")]);
        let window = ready_window(&application, &fake, 1);
        assert!(
            window
                .action_names()
                .iter()
                .any(|name| name == "clean-completed"),
            "the main menu must expose `clean-completed`: {:?}",
            window.action_names()
        );
        assert!(
            !window.clean_completed_enabled(),
            "clean-completed must be disabled with no completed task"
        );
        let action = window
            .window()
            .lookup_action("clean-completed")
            .expect("the clean-completed action must be registered");
        assert!(!action.is_enabled());
        // Asked anyway, it explains the state instead of pretending to work.
        window.begin_clean_completed();
        assert!(
            !window.confirm_dialog_open(),
            "with nothing completed there is nothing to confirm"
        );
        assert!(
            fake.calls_to("download.clean_completed").is_empty(),
            "with nothing completed the daemon must not be asked"
        );

        // With one completed task the action turns on.
        let fake = DemoClient::with_downloads(vec![
            row("run", "downloading", "ubuntu.iso"),
            row("done", "completed", "debian.iso"),
        ]);
        let window = ready_window(&application, &fake, 2);
        assert!(
            window.clean_completed_enabled(),
            "clean-completed must be enabled with one completed task"
        );
        let action = window
            .window()
            .lookup_action("clean-completed")
            .expect("the clean-completed action must be registered");
        assert!(action.is_enabled());
    });
}

#[test]
fn cleaning_completed_reports_the_count_and_keeps_the_files() {
    run_gui("clean_flow", || {
        let application = app("io.github.wilkin.Segmenta.Clean2");
        let fake = DemoClient::with_downloads(vec![
            row("run", "downloading", "ubuntu.iso"),
            row("one", "completed", "debian.iso"),
            row("two", "completed", "fedora.iso"),
        ]);
        let window = ready_window(&application, &fake, 3);

        // The confirmation names the count and offers no way to touch the files.
        window.begin_clean_completed();
        assert!(
            window.confirm_dialog_open(),
            "the clean confirmation was not shown"
        );
        let body = window.confirm_body().unwrap_or_default();
        assert!(
            body.contains('2'),
            "the confirmation must name the 2 tasks: {body}"
        );
        assert_eq!(
            window.confirm_delete_files(),
            None,
            "cleaning completed tasks must not offer to delete files"
        );

        window.accept_confirmation();
        assert!(
            pump_until(
                || window.last_clean_report().is_some(),
                Duration::from_secs(5)
            ),
            "the clean answer never arrived"
        );
        let report = window.last_clean_report().expect("clean report");
        assert_eq!(report.removed, 2, "both completed tasks were cleaned");
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert!(
            !window.confirm_dialog_open(),
            "the confirmation must close once the answer is known"
        );
        // ...and it must not leave the button frozen: the Ubuntu QA showed "Limpiando…"
        // disabled forever because `can_close(false)` also blocked the programmatic close.
        assert!(
            !window.confirm_waiting(),
            "the dialog was left in its pending state after the answer"
        );

        // The list was refreshed: only the running task remains.
        assert!(
            pump_until(|| window.rows().len() == 1, Duration::from_secs(5)),
            "the list was not refreshed after cleaning: {:?}",
            window.rows()
        );
        assert_eq!(window.rows()[0].task_id, "run");

        // The contract is explicit: clean never carries a delete-files flag and the
        // fake daemon never removed a file.
        let calls = fake.calls_to("download.clean_completed");
        assert_eq!(calls.len(), 1, "{calls:?}");
        for params in &calls {
            assert!(
                params.get("delete_files").is_none(),
                "clean_completed must not carry delete_files: {params}"
            );
            assert!(
                params.get("delete_file").is_none(),
                "clean_completed must not carry delete_file: {params}"
            );
        }
        assert!(
            fake.calls_to("download.delete_many").is_empty(),
            "cleaning completed tasks must never go through the batch delete"
        );
        assert_eq!(
            fake.deleted_files(),
            0,
            "cleaning completed tasks must never delete a file"
        );
    });
}

#[test]
fn multi_selection_survives_a_reload_and_the_delete_dialog_counts_files() {
    run_gui("multi_selection", || {
        let application = app("io.github.wilkin.Segmenta.Selection1");
        let fake = DemoClient::with_downloads(vec![
            row("done", "completed", "debian.iso"),
            row("failed", "failed", "fedora.iso"),
            row("run", "downloading", "ubuntu.iso"),
        ]);
        let window = ready_window(&application, &fake, 3);

        // Multi-selection is an explicit, discoverable mode, not a hidden ctrl-click.
        assert!(!window.multi_select_enabled());
        window.set_multi_select(true);
        assert!(window.multi_select_enabled());
        assert!(
            window
                .action_names()
                .iter()
                .any(|name| name == "multi-select"),
            "the menu must expose the multi-selection toggle: {:?}",
            window.action_names()
        );
        assert!(window.set_task_selected("done", true));
        assert!(window.set_task_selected("failed", true));
        assert!(
            !window.set_task_selected("ghost", true),
            "an unknown task cannot be selected"
        );
        assert_eq!(
            sorted(window.selected_task_ids()),
            vec!["done".to_string(), "failed".to_string()]
        );

        // A refresh that keeps the same tasks must keep the selection.
        let before = fake.list_calls();
        window.reload();
        assert!(
            pump_until(
                || fake.list_calls() > before && window.rows().len() == 3,
                Duration::from_secs(5)
            ),
            "the reload did not finish"
        );
        assert_eq!(
            sorted(window.selected_task_ids()),
            vec!["done".to_string(), "failed".to_string()],
            "the selection must survive a refresh of the same tasks"
        );
        assert_eq!(window.selection_file_count(), 2);

        window.begin_delete_selection();
        assert!(
            window.confirm_dialog_open(),
            "the delete confirmation was not shown"
        );
        let summary = window.confirm_summary().expect("delete summary");
        assert_eq!(summary.tasks, 2, "the dialog must name 2 tasks");
        assert_eq!(summary.files, 2, "the dialog must name 2 files");
        assert_eq!(summary.active, 0);
        let body = window.confirm_body().unwrap_or_default();
        assert!(
            body.contains('2'),
            "the dialog body must state the counts: {body}"
        );
        assert_eq!(
            window.confirm_delete_files(),
            Some(false),
            "the file checkbox starts unchecked"
        );
        assert_eq!(
            window.confirm_checkbox_label(),
            Some(tr(segmenta_i18n::current(), Msg::DeleteFilesCheckbox).to_string())
        );

        // The keyboard default is the safe one: it dismisses and sends nothing.
        window.press_confirm_default();
        assert!(
            !window.confirm_dialog_open(),
            "the default action must close the dialog"
        );
        assert!(
            fake.calls_to("download.delete_many").is_empty(),
            "the safe default must not send a delete"
        );

        // A selection with a running task says so before deleting.
        assert!(window.set_task_selected("run", true));
        window.begin_delete_selection();
        let summary = window.confirm_summary().expect("delete summary");
        assert_eq!(summary.active, 1);
        let body = window.confirm_body().unwrap_or_default();
        let notice =
            tr(segmenta_i18n::current(), Msg::DeleteSelectionActive).replace("{count}", "1");
        assert!(
            body.contains(&notice),
            "the active notice is missing: {body}"
        );
        window.press_confirm_default();
        assert!(!window.confirm_dialog_open());
    });
}

#[test]
fn the_delete_dialog_sends_delete_files_from_the_checkbox() {
    run_gui("delete_files_flag", || {
        let application = app("io.github.wilkin.Segmenta.Selection2");
        let fake = DemoClient::with_downloads(vec![
            row("a", "completed", "a.iso"),
            row("b", "completed", "b.iso"),
            row("c", "completed", "c.iso"),
        ]);
        let window = ready_window(&application, &fake, 3);
        window.set_multi_select(true);
        assert!(window.set_task_selected("a", true));
        assert!(window.set_task_selected("b", true));

        // Checkbox off: the record goes away and the file stays.
        window.begin_delete_selection();
        assert_eq!(window.confirm_delete_files(), Some(false));
        window.accept_confirmation();
        assert!(
            pump_until(
                || window.last_delete_report().is_some(),
                Duration::from_secs(5)
            ),
            "the batch answer never arrived"
        );
        let report = window.last_delete_report().expect("batch report");
        assert_eq!(report.removed, 2);
        assert_eq!(report.files_deleted, 0);
        let calls = fake.calls_to("download.delete_many");
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(calls[0]["delete_files"], json!(false));
        let ids = calls[0]["task_ids"].as_array().expect("task_ids array");
        assert_eq!(ids.len(), 2);
        for id in ["a", "b"] {
            assert!(
                ids.iter().any(|value| value == id),
                "task {id} missing from {ids:?}"
            );
        }
        assert_eq!(fake.deleted_files(), 0);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));
        assert_eq!(window.rows()[0].task_id, "c");

        // Checkbox on: the files are deleted too and the flow reports it.
        assert!(window.set_task_selected("c", true));
        window.begin_delete_selection();
        assert_eq!(
            window.confirm_delete_files(),
            Some(false),
            "every dialog starts unchecked"
        );
        window.set_confirm_delete_files(true);
        assert_eq!(window.confirm_delete_files(), Some(true));
        window.accept_confirmation();
        assert!(
            pump_until(
                || fake.calls_to("download.delete_many").len() == 2,
                Duration::from_secs(5)
            ),
            "the second batch never reached the client"
        );
        let calls = fake.calls_to("download.delete_many");
        assert_eq!(calls[1]["delete_files"], json!(true));
        assert!(
            pump_until(
                || window
                    .last_delete_report()
                    .map(|report| report.files_deleted)
                    == Some(1),
                Duration::from_secs(5)
            ),
            "the second report never arrived"
        );
        assert_eq!(fake.deleted_files(), 1);
        assert!(pump_until(
            || window.rows().is_empty(),
            Duration::from_secs(5)
        ));
    });
}

#[test]
fn a_batch_with_one_failing_task_names_the_failure() {
    run_gui("batch_failure", || {
        let application = app("io.github.wilkin.Segmenta.Selection3");
        let fake = DemoClient::with_downloads(vec![
            row("ok", "completed", "ok.iso"),
            row("broken", "completed", "broken.iso"),
        ]);
        fake.fail_task("broken", "no se pudo borrar el archivo");
        let window = ready_window(&application, &fake, 2);
        window.set_multi_select(true);
        assert!(window.set_task_selected("ok", true));
        assert!(window.set_task_selected("broken", true));

        window.begin_delete_selection();
        window.set_confirm_delete_files(true);
        window.accept_confirmation();
        assert!(
            pump_until(
                || window.last_delete_report().is_some(),
                Duration::from_secs(5)
            ),
            "the batch answer never arrived"
        );
        let report = window.last_delete_report().expect("batch report");
        assert_eq!(report.removed, 1, "only the removable task was removed");
        assert!(
            !report.is_full_success(),
            "a failing task must not be reported as full success"
        );
        let summary = report.summary(segmenta_i18n::current());
        assert!(
            summary.contains("broken"),
            "the failure is not named: {summary}"
        );
        assert!(
            summary.contains("no se pudo borrar el archivo"),
            "the exact error is missing: {summary}"
        );
        // The failed task stays listed: the GUI never pretends it is gone.
        assert!(
            window.rows().iter().any(|item| item.task_id == "broken"),
            "the failed task disappeared from the list: {:?}",
            window.rows()
        );
        assert!(!window.rows().iter().any(|item| item.task_id == "ok"));
    });
}

/// `download.delete_many` blocked until the test releases it, so "the dialog stays
/// open until the answer is known" is observed instead of assumed.
#[derive(Clone)]
struct GatedClient {
    inner: DemoClient,
    gate: Arc<(Mutex<bool>, Condvar)>,
}

impl GatedClient {
    fn release(gate: &Arc<(Mutex<bool>, Condvar)>) {
        let (lock, cvar) = &**gate;
        *lock.lock().unwrap() = true;
        cvar.notify_all();
    }
}

impl Client for GatedClient {
    fn request(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        if method == "download.delete_many" {
            let (lock, cvar) = &*self.gate;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = cvar.wait(released).unwrap();
            }
        }
        self.inner.request(method, params)
    }

    fn poll_event(&self) -> Option<ClientEvent> {
        self.inner.poll_event()
    }
}

#[test]
fn the_delete_dialog_stays_open_until_the_batch_answer_arrives() {
    run_gui("dialog_waits", || {
        let application = app("io.github.wilkin.Segmenta.Selection4");
        let fake = DemoClient::with_downloads(vec![row("a", "completed", "a.iso")]);
        fake.set_settings(settings());
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let client: Arc<dyn Client> = Arc::new(GatedClient {
            inner: fake.clone(),
            gate: Arc::clone(&gate),
        });
        let window = SegmentaWindow::new(&application, client);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));

        window.set_multi_select(true);
        assert!(window.set_task_selected("a", true));
        window.begin_delete_selection();
        assert!(window.confirm_dialog_open());
        window.accept_confirmation();

        // The answer is still on its way: the dialog must not close or claim success.
        let closed_early =
            pump_until_stable(|| !window.confirm_dialog_open(), Duration::from_millis(400));
        let open_while_waiting = window.confirm_dialog_open();
        let reported_while_waiting = window.last_delete_report().is_some();
        GatedClient::release(&gate);

        assert!(
            !closed_early && open_while_waiting,
            "the dialog closed before the answer arrived"
        );
        assert!(
            !reported_while_waiting,
            "the dialog reported a result the daemon had not sent yet"
        );
        assert!(
            pump_until(
                || window.last_delete_report().is_some(),
                Duration::from_secs(5)
            ),
            "the batch answer never arrived after the daemon was released"
        );
        assert!(
            !window.confirm_dialog_open(),
            "the dialog stayed open after the answer"
        );
    });
}
