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
                // Any accidental production client resolves to a dead socket instead of
                // the daemon of whoever runs the suite.
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

/// The application of this binary: a test id and `NON_UNIQUE`, so it never collides on
/// D-Bus with a `segmenta-gui` the user has open on the desktop.
fn app() -> adw::Application {
    adw::Application::builder()
        .application_id("io.github.wilkin.Segmenta.TestPrefs")
        .flags(segmenta_gui::gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build()
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

// Root cause of the old segfault, kept so nobody reintroduces it: the test used to call
// `application.activate()` on an `AdwApplication` that had no window yet. GTK refuses to
// attach a window before the `GApplication::startup` signal has been emitted, and here it
// died inside `gtk_window_set_application`. `register()` emits startup, which is what makes
// adding the parent window legal; `activate()` is unnecessary for building a dialog and is
// what crashed.
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
            unique.len() >= 8,
            "the dialog must build its real pages, got {unique:?}"
        );
        assert!(
            titles.iter().any(|title| title == "DNS"),
            "the DNS page must be in the preferences dialog, got {titles:?}"
        );
        assert!(
            titles.iter().any(|title| title == "Reglas de sitios"),
            "the site rules page must be in the preferences dialog, got {titles:?}"
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

#[test]
fn the_dns_page_reflects_dns_mode_and_selecting_dot_shows_note() {
    run_gui("dns_page_mode", || {
        let application = adw::Application::builder()
            .application_id("io.github.wilkin.Segmenta.TestPrefsDnsMode")
            .flags(segmenta_gui::gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        let _ = application.register(gtk::gio::Cancellable::NONE);

        let system_settings = prefs::DnsSettings {
            dns_mode: "system".to_string(),
            ..Default::default()
        };
        let save_log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let save_sink = {
            let log = std::rc::Rc::clone(&save_log);
            std::rc::Rc::new(move |patch: serde_json::Value| {
                log.borrow_mut().push(patch);
            })
        };

        let page = prefs::dns_page(&system_settings, save_sink);
        assert!(
            !page.dot_note.is_visible(),
            "DoT note must be hidden when dns_mode is system"
        );
        assert_eq!(page.mode.selected(), 0);
        assert!(
            !page.provider.is_sensitive(),
            "DoH provider must be insensitive when mode is system"
        );

        // Selecting dot makes the note visible
        page.mode.set_selected(2);
        assert!(
            page.dot_note.is_visible(),
            "DoT note must be visible when dot is selected"
        );
        assert!(
            !page.provider.is_sensitive(),
            "DoH provider must be insensitive when mode is dot"
        );

        // Selecting doh hides the note and enables the provider
        page.mode.set_selected(1);
        assert!(
            !page.dot_note.is_visible(),
            "DoT note must be hidden when mode is doh"
        );
        assert!(
            page.provider.is_sensitive(),
            "DoH provider must be sensitive when mode is doh"
        );

        // Submitting custom URL issues settings.update
        page.custom_url.set_text("https://dns.quad9.net/dns-query");
        page.custom_url.emit_by_name::<()>("apply", &[]);
        let logs = save_log.borrow().clone();
        assert!(
            logs.iter().any(|patch| patch
                == &serde_json::json!({ "dns_custom_url": "https://dns.quad9.net/dns-query" })),
            "custom URL apply must issue settings.update with dns_custom_url, got {logs:?}"
        );

        // A page starting with dot has the note visible immediately
        let dot_settings = prefs::DnsSettings {
            dns_mode: "dot".to_string(),
            ..Default::default()
        };
        let dummy_save = std::rc::Rc::new(|_| {});
        let dot_page = prefs::dns_page(&dot_settings, dummy_save);
        assert!(
            dot_page.dot_note.is_visible(),
            "DoT note must be visible immediately when initialized with dns_mode = dot"
        );
        assert_eq!(dot_page.mode.selected(), 2);
    });
}

#[test]
fn changing_the_dns_provider_issues_exactly_one_settings_update() {
    run_gui("dns_page_provider", || {
        let application = adw::Application::builder()
            .application_id("io.github.wilkin.Segmenta.TestPrefsDnsProvider")
            .flags(segmenta_gui::gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        let _ = application.register(gtk::gio::Cancellable::NONE);

        let settings = prefs::DnsSettings {
            dns_mode: "doh".to_string(),
            dns_provider: "cloudflare".to_string(),
            ..Default::default()
        };
        let updates = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let save_sink = {
            let log = std::rc::Rc::clone(&updates);
            std::rc::Rc::new(move |patch: serde_json::Value| {
                log.borrow_mut().push(patch);
            })
        };

        let page = prefs::dns_page(&settings, save_sink);
        assert_eq!(
            updates.borrow().len(),
            0,
            "no settings.update should be issued during page creation"
        );

        // Change provider from cloudflare (0) to quad9 (1)
        page.provider.set_selected(1);

        let sent = updates.borrow().clone();
        assert_eq!(
            sent.len(),
            1,
            "changing the provider must issue exactly one settings.update, got {sent:?}"
        );
        assert_eq!(
            sent[0],
            serde_json::json!({ "dns_provider": "quad9" }),
            "expected dns_provider: quad9 patch"
        );
    });
}

fn collect_all_text(root: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(row) = root.downcast_ref::<adw::ActionRow>() {
        let title = row.title().to_string();
        if !title.is_empty() {
            out.push(title);
        }
        if let Some(subtitle) = row.subtitle() {
            let s = subtitle.to_string();
            if !s.is_empty() {
                out.push(s);
            }
        }
    }
    if let Some(entry) = root.downcast_ref::<adw::EntryRow>() {
        let title = entry.title().to_string();
        if !title.is_empty() {
            out.push(title);
        }
        let text = entry.text().to_string();
        if !text.is_empty() {
            out.push(text);
        }
    }
    if let Some(label) = root.downcast_ref::<gtk::Label>() {
        let text = label.text().to_string();
        if !text.is_empty() {
            out.push(text);
        }
    }
    let mut next = root.first_child();
    while let Some(child) = next {
        collect_all_text(&child, out);
        next = child.next_sibling();
    }
}

#[test]
fn site_rules_page_lists_daemon_rules_and_shows_no_header_value() {
    run_gui("site_rules_page", || {
        let application = adw::Application::builder()
            .application_id("io.github.wilkin.Segmenta.TestPrefsSiteRules")
            .flags(segmenta_gui::gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        let _ = application.register(gtk::gio::Cancellable::NONE);

        // Daemon response includes stored rules with headers_secret_id and headers
        let daemon_json = serde_json::json!([
            {
                "id": "rule-1",
                "domain_pattern": "files.example.com",
                "user_agent": "CustomAgent/2.0",
                "max_connections": 8,
                "browser_like": true,
                "headers_secret_id": "site-rule:example-alias-1234",
                "headers": [["authorization", "Bearer super_secret_token"]]
            },
            {
                "id": "rule-2",
                "domain_pattern": "*.mirror.org",
                "user_agent": null,
                "max_connections": 4,
                "browser_like": false,
                "headers_secret_id": "secret-alias-5678"
            }
        ]);

        let entries = prefs::site_rule_entries(&daemon_json);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].domain_pattern, "files.example.com");
        assert_eq!(entries[0].user_agent.as_deref(), Some("CustomAgent/2.0"));
        assert_eq!(entries[0].max_connections, Some(8));

        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let call_sink = {
            let log = std::rc::Rc::clone(&calls);
            std::rc::Rc::new(move |method: &str, params: serde_json::Value| {
                log.borrow_mut().push((method.to_string(), params));
            })
        };

        let page = prefs::site_rules_page(&entries, call_sink);

        let mut rendered_texts = Vec::new();
        collect_all_text(page.page.upcast_ref::<gtk::Widget>(), &mut rendered_texts);
        let combined = rendered_texts.join(" ");

        // Rules are listed
        assert!(
            combined.contains("files.example.com"),
            "domain pattern files.example.com must be visible, got {rendered_texts:?}"
        );
        assert!(
            combined.contains("*.mirror.org"),
            "domain pattern *.mirror.org must be visible, got {rendered_texts:?}"
        );
        assert!(
            combined.contains("CustomAgent/2.0"),
            "user agent CustomAgent/2.0 must be visible, got {rendered_texts:?}"
        );

        // Headers and secrets must NEVER be shown
        assert!(
            !combined.contains("site-rule:example-alias-1234"),
            "headers_secret_id leaked into the UI: {rendered_texts:?}"
        );
        assert!(
            !combined.contains("secret-alias-5678"),
            "headers_secret_id leaked into the UI: {rendered_texts:?}"
        );
        assert!(
            !combined.contains("super_secret_token"),
            "secret header value leaked into the UI: {rendered_texts:?}"
        );
        assert!(
            !combined.contains("authorization"),
            "header name leaked into the UI: {rendered_texts:?}"
        );

        // Removing a rule calls site_rule.remove
        let removals = page.removals.borrow().clone();
        assert_eq!(removals.len(), 2);
        removals[0].emit_clicked();
        let sent = calls.borrow().clone();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "site_rule.remove");
        assert_eq!(sent[0].1["id"], "rule-1");
        // Only the id: the daemon's `site_rule.remove` takes exactly this field.
        assert!(
            sent[0].1.get("domain_pattern").is_none(),
            "the remove payload must match the daemon params: {:?}",
            sent[0].1
        );

        // Add affordance validation
        page.domain_pattern.set_text("");
        page.add.emit_clicked();
        assert!(
            page.error.is_visible(),
            "empty domain pattern must show error"
        );
        assert_eq!(
            calls.borrow().len(),
            1,
            "validation error must not send RPC"
        );

        page.domain_pattern.set_text("api.example.com");
        page.user_agent.set_text("Custom/3.0");
        page.max_connections.set_value(16.0);
        page.add.emit_clicked();
        assert!(!page.error.is_visible(), "valid addition must hide error");
        let sent = calls.borrow().clone();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].0, "site_rule.upsert");
        assert_eq!(sent[1].1["domain_pattern"], "api.example.com");
        assert_eq!(sent[1].1["user_agent"], "Custom/3.0");
        assert_eq!(sent[1].1["max_connections"], 16);

        // Empty rules page
        let empty_calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let empty_sink = std::rc::Rc::new(move |method: &str, params: serde_json::Value| {
            empty_calls.borrow_mut().push((method.to_string(), params));
        });
        let empty_page = prefs::site_rules_page(&[], empty_sink);
        let mut empty_texts = Vec::new();
        collect_all_text(
            empty_page.list.upcast_ref::<gtk::Widget>(),
            &mut empty_texts,
        );
        let empty_combined = empty_texts.join(" ");
        assert!(
            empty_combined.contains("No hay reglas de sitios configuradas"),
            "empty state must be displayed when no rules exist, got {empty_texts:?}"
        );
    });
}
