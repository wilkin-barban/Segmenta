//! Preferences must open with every real page, not an empty shell (0.2.1 P0).
//!
//! GTK is not thread safe and `gtk::init()` refuses to run twice, so the whole body runs on
//! one dedicated toolkit thread — the same shape the window tests use. Building the dialog on
//! the test thread directly used to segfault the process.

use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use segmenta_gui::adw;
use segmenta_gui::adw::prelude::*;
use segmenta_gui::app;
use segmenta_gui::client::{Client, DemoClient};
use segmenta_gui::gtk;
use segmenta_gui::prefs;

/// One test body, executed on the toolkit thread.
type Job = Box<dyn FnOnce() + Send + 'static>;

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
        eprintln!(
            "skipping GUI test `{name}`: gtk::init() failed ({err}); \
             run `xvfb-run -a cargo test -p segmenta-gui`"
        );
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

fn app() -> adw::Application {
    app::build()
}

fn window_prefs_parent(application: &adw::Application) -> adw::ApplicationWindow {
    adw::ApplicationWindow::builder()
        .application(application)
        .title("Segmenta")
        .build()
}

/// Titles of every `AdwPreferencesPage` inside the dialog, mapped or not.
fn page_titles(root: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(page) = root.downcast_ref::<adw::PreferencesPage>() {
        let title = page.title().to_string();
        if !title.is_empty() {
            out.push(title);
        }
    }
    let mut next = root.first_child();
    while let Some(child) = next {
        page_titles(&child, out);
        next = child.next_sibling();
    }
}

/// Titles of every `AdwPreferencesGroup` inside the dialog, mapped or not.
fn group_titles(root: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(group) = root.downcast_ref::<adw::PreferencesGroup>() {
        let title = group.title().to_string();
        if !title.is_empty() {
            out.push(title);
        }
    }
    let mut next = root.first_child();
    while let Some(child) = next {
        group_titles(&child, out);
        next = child.next_sibling();
    }
}

// Known breakage, not introduced here: creating an `AdwApplicationWindow` with
// `gtk_window_set_application` segfaults in this binary (also on the v0.7.0 tag). The test
// stays disabled with an explicit reason instead of crashing the suite; the debt is tracked
// in odd/tasks/segmenta-v0.7.0-remediacion-gui.md.
#[test]
fn preferences_open_with_every_real_page_and_controls() {
    run_gui("prefs_pages", || {
        // `register` emits the startup signal, which is what makes adding a window legal.
        // `activate` is what used to crash `gtk_window_set_application`.
        let application = app();
        let _ = application.register(gtk::gio::Cancellable::NONE);
        let client: Arc<dyn Client> = Arc::new(DemoClient::empty());
        let dialog = prefs::present(
            // `open_preferences` uses the window itself as parent.
            &window_prefs_parent(&application),
            client,
            Default::default(),
            std::rc::Rc::new(|_| {}),
        );

        let context = gtk::glib::MainContext::default();
        for _ in 0..20 {
            while context.iteration(false) {}
        }

        let mut titles = Vec::new();
        page_titles(dialog.upcast_ref::<gtk::Widget>(), &mut titles);
        let mut unique = titles.clone();
        unique.sort();
        unique.dedup();
        assert!(
            unique.len() >= 6,
            "the dialog must build its real pages, got {unique:?}"
        );

        // The window exposes the VPN interface and the Debrid credentials.
        let mut groups = Vec::new();
        group_titles(dialog.upcast_ref::<gtk::Widget>(), &mut groups);
        assert!(
            groups.iter().any(|title| title == "VPN"),
            "the VPN group must be there, got {groups:?}"
        );
        assert!(
            groups.iter().any(|title| title == "Debrid"),
            "the Debrid group must be there, got {groups:?}"
        );
    });
}
