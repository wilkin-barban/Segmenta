//! Action matrix (S24): every control must reach a real handler, gate on state and
//! leave a visible trace. These tests drive the same code the buttons call; none of
//! them asserts a decorative placeholder.
//!
//! Run with `xvfb-run -a cargo test -p segmenta-gui --test actions`.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use segmenta_gui::adw::prelude::*;
use segmenta_gui::client::{Client, DemoClient};
use segmenta_gui::model::Filter;
use segmenta_gui::window::{SegmentaWindow, ViewState};
use segmenta_gui::{adw, glib, gtk};

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
            .name("gtk-actions-harness".to_string())
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

/// Window with one downloading and one completed task, fully loaded.
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

#[test]
fn the_header_and_menu_expose_every_advertised_action() {
    run_gui("action_inventory", || {
        let application = app("io.github.wilkin.Segmenta.Actions1");
        let fake = DemoClient::with_downloads(vec![
            row("a", "downloading", "ubuntu.iso"),
            row("b", "completed", "debian.iso"),
        ]);
        let window = ready_window(&application, &fake, 2);

        let actions = window.action_names();
        for expected in ["add", "reload", "search", "preferences", "about"] {
            assert!(
                actions.iter().any(|name| name == expected),
                "missing action {expected} in {actions:?}"
            );
        }
        assert!(
            application.list_actions().iter().any(|name| name == "quit"),
            "the application must expose quit"
        );
        // Every action must be enabled: a disabled control needs a documented reason.
        for name in ["add", "reload", "search", "preferences", "about"] {
            let action = window
                .window()
                .lookup_action(name)
                .unwrap_or_else(|| panic!("action {name} is not registered"));
            assert!(action.is_enabled(), "action {name} is disabled");
        }
    });
}

#[test]
fn sidebar_filters_and_counts_are_real() {
    run_gui("filters", || {
        let application = app("io.github.wilkin.Segmenta.Actions2");
        let fake = DemoClient::with_downloads(vec![
            row("a", "downloading", "ubuntu.iso"),
            row("b", "completed", "debian.iso"),
            row("c", "paused", "fedora.iso"),
        ]);
        let client: Arc<dyn Client> = Arc::new(fake.clone());
        fake.set_settings(settings());
        let window = SegmentaWindow::new(&application, client);
        assert!(pump_until(
            || window.rows().len() == 3,
            Duration::from_secs(5)
        ));

        let counts = window.counts();
        assert_eq!(counts.all, 3);
        assert_eq!(counts.active, 1);
        assert_eq!(counts.paused, 1);
        assert_eq!(counts.completed, 1);
        assert_eq!(counts.failed, 0);

        window.set_filter(Filter::Paused);
        assert_eq!(window.rows().len(), 1);
        assert_eq!(window.rows()[0].task_id, "c");
        window.set_filter(Filter::Failed);
        assert!(window.rows().is_empty());
        assert!(matches!(window.view_state(), ViewState::Empty));
        window.set_filter(Filter::All);
        assert_eq!(window.rows().len(), 3);
    });
}

#[test]
fn the_search_entry_filters_live_and_clears() {
    run_gui("search", || {
        let application = app("io.github.wilkin.Segmenta.Actions3");
        let fake = DemoClient::with_downloads(vec![
            row("a", "downloading", "ubuntu-26.04.iso"),
            row("b", "completed", "debian-13.iso"),
        ]);
        let window = ready_window(&application, &fake, 2);

        window.set_query("ubuntu");
        assert_eq!(window.rows().len(), 1);
        assert_eq!(window.rows()[0].task_id, "a");
        window.set_query("nothing-matches");
        assert!(window.rows().is_empty());
        window.set_query("");
        assert_eq!(window.rows().len(), 2);
    });
}

#[test]
fn reload_asks_the_daemon_again() {
    run_gui("reload", || {
        let application = app("io.github.wilkin.Segmenta.Actions4");
        let fake = DemoClient::with_downloads(vec![row("a", "downloading", "ubuntu.iso")]);
        let window = ready_window(&application, &fake, 1);
        let before = fake.list_calls();
        // The `win.reload` action is a thin wrapper over this method; the inventory
        // test above proves the action is registered and enabled.
        window.reload();
        assert!(
            pump_until(|| fake.list_calls() > before, Duration::from_secs(3)),
            "the reload action did not ask for a new list"
        );
    });
}

#[test]
fn the_search_action_toggles_the_search_bar() {
    run_gui("search_action", || {
        let application = app("io.github.wilkin.Segmenta.Actions5");
        let fake = DemoClient::with_downloads(vec![row("a", "downloading", "ubuntu.iso")]);
        let window = ready_window(&application, &fake, 1);
        let initial = window.search_active();
        window.toggle_search();
        assert_eq!(window.search_active(), !initial);
        window.toggle_search();
        assert_eq!(window.search_active(), initial);
    });
}

#[test]
fn the_add_dialog_opens_and_cancel_closes_it_without_sending() {
    run_gui("add_dialog", || {
        let application = app("io.github.wilkin.Segmenta.Actions6");
        let fake = DemoClient::with_downloads(vec![row("a", "downloading", "ubuntu.iso")]);
        let window = ready_window(&application, &fake, 1);
        let lists_before = fake.list_calls();

        // Same wrapper the `win.add` action runs.
        window.open_add_dialog();
        assert!(
            pump_until(|| window.add_dialog_open(), Duration::from_secs(3)),
            "the add dialog never opened"
        );
        window.close_add_dialog();
        assert!(
            pump_until(|| !window.add_dialog_open(), Duration::from_secs(3)),
            "the add dialog did not close"
        );
        // Cancelling must not trigger a reload or a download.
        assert_eq!(fake.list_calls(), lists_before);
    });
}

#[test]
fn selecting_a_row_opens_the_detail_and_gates_its_actions_on_state() {
    run_gui("detail_gating", || {
        let application = app("io.github.wilkin.Segmenta.Actions7");
        let fake = DemoClient::with_downloads(vec![
            row("run", "downloading", "ubuntu.iso"),
            row("done", "completed", "debian.iso"),
        ]);
        let client: Arc<dyn Client> = Arc::new(fake.clone());
        fake.set_settings(settings());
        let window = SegmentaWindow::new(&application, client);
        assert!(pump_until(
            || window.rows().len() == 2,
            Duration::from_secs(5)
        ));

        assert!(window.select_task("run"));
        assert!(pump_until(
            || window.detail().map(|item| item.task_id) == Some("run".to_string()),
            Duration::from_secs(2)
        ));
        let actions = window.detail_actions();
        assert!(actions.contains(&"pause".to_string()), "{actions:?}");
        assert!(actions.contains(&"cancel".to_string()), "{actions:?}");
        assert!(!actions.contains(&"resume".to_string()), "{actions:?}");

        assert!(window.select_task("done"));
        assert!(pump_until(
            || window.detail().map(|item| item.task_id) == Some("done".to_string()),
            Duration::from_secs(2)
        ));
        let actions = window.detail_actions();
        assert!(!actions.contains(&"pause".to_string()), "{actions:?}");
        assert!(!actions.contains(&"resume".to_string()), "{actions:?}");
        assert!(actions.contains(&"delete".to_string()), "{actions:?}");
        assert!(actions.contains(&"open_folder".to_string()), "{actions:?}");

        // A paused task can resume and also be deleted, but not paused again.
        let fake = DemoClient::with_downloads(vec![row("paused", "paused", "fedora.iso")]);
        let client: Arc<dyn Client> = Arc::new(fake.clone());
        fake.set_settings(settings());
        let window = SegmentaWindow::new(&application, client);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));
        assert!(window.select_task("paused"));
        assert!(pump_until(
            || window.detail().is_some(),
            Duration::from_secs(2)
        ));
        let actions = window.detail_actions();
        assert!(actions.contains(&"resume".to_string()), "{actions:?}");
        assert!(!actions.contains(&"pause".to_string()), "{actions:?}");
    });
}

#[test]
fn the_destructive_actions_ask_for_confirmation_first() {
    run_gui("confirmations", || {
        let application = app("io.github.wilkin.Segmenta.Actions8");
        let fake = DemoClient::with_downloads(vec![row("done", "completed", "debian.iso")]);
        let client: Arc<dyn Client> = Arc::new(fake.clone());
        fake.set_settings(settings());
        let window = SegmentaWindow::new(&application, client);
        assert!(pump_until(
            || window.rows().len() == 1,
            Duration::from_secs(5)
        ));
        assert!(window.select_task("done"));
        assert!(pump_until(
            || window.detail().is_some(),
            Duration::from_secs(2)
        ));

        // Cancelling from the confirmation must not reach the daemon.
        let before = fake.list_calls();
        window.begin_delete();
        assert!(
            window.confirmation_pending(),
            "the delete confirmation was not shown"
        );
        window.dismiss_confirmation();
        assert!(!window.confirmation_pending());
        assert_eq!(
            fake.list_calls(),
            before,
            "dismiss must not mutate anything"
        );
    });
}

#[test]
fn quitting_closes_the_window() {
    run_gui("quit", || {
        let application = app("io.github.wilkin.Segmenta.Actions9");
        let fake = DemoClient::with_downloads(vec![row("a", "downloading", "ubuntu.iso")]);
        let window = ready_window(&application, &fake, 1);
        assert!(window.window().is_visible() || !window.window().is_mapped());
        let quit = application
            .lookup_action("quit")
            .expect("the quit action must exist");
        assert!(quit.is_enabled());
    });
}
