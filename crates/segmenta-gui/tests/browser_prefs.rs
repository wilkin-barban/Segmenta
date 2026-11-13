//! Preferences → Navegadores (ODD 0.2 E1): the states the page paints.
//!
//! Pure mapping, no toolkit: an unsupported browser, a browser with the native
//! manifest but no extension and a browser with the extension installed must
//! never collapse into the same sentence, and the page must not offer an action
//! that can only fail.

use serde_json::{json, Value};

use segmenta_gui::prefs::{browser_rows, load_steps, BrowserState};
use segmenta_i18n::Locale;

const PREPARED: &str = "/home/user/.local/share/segmenta/extensions/chromium-0.2.0";

/// One `extension.status` answer with the four states the daemon can report.
fn status() -> Value {
    json!({
        "extension_id": "kdeldnifdbpemnhfpmhonlnibbhnoaan",
        "firefox_extension_id": "segmenta@wilkin.github.io",
        "host_path": "/usr/bin/segmenta-host",
        "extension_version": "0.2.0",
        "browsers": [
            {
                "id": "chromium",
                "name": "Chromium",
                "family": "chromium",
                "command": "/snap/bin/chromium",
                "version": "Chromium 120.0",
                "unsupported": "snap",
                "manifest_path": "/etc/chromium/native-messaging-hosts/io.github.wilkin.segmenta.json",
                "manifest_present": true,
                "user_manifest_present": false,
                "extension_installed": false,
                "prepared_path": PREPARED,
                "archive_path": "/usr/share/segmenta/extensions/segmenta-chromium.zip",
                "state": "unsupported"
            },
            {
                "id": "chrome",
                "name": "Google Chrome",
                "family": "chromium",
                "command": "/usr/bin/google-chrome",
                "version": "Google Chrome 120.0",
                "unsupported": null,
                "manifest_path": "/etc/opt/chrome/native-messaging-hosts/io.github.wilkin.segmenta.json",
                "manifest_present": true,
                "user_manifest_present": false,
                "extension_installed": false,
                "prepared_path": PREPARED,
                "archive_path": "/usr/share/segmenta/extensions/segmenta-chromium.zip",
                "state": "native-ready"
            },
            {
                "id": "brave",
                "name": "Brave",
                "family": "chromium",
                "command": null,
                "version": null,
                "unsupported": null,
                "manifest_path": "/etc/brave/native-messaging-hosts/io.github.wilkin.segmenta.json",
                "manifest_present": false,
                "user_manifest_present": false,
                "extension_installed": false,
                "prepared_path": null,
                "archive_path": "/usr/share/segmenta/extensions/segmenta-chromium.zip",
                "state": "not-installed"
            },
            {
                "id": "firefox",
                "name": "Firefox",
                "family": "firefox",
                "command": "/usr/bin/firefox",
                "version": "Firefox 128.0",
                "unsupported": null,
                "manifest_path": "/usr/lib/mozilla/native-messaging-hosts/io.github.wilkin.segmenta.json",
                "manifest_present": false,
                "user_manifest_present": false,
                "extension_installed": true,
                "prepared_path": null,
                "archive_path": "/usr/share/segmenta/extensions/segmenta-firefox.zip",
                "state": "extension-installed"
            }
        ]
    })
}

fn row(locale: Locale, id: &str) -> segmenta_gui::prefs::BrowserRow {
    browser_rows(locale, &status())
        .into_iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("`{id}` is not listed"))
}

#[test]
fn only_detected_browsers_are_listed() {
    let ids: Vec<String> = browser_rows(Locale::Es, &status())
        .into_iter()
        .map(|row| row.id)
        .collect();
    // Brave has no `command`: it is not installed, so it is not a row.
    assert_eq!(ids, vec!["chromium", "chrome", "firefox"]);
}

#[test]
fn the_three_states_never_paint_the_same_sentence() {
    let unsupported = row(Locale::En, "chromium");
    let manifest_only = row(Locale::En, "chrome");
    let installed = row(Locale::En, "firefox");
    assert_eq!(unsupported.state, BrowserState::Unsupported);
    assert_eq!(manifest_only.state, BrowserState::NativeReady);
    assert_eq!(installed.state, BrowserState::ExtensionInstalled);
    assert_ne!(unsupported.status, manifest_only.status);
    assert_ne!(unsupported.status, installed.status);
    assert_ne!(manifest_only.status, installed.status);
}

#[test]
fn a_manifest_without_the_extension_is_stated_as_not_installed() {
    let manifest_only = row(Locale::En, "chrome");
    let text = manifest_only.status.to_lowercase();
    assert!(
        text.contains("manifest"),
        "the row does not say the manifest is there: {text}"
    );
    assert!(
        text.contains("not installed"),
        "a browser without the extension is not stated as not installed: {text}"
    );
}

#[test]
fn an_unsupported_package_says_which_package_and_why() {
    let unsupported = row(Locale::En, "chromium");
    let text = unsupported.status.to_lowercase();
    assert!(text.contains("snap"), "the reason is missing: {text}");
    assert!(
        text.contains("native host"),
        "the row does not explain what cannot run: {text}"
    );
}

#[test]
fn no_row_shows_a_bare_ready_word() {
    let bare = ["listo", "ready", "pronto"];
    for locale in [Locale::En, Locale::Es, Locale::PtBr] {
        for row in browser_rows(locale, &status()) {
            let text = row.status.trim().to_lowercase();
            assert!(!text.is_empty(), "`{}` has no state text", row.id);
            assert!(
                !bare.contains(&text.as_str()),
                "`{}` is painted as a bare `{text}`",
                row.id
            );
            // "Listo" may not be the headline either: whatever exists, the row
            // says what is still missing.
            assert!(
                !text.starts_with("listo")
                    && !text.starts_with("ready")
                    && !text.starts_with("pronto"),
                "`{}` starts with a bare readiness word: {text}",
                row.id
            );
        }
    }
}

#[test]
fn unsupported_browser_offers_no_action_that_can_only_fail() {
    let unsupported = row(Locale::Es, "chromium");
    assert!(
        !unsupported.actions.prepare && !unsupported.actions.open && !unsupported.actions.copy_path,
        "a snap browser still offers actions: {:?}",
        unsupported.actions
    );
    let usable = row(Locale::Es, "chrome");
    assert!(
        usable.actions.prepare,
        "no prepare action on a usable browser"
    );
    assert!(usable.actions.open, "no open action on a usable browser");
    assert!(
        usable.actions.copy_path,
        "no copy action although the extension is prepared"
    );
}

#[test]
fn copy_path_appears_only_when_the_extension_is_prepared() {
    // Firefox has no `prepared_path` in the payload: nothing to copy yet.
    assert_eq!(row(Locale::Es, "firefox").prepared_path, None);
    assert!(!row(Locale::Es, "firefox").actions.copy_path);
    assert_eq!(
        row(Locale::Es, "chrome").prepared_path.as_deref(),
        Some(PREPARED)
    );
}

#[test]
fn a_missing_manifest_points_at_the_install_step() {
    let mut payload = status();
    payload["browsers"][1]["state"] = json!("native-missing");
    payload["browsers"][1]["manifest_present"] = json!(false);
    let row = browser_rows(Locale::En, &payload)
        .into_iter()
        .find(|row| row.id == "chrome")
        .expect("chrome is not listed");
    assert_eq!(row.state, BrowserState::NativeMissing);
    let text = row.status.to_lowercase();
    assert!(
        text.contains("manifest"),
        "the missing manifest is not named: {text}"
    );
    assert!(
        text.contains("not installed"),
        "the missing extension is not named: {text}"
    );
}

#[test]
fn load_steps_are_numbered_and_family_specific() {
    let chromium = load_steps(Locale::En, "chromium");
    assert_eq!(chromium.len(), 4, "chromium steps: {chromium:?}");
    assert!(chromium[0].starts_with("1."), "not numbered: {chromium:?}");
    assert!(
        chromium
            .iter()
            .any(|step| step.contains("chrome://extensions")),
        "the Chromium page is never named: {chromium:?}"
    );
    assert!(
        chromium.iter().any(|step| step.contains("Load unpacked")),
        "loading unpacked is never named: {chromium:?}"
    );

    let firefox = load_steps(Locale::En, "firefox");
    assert_eq!(firefox.len(), 3, "firefox steps: {firefox:?}");
    assert!(
        firefox.iter().any(|step| step.contains("about:debugging")),
        "the Firefox page is never named: {firefox:?}"
    );
    assert!(
        firefox
            .iter()
            .any(|step| step.to_lowercase().contains("temporary")),
        "a temporary load is not stated: {firefox:?}"
    );
    assert!(
        firefox.iter().any(|step| step.contains("closes")),
        "losing the add-on on restart is not stated: {firefox:?}"
    );

    // Every catalog is filled and the numbers are contiguous from 1.
    for locale in [Locale::En, Locale::Es, Locale::PtBr] {
        for family in ["chromium", "firefox"] {
            let steps = load_steps(locale, family);
            assert!(!steps.is_empty(), "{family} has no steps in {locale:?}");
            for (index, step) in steps.iter().enumerate() {
                assert!(
                    step.starts_with(&format!("{}.", index + 1)),
                    "{family}/{locale:?} step {index} is not numbered: {step}"
                );
            }
        }
    }
    // An unknown family has no steps instead of a guessed how-to.
    assert!(load_steps(Locale::En, "other").is_empty());
}
