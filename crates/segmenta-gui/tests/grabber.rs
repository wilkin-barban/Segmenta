//! Link Grabber dialog: paste, preview, queue — all on the toolkit thread.

use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use segmenta_gui::adw;
use segmenta_gui::adw::prelude::*;
use segmenta_gui::grabber;
use segmenta_gui::gtk;

type Job = Box<dyn FnOnce() + Send + 'static>;
type Calls = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

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
                std::env::set_var(segmenta_gui::client::TEST_GUARD_ENV, "1");
                let init = gtk::init()
                    .map_err(|err| err.to_string())
                    .and_then(|()| adw::init().map_err(|err| err.to_string()));
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
    let (done, outcome) = channel::<Result<(), String>>();
    let job: Job = Box::new(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
            .map_err(|_| "the GUI test panicked".to_string());
        let _ = done.send(result);
    });
    if harness.jobs.send(job).is_err() {
        panic!("the GTK harness thread is gone");
    }
    match outcome.recv_timeout(Duration::from_secs(60)) {
        Ok(Ok(())) => {}
        Ok(Err(message)) => panic!("{message}"),
        Err(err) => panic!("the GUI test never finished: {err}"),
    }
}

/// Pump the main loop until `ready` is true or the deadline passes.
fn pump_until(mut ready: impl FnMut() -> bool, deadline: Duration) -> bool {
    let until = std::time::Instant::now() + deadline;
    while std::time::Instant::now() < until {
        while gtk::glib::MainContext::default().iteration(false) {}
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    ready()
}

fn app() -> adw::Application {
    adw::Application::builder()
        .application_id("io.github.wilkin.Segmenta.TestGrabber")
        .flags(segmenta_gui::gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build()
}

fn dialog_with(calls: Calls) -> grabber::GrabberDialog {
    let preview_calls = Arc::clone(&calls);
    let preview: segmenta_gui::prefs::RpcCall = std::rc::Rc::new(move |method: &str, params| {
        preview_calls
            .lock()
            .unwrap()
            .push((method.to_string(), params.clone()));
        // The daemon answers; here the answer is rendered by the test.
        let _ = params;
    });
    let enqueue_calls = Arc::clone(&calls);
    let enqueue: segmenta_gui::prefs::RpcCall = std::rc::Rc::new(move |method: &str, params| {
        enqueue_calls
            .lock()
            .unwrap()
            .push((method.to_string(), params.clone()));
    });
    grabber::grabber_dialog(preview, enqueue)
}

#[test]
fn pasting_text_asks_the_daemon_for_a_preview() {
    run_gui("grabber_preview", || {
        let app = app();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let dialog = dialog_with(Arc::clone(&calls));
        dialog.window.set_application(Some(&app));

        dialog.text.buffer().set_text("https://example.test/a.bin");
        assert!(
            pump_until(|| !calls.lock().unwrap().is_empty(), Duration::from_secs(5)),
            "the dialog never asked for a preview"
        );
        let sent = calls.lock().unwrap().clone();
        assert_eq!(sent[0].0, "grabber.preview", "{sent:?}");
        assert!(sent[0].1["text"]
            .as_str()
            .unwrap_or_default()
            .contains("a.bin"));
    });
}

#[test]
fn an_answer_paints_the_list_and_queueing_adds_one_call_per_link() {
    run_gui("grabber_apply", || {
        let app = app();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let dialog = dialog_with(Arc::clone(&calls));
        dialog.window.set_application(Some(&app));

        let answer = serde_json::json!({
            "accepted": ["https://example.test/a.bin", "https://example.test/b.bin"],
            "refused": [{ "line": "nope", "rejection": { "kind": "not_a_link" } }]
        });
        grabber::apply_preview(&dialog, &answer);
        assert_eq!(grabber::planned_links(&dialog).len(), 2);
        assert!(
            dialog.summary.label().contains('2'),
            "{}",
            dialog.summary.label()
        );
        assert!(dialog.refused.label().contains("nope"));
        assert!(dialog.refused.label().contains("not a downloadable link"));
        assert!(dialog.queue.is_sensitive());

        dialog.queue.emit_clicked();
        let sent = calls.lock().unwrap().clone();
        let adds: Vec<_> = sent
            .iter()
            .filter(|(method, _)| method == "download.add")
            .collect();
        assert_eq!(adds.len(), 2, "{sent:?}");
        assert_eq!(adds[0].1["url"], "https://example.test/a.bin");
    });
}

#[test]
fn an_empty_answer_disables_queueing_and_clears_the_list() {
    run_gui("grabber_empty", || {
        let app = app();
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let dialog = dialog_with(calls);
        dialog.window.set_application(Some(&app));

        grabber::apply_preview(
            &dialog,
            &serde_json::json!({ "accepted": [], "refused": [] }),
        );
        assert!(!dialog.queue.is_sensitive());
        assert!(grabber::planned_links(&dialog).is_empty());
        assert!(
            dialog.summary.label().contains("nothing"),
            "{}",
            dialog.summary.label()
        );
    });
}
