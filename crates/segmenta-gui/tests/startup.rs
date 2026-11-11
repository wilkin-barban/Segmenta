//! Startup contract: the normal window must list a completed task in under two
//! seconds, and a missing socket must produce an actionable error whose retry really
//! reconnects. Spec S23.
//!
//! Run with `xvfb-run -a cargo test -p segmenta-gui --test startup`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use segmenta_gui::adw::prelude::*;
use segmenta_gui::client::{Client, UnixClient};
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
            .name("gtk-startup-harness".to_string())
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

fn completed_row(task_id: &str) -> Value {
    json!({
        "task_id": task_id,
        "url": "https://example.test/ubuntu.iso",
        "final_path": "/home/user/Descargas/ubuntu.iso",
        "filename": "ubuntu.iso",
        "status": "completed",
        "total_bytes": 1024u64,
        "downloaded_bytes": 1024u64,
        "speed_bps": 0u64,
        "eta_seconds": null,
        "retry_count": 0u32,
        "supports_ranges": true,
        "queue_name": "main",
        "created_at": "2026-04-24T10:00:00Z",
        "completed_at": "2026-04-24T10:01:00Z",
        "error_code": null,
        "error_message": null,
        "segments": []
    })
}

fn settings_payload() -> Value {
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
        "postprocess_command": []
    })
}

/// Minimal daemon that answers the three startup methods.
fn fake_daemon(listener: UnixListener, rows: Vec<Value>) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let rows = rows.clone();
            std::thread::spawn(move || serve(stream, rows));
        }
    });
}

fn serve(stream: UnixStream, rows: Vec<Value>) {
    let mut writer = stream.try_clone().expect("clone");
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { return };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let result = match request["method"].as_str().unwrap_or_default() {
            "system.hello" => json!({
                "protocol_version": 1,
                "daemon_version": "0.0.2",
                "min_client_version": 1
            }),
            "settings.get" => settings_payload(),
            "download.list" => Value::Array(rows.clone()),
            "download.status" => rows.first().cloned().unwrap_or(Value::Null),
            "system.status" => json!({
                "daemon_version": "0.0.2",
                "uptime_seconds": 1u64,
                "memory_bytes": 1024u64,
                "active_connections": 0u64,
                "event_subscribers": 1u64,
                "secret_backend": "keyring"
            }),
            other => {
                let error = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("unknown method {other}") }
                });
                let _ = writeln!(writer, "{error}");
                continue;
            }
        };
        let response = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        if writeln!(writer, "{response}").is_err() {
            return;
        }
    }
}

fn socket_listener(dir: &Path, name: &str) -> (PathBuf, UnixListener) {
    let path = dir.join(name);
    let listener = UnixListener::bind(&path).expect("bind the fake daemon");
    (path, listener)
}

#[test]
fn normal_startup_lists_a_completed_task_in_under_two_seconds() {
    run_gui("normal_startup", || {
        let dir = tempfile::tempdir().unwrap();
        let (path, listener) = socket_listener(dir.path(), "daemon.sock");
        fake_daemon(listener, vec![completed_row("task-1")]);

        let application = app("io.github.wilkin.Segmenta.Startup1");
        let client: Arc<dyn Client> = Arc::new(UnixClient::connect(path));
        let started = Instant::now();
        let window = SegmentaWindow::new(&application, client);
        let listed = pump_until(|| window.rows().len() == 1, Duration::from_secs(5));
        let elapsed = started.elapsed();
        assert!(listed, "the completed task never appeared");
        assert!(
            elapsed < Duration::from_secs(2),
            "startup took {elapsed:?}, expected under 2 s"
        );
        assert_eq!(window.rows()[0].filename, "ubuntu.iso");
        assert!(matches!(window.view_state(), ViewState::Ready));
    });
}

#[test]
fn a_missing_socket_shows_the_enable_command_and_retry_reconnects() {
    run_gui("missing_socket", || {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.sock");

        let application = app("io.github.wilkin.Segmenta.Startup2");
        let client: Arc<dyn Client> = Arc::new(UnixClient::connect(path.clone()));
        let window = SegmentaWindow::new(&application, client);

        let failed = pump_until(
            || matches!(window.view_state(), ViewState::Error(_)),
            Duration::from_secs(6),
        );
        assert!(failed, "a missing socket must reach the error state");
        let ViewState::Error(message) = window.view_state() else {
            unreachable!()
        };
        assert!(
            message.contains("daemon.sock"),
            "the error must carry the socket detail: {message}"
        );
        let description = window.error_description().unwrap_or_default();
        assert!(
            description.contains("systemctl --user enable --now segmenta.socket"),
            "the actionable command must be on screen: {description}"
        );

        // The daemon appears late (socket activation). Retry must reconnect and list.
        let listener = UnixListener::bind(&path).expect("bind late");
        fake_daemon(listener, vec![completed_row("late-task")]);
        window.retry();
        let listed = pump_until(|| window.rows().len() == 1, Duration::from_secs(6));
        assert!(listed, "retry did not reconnect and reload");
        assert_eq!(window.rows()[0].task_id, "late-task");
        assert!(matches!(window.view_state(), ViewState::Ready));
    });
}
