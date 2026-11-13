//! Preferences must open with every real page, not an empty shell (0.2.1 P0).

use std::sync::Arc;

use segmenta_gui::adw;
use segmenta_gui::adw::prelude::*;
use segmenta_gui::app;
use segmenta_gui::client::{Client, DemoClient};
use segmenta_gui::gtk;
use segmenta_gui::prefs;

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

#[test]
fn preferences_open_with_every_real_page_and_controls() {
    if gtk::init().is_err() {
        eprintln!("skipping: no display; run under `xvfb-run -a cargo test -p segmenta-gui`");
        return;
    }
    let application = app::build();
    let _ = application.register(gtk::gio::Cancellable::NONE);
    application.activate();
    let client: Arc<dyn Client> = Arc::new(DemoClient::empty());
    let window = app::build_window_with_client(&application, client.clone());
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
    // Every page the directive names must exist, in the active locale.
    let locale = prefs::locale();
    let expected_pages = [
        segmenta_i18n::Msg::PrefsGeneral,
        segmenta_i18n::Msg::PrefsConnectionsSection,
        segmenta_i18n::Msg::PrefsCapture,
        segmenta_i18n::Msg::PrefsBrowsers,
        segmenta_i18n::Msg::PrefsAntivirus,
        segmenta_i18n::Msg::PrefsSiteLogins,
        segmenta_i18n::Msg::About,
    ];
    for msg in expected_pages {
        let expected = segmenta_i18n::tr(locale, msg);
        assert!(
            unique.iter().any(|title| title == expected),
            "page `{expected}` is missing from {unique:?}"
        );
    }
    assert!(
        dialog.is_visible() || dialog.root().is_some(),
        "the dialog was not presented"
    );
    // A narrow dialog collapses Adw into a bottom switcher that clips the last tabs and
    // squeezes row titles, which is exactly what the X11 capture showed.
    assert!(
        dialog.content_width() >= 900,
        "the dialog must be wide enough for seven pages, got {}",
        dialog.content_width()
    );
    let _ = window;
}

fn window_prefs_parent(application: &adw::Application) -> adw::ApplicationWindow {
    adw::ApplicationWindow::builder()
        .application(application)
        .title("Segmenta")
        .build()
}
