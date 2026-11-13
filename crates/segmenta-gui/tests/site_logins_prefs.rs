//! Site logins page (U1b): origin visible, password write-only and never rendered back.

use std::cell::RefCell;
use std::rc::Rc;

use segmenta_gui::adw;
use segmenta_gui::adw::prelude::*;
use segmenta_gui::gtk;
use segmenta_gui::prefs::{site_logins_page, RpcCall, SiteLoginEntry};
use serde_json::Value;

/// Records every call the page makes, in order.
/// Every call the page made, and the sink that records it.
type Recorder = (Rc<RefCell<Vec<(String, Value)>>>, RpcCall);

fn recorder() -> Recorder {
    let log: Rc<RefCell<Vec<(String, Value)>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&log);
    (
        log,
        Rc::new(move |method: &str, params: Value| {
            sink.borrow_mut().push((method.to_string(), params));
        }),
    )
}

fn entry(origin: &str, username: &str) -> SiteLoginEntry {
    SiteLoginEntry {
        origin: origin.to_string(),
        domain: "files.example".to_string(),
        username: username.to_string(),
    }
}

/// GTK must be initialized once, on the thread that owns the widgets, so the whole page
/// contract runs in one test instead of one per thread.
#[test]
fn the_site_logins_page_keeps_the_password_write_only() {
    if gtk::init().is_err() {
        // Same convention as the window suite: without a display the test cannot run, and
        // the GUI gate is the one that runs it for real under xvfb.
        eprintln!(
            "skipping the site logins page test: gtk::init() failed; \
             run `xvfb-run -a cargo test -p segmenta-gui`"
        );
        return;
    }
    let _app = adw::Application::builder()
        .application_id("io.github.wilkin.Segmenta.TestSiteLogins")
        .build();
    let (_, call) = recorder();
    let page = site_logins_page(
        &[
            entry("https://files.example", "ana"),
            entry("http://old.example", "luis"),
        ],
        call,
    );

    let rows: Vec<String> = page
        .list
        .observe_children()
        .into_iter()
        .filter_map(|child| child.ok().and_then(|c| c.downcast::<adw::ActionRow>().ok()))
        .map(|row| row.title().to_string())
        .collect();
    assert!(
        rows.contains(&"https://files.example".to_string()),
        "origin missing: {rows:?}"
    );
    assert!(
        rows.contains(&"http://old.example".to_string()),
        "origin missing: {rows:?}"
    );
    assert!(
        !rows.iter().any(|title| title.contains("s3cr3t")),
        "a password reached the list: {rows:?}"
    );
    assert!(!page.page.title().is_empty());
    let (_, call) = recorder();
    let page = site_logins_page(&[], call);
    let first = page
        .list
        .observe_children()
        .into_iter()
        .next()
        .and_then(|child| child.ok())
        .and_then(|child| child.downcast::<adw::ActionRow>().ok())
        .expect("the empty state row");
    assert_eq!(
        first.title(),
        segmenta_i18n::tr(
            segmenta_gui::prefs::locale(),
            segmenta_i18n::Msg::SiteLoginsEmpty
        )
    );
    let (log, call) = recorder();
    let page = site_logins_page(&[], call);
    page.origin.set_text("https://files.example:8443");
    page.username.set_text("ana");
    page.password.set_text("s3cr3t");

    page.add.emit_clicked();

    let sent = log.borrow().clone();
    assert_eq!(sent.len(), 1, "expected one call, got {sent:?}");
    assert_eq!(sent[0].0, "site_login.set");
    assert_eq!(sent[0].1["origin"], "https://files.example:8443");
    assert_eq!(sent[0].1["username"], "ana");
    assert_eq!(sent[0].1["password"], "s3cr3t");
    assert_eq!(
        page.password.text(),
        "",
        "the password must not stay in the widget"
    );
    assert!(
        page.error.text().is_empty() || !page.error.is_visible(),
        "a valid save must not show an error"
    );
    let (log, call) = recorder();
    let page = site_logins_page(&[], call);
    page.origin.set_text("https://files.example");
    page.username.set_text("ana");
    // No password.

    page.add.emit_clicked();

    assert!(
        log.borrow().is_empty(),
        "an incomplete form must not reach the daemon"
    );
    assert!(page.error.is_visible(), "the page must say what is missing");
    let (log, call) = recorder();
    let page = site_logins_page(&[entry("https://files.example", "ana")], call);

    page.removals
        .borrow()
        .first()
        .expect("one remove button per origin")
        .emit_clicked();

    let sent = log.borrow().clone();
    assert_eq!(sent.len(), 1, "expected one call, got {sent:?}");
    assert_eq!(sent[0].0, "site_login.remove");
    assert_eq!(sent[0].1["origin"], "https://files.example");

    // L21: a login the daemon already had appears when the page loads, and the password
    // never reaches the widget tree because the daemon does not send one.
    let client: std::sync::Arc<dyn segmenta_gui::client::Client> = std::sync::Arc::new(
        segmenta_gui::client::DemoClient::with_site_logins(vec![
            serde_json::json!({ "origin": "https://files.example", "username": "ana", "has_password": true }),
        ]),
    );
    let (_, reload_call) = recorder();
    let loaded = site_logins_page(&[], reload_call);
    segmenta_gui::prefs::load_site_logins(
        &loaded.list,
        std::sync::Arc::clone(&client),
        std::rc::Rc::new(|_: &str, _: Value| {}),
    );
    let context = segmenta_gui::glib::MainContext::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        while context.iteration(false) {}
        let titles: Vec<String> = loaded
            .list
            .observe_children()
            .into_iter()
            .filter_map(|child| child.ok().and_then(|c| c.downcast::<adw::ActionRow>().ok()))
            .map(|row| row.title().to_string())
            .collect();
        if titles.iter().any(|title| title == "https://files.example") {
            assert!(
                !titles.iter().any(|title| title.contains("s3cr3t")),
                "the loaded list leaked a password: {titles:?}"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the stored login never appeared: {titles:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
