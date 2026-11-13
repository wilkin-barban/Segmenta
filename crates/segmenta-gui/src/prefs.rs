//! Preferences dialog.
//!
//! Every control saves immediately with `settings.update` (idiomatic
//! Libadwaita: no explicit save button) and any daemon rejection is shown as a
//! toast coming from the JSON-RPC error, for example an out-of-range value.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use adw::prelude::*;

use crate::client::{Client, RequestBus};
use crate::model::Settings;
use crate::{adw, gtk};
use segmenta_i18n::{tr, Locale, Msg};

fn locale() -> segmenta_i18n::Locale {
    segmenta_i18n::current()
}

/// Opens the preferences dialog over `parent`.
pub fn present(
    parent: &adw::ApplicationWindow,
    client: Arc<dyn Client>,
    settings: Settings,
    on_saved: Rc<dyn Fn(Settings)>,
) {
    let dialog = adw::PreferencesDialog::builder()
        .title(tr(locale(), Msg::Preferences))
        .content_width(640)
        .content_height(720)
        .search_enabled(true)
        .build();

    let bus = Rc::new(RequestBus::new());
    let browser_bus = Rc::new(RequestBus::new());

    // ---------------------------------------------------------------- General
    let general = adw::PreferencesPage::builder()
        .title(tr(locale(), Msg::PrefsGeneral))
        .icon_name("emblem-system-symbolic")
        .build();
    let storage = adw::PreferencesGroup::builder()
        .title(tr(locale(), Msg::PrefsFolders))
        .build();

    let download_dir = adw::ActionRow::builder()
        .title(tr(locale(), Msg::DownloadFolder))
        .subtitle(settings.download_dir.clone())
        .activatable(true)
        .build();
    download_dir.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
    storage.add(&download_dir);
    // Per-category folders are not part of the 0.2 UI: no row, no editor. The
    // `category_dirs` setting is still read from old databases and kept as is.
    general.add(&storage);

    let session = adw::PreferencesGroup::builder()
        .title(tr(locale(), Msg::SessionSection))
        .build();
    let autostart = adw::SwitchRow::builder()
        .title(tr(locale(), Msg::Autostart))
        .subtitle(tr(locale(), Msg::AutostartHint))
        .active(settings.autostart)
        .build();
    session.add(&autostart);
    general.add(&session);

    // -------------------------------------------------- Connections and limits
    let network = adw::PreferencesPage::builder()
        .title(tr(locale(), Msg::PrefsConnectionsSection))
        .icon_name("network-transmit-receive-symbolic")
        .build();
    let limits = adw::PreferencesGroup::builder()
        .title(tr(locale(), Msg::PrefsTransfer))
        .description(tr(locale(), Msg::RetriesHint))
        .build();

    let max_connections = adw::SpinRow::with_range(1.0, 32.0, 1.0);
    max_connections.set_title(tr(locale(), Msg::ConnectionsPerDownload));
    max_connections.set_subtitle(tr(locale(), Msg::ConnectionsRangeHint));
    max_connections.set_value(f64::from(settings.max_connections.clamp(1, 32)));
    limits.add(&max_connections);

    let speed_limit = adw::SpinRow::with_range(0.0, 1_000_000.0, 128.0);
    speed_limit.set_title(tr(locale(), Msg::SpeedLimit));
    speed_limit.set_subtitle(tr(locale(), Msg::SpeedLimitHint));
    speed_limit.set_value(settings.speed_limit_kib() as f64);
    limits.add(&speed_limit);

    let retries = adw::SpinRow::with_range(0.0, 50.0, 1.0);
    retries.set_title(tr(locale(), Msg::RetriesPerTask));
    retries.set_value(f64::from(settings.max_retries));
    limits.add(&retries);
    network.add(&limits);

    let proxy = adw::PreferencesGroup::builder().title("Proxy").build();
    let proxy_mode = adw::ActionRow::builder()
        .title(tr(locale(), Msg::ProxyMode))
        .subtitle(&settings.proxy_mode)
        .build();
    proxy.add(&proxy_mode);
    let proxy_url = adw::ActionRow::builder()
        .title(tr(locale(), Msg::ProxyServer))
        .subtitle(
            settings
                .proxy_url
                .clone()
                .unwrap_or_else(|| "—".to_string()),
        )
        .build();
    proxy.add(&proxy_url);
    let local = adw::SwitchRow::builder()
        .title(tr(locale(), Msg::AllowLocal))
        .subtitle(tr(locale(), Msg::AllowLocalHint))
        .active(settings.allow_local_downloads)
        .build();
    proxy.add(&local);
    network.add(&proxy);

    // -------------------------------------------------------------- Browsers
    //
    // The page never guesses readiness: every row is the `extension.status`
    // `state` in plain language plus the three actions the contract already
    // exposes. There is no store and no signature, and the GUI says so instead
    // of implying that a manifest alone means "installed" (E1).
    let browser_handlers = BrowserHandlers {
        prepare: {
            let bus = Rc::clone(&browser_bus);
            let client = Arc::clone(&client);
            Rc::new(move |family: &str| {
                bus.spawn(&client, "extension.prepare", json!({ "family": family }));
            })
        },
        open: {
            let bus = Rc::clone(&browser_bus);
            let client = Arc::clone(&client);
            Rc::new(move |browser_id: &str| {
                bus.spawn(
                    &client,
                    "extension.open",
                    json!({ "browser_id": browser_id }),
                );
            })
        },
        copy_path: {
            let dialog = dialog.clone();
            let toast_locale = locale();
            Rc::new(move |path: &str| {
                dialog.clipboard().set_text(path);
                dialog.add_toast(adw::Toast::new(tr(toast_locale, Msg::ExtensionPathCopied)));
            })
        },
        install_native: {
            let bus = Rc::clone(&browser_bus);
            let client = Arc::clone(&client);
            Rc::new(move || {
                bus.spawn(&client, "extension.install-native", json!({}));
            })
        },
    };
    let browser_page = browsers_page(locale(), &json!({}), browser_handlers.clone());
    let browsers_page_widget = browser_page.page;
    let browsers_group = browser_page.group;
    browser_bus.spawn(&client, "extension.status", json!({}));

    // ------------------------------------------------------------ Web capture
    let capture = adw::PreferencesPage::builder()
        .title(tr(locale(), Msg::PrefsCapture))
        .icon_name("web-browser-symbolic")
        .build();
    let capture_group = adw::PreferencesGroup::builder()
        .title(tr(locale(), Msg::PrefsCapture))
        .description(tr(locale(), Msg::CaptureEnabledHint))
        .build();

    let capture_enabled = adw::SwitchRow::builder()
        .title(tr(locale(), Msg::CaptureEnabled))
        .active(settings.capture_enabled)
        .build();
    capture_group.add(&capture_enabled);
    let capture_torrents = adw::SwitchRow::builder()
        .title(tr(locale(), Msg::CaptureTorrents))
        .active(settings.capture_torrents)
        .build();
    capture_group.add(&capture_torrents);
    let capture_magnet = adw::SwitchRow::builder()
        .title(tr(locale(), Msg::CaptureMagnet))
        .active(settings.capture_magnet)
        .build();
    capture_group.add(&capture_magnet);

    // Tokens are edited in a small dialog: the row only shows the current count.
    let extensions_row = adw::ActionRow::builder()
        .title(tr(locale(), Msg::CaptureExtensions))
        .subtitle(format!(
            "{} · {}",
            settings.capture_extensions.len(),
            tr(locale(), Msg::CaptureExtensionsHint)
        ))
        .build();
    let extensions_button = gtk::Button::with_label(tr(locale(), Msg::Edit));
    extensions_row.add_suffix(&extensions_button);
    extensions_row.set_activatable_widget(Some(&extensions_button));
    capture_group.add(&extensions_row);

    let denylist_row = adw::ActionRow::builder()
        .title(tr(locale(), Msg::DomainDenylist))
        .subtitle(format!(
            "{} · {}",
            settings.domain_denylist.len(),
            tr(locale(), Msg::DomainDenylistHint)
        ))
        .build();
    let denylist_button = gtk::Button::with_label(tr(locale(), Msg::Edit));
    denylist_row.add_suffix(&denylist_button);
    denylist_row.set_activatable_widget(Some(&denylist_button));
    capture_group.add(&denylist_row);
    capture.add(&capture_group);

    // -------------------------------------------------------------- About page
    let about = adw::PreferencesPage::builder()
        .title(tr(locale(), Msg::About))
        .icon_name("help-about-symbolic")
        .build();
    let info = adw::PreferencesGroup::new();
    let version = adw::ActionRow::builder()
        .title("Segmenta")
        .subtitle(tr(locale(), Msg::VersionLabel).replace("{}", env!("CARGO_PKG_VERSION")))
        .build();
    info.add(&version);
    let license = adw::ActionRow::builder()
        .title(tr(locale(), Msg::License))
        .subtitle("GPL-3.0-or-later")
        .build();
    info.add(&license);
    let website = adw::ActionRow::builder()
        .title(tr(locale(), Msg::Website))
        .subtitle("github.com/wilkin-barban/Segmenta")
        .activatable(true)
        .build();
    info.add(&website);
    let protocol = adw::ActionRow::builder()
        .title(tr(locale(), Msg::ProtocolLabel))
        .subtitle(tr(locale(), Msg::ProtocolValue))
        .build();
    info.add(&protocol);
    // `system.status` is informational: it fills this row and never blocks the
    // dialog while the daemon answers.
    let daemon_row = adw::ActionRow::builder()
        .title(tr(locale(), Msg::DaemonVersion))
        .subtitle(tr(locale(), Msg::Querying))
        .build();
    info.add(&daemon_row);
    about.add(&info);

    dialog.add(&general);
    dialog.add(&network);
    dialog.add(&capture);
    dialog.add(&browsers_page_widget);
    dialog.add(&about);

    // ------------------------------------------------------------ Save helpers
    //
    // One poller for the whole dialog: every control only enqueues the partial
    // `settings.update`, and the tick forwards the answer (or the JSON-RPC
    // error) to a toast. This keeps a single main-loop source alive and makes
    // rapid changes safe.
    let open = Rc::new(std::cell::Cell::new(true));
    let open_for_close = Rc::clone(&open);
    dialog.connect_closed(move |_| open_for_close.set(false));

    // Browsers poll: `extension.status` repaints the rows; `extension.prepare`,
    // `extension.open` and `extension.install-native` report back and ask for a
    // fresh status so every row shows its post-action state.
    let browser_views: Rc<std::cell::RefCell<Vec<BrowserRowView>>> =
        Rc::new(std::cell::RefCell::new(browser_page.rows));
    let browser_views_poll = Rc::clone(&browser_views);
    let browser_group_poll = browsers_group.clone();
    let browser_dialog_poll = dialog.clone();
    let browser_open_poll = Rc::clone(&open);
    let browser_locale = locale();
    let browser_handlers_poll = browser_handlers.clone();
    let refresh_poll_bus = Rc::clone(&browser_bus);
    let refresh_poll_client = Arc::clone(&client);
    let browser_poll = Rc::clone(&browser_bus);
    gtk::glib::timeout_add_local(Duration::from_millis(150), move || {
        if !browser_open_poll.get() {
            return gtk::glib::ControlFlow::Break;
        }
        for (_, result) in browser_poll.take() {
            match result {
                Ok(value) => {
                    if value.get("browsers").and_then(Value::as_array).is_some() {
                        for view in browser_views_poll.borrow_mut().drain(..) {
                            browser_group_poll.remove(&view.row);
                        }
                        let painted = fill_browser_rows(
                            &browser_group_poll,
                            browser_locale,
                            &value,
                            &browser_handlers_poll,
                        );
                        *browser_views_poll.borrow_mut() = painted;
                    } else if let Some(installed) = value.get("installed").and_then(Value::as_array)
                    {
                        // Refresh so every row shows its post-install state.
                        refresh_poll_bus.spawn(&refresh_poll_client, "extension.status", json!({}));
                        let errors = value
                            .get("errors")
                            .and_then(Value::as_array)
                            .map(|entries| entries.len())
                            .unwrap_or(0);
                        browser_dialog_poll.add_toast(adw::Toast::new(&format!(
                            "{}: {} · {}",
                            tr(browser_locale, Msg::InstallNative),
                            installed.len(),
                            errors
                        )));
                    } else if let Some(prepared) = value.get("prepared").and_then(Value::as_array) {
                        // The prepared folder is what `Copy path` hands over: repaint.
                        refresh_poll_bus.spawn(&refresh_poll_client, "extension.status", json!({}));
                        browser_dialog_poll.add_toast(adw::Toast::new(&format!(
                            "{}: {}",
                            tr(browser_locale, Msg::ExtensionPrepared),
                            prepared.len()
                        )));
                    }
                }
                Err(err) => browser_dialog_poll.add_toast(adw::Toast::new(&err.ui_message())),
            }
        }
        gtk::glib::ControlFlow::Continue
    });

    let bus_poll = Rc::clone(&bus);
    let dialog_poll = dialog.clone();
    let saved = Rc::clone(&on_saved);
    let open_poll = Rc::clone(&open);
    gtk::glib::timeout_add_local(Duration::from_millis(120), move || {
        if !open_poll.get() {
            return gtk::glib::ControlFlow::Break;
        }
        for (_, result) in bus_poll.take() {
            match result {
                Ok(value) => match serde_json::from_value::<Settings>(value) {
                    Ok(settings) => saved(settings),
                    Err(err) => dialog_poll.add_toast(adw::Toast::new(&format!(
                        "Respuesta inválida del daemon: {err}"
                    ))),
                },
                Err(err) => dialog_poll.add_toast(adw::Toast::new(&err.ui_message())),
            }
        }
        gtk::glib::ControlFlow::Continue
    });

    let save: Rc<dyn Fn(Value)> = {
        let bus = Rc::clone(&bus);
        let client = Arc::clone(&client);
        Rc::new(move |patch: Value| {
            bus.spawn(&client, "settings.update", patch);
        })
    };

    let save_capture = Rc::clone(&save);
    capture_enabled.connect_active_notify(move |row| {
        save_capture(json!({ "capture_enabled": row.is_active() }));
    });
    let save_torrents = Rc::clone(&save);
    capture_torrents.connect_active_notify(move |row| {
        save_torrents(json!({ "capture_torrents": row.is_active() }));
    });
    let save_magnet = Rc::clone(&save);
    capture_magnet.connect_active_notify(move |row| {
        save_magnet(json!({ "capture_magnet": row.is_active() }));
    });

    let edit_extensions = {
        let dialog = dialog.clone();
        let save = Rc::clone(&save);
        let current = settings.capture_extensions.clone();
        let row = extensions_row.clone();
        let locale = locale();
        move || {
            edit_list(
                &dialog,
                tr(locale, Msg::CaptureExtensions),
                tr(locale, Msg::CaptureExtensionsHint),
                &current,
                {
                    let save = Rc::clone(&save);
                    let row = row.clone();
                    move |tokens: Vec<String>| {
                        row.set_subtitle(&format!("{}", tokens.len()));
                        save(json!({ "capture_extensions": tokens }));
                    }
                },
            );
        }
    };
    let edit_extensions_handle = edit_extensions.clone();
    extensions_button.connect_clicked(move |_| edit_extensions_handle());
    extensions_row.connect_activated({
        let edit = edit_extensions.clone();
        move |_| edit()
    });

    let edit_denylist = {
        let dialog = dialog.clone();
        let save = Rc::clone(&save);
        let current = settings.domain_denylist.clone();
        let row = denylist_row.clone();
        let locale = locale();
        move || {
            edit_list(
                &dialog,
                tr(locale, Msg::DomainDenylist),
                tr(locale, Msg::DomainDenylistHint),
                &current,
                {
                    let save = Rc::clone(&save);
                    let row = row.clone();
                    move |tokens: Vec<String>| {
                        row.set_subtitle(&format!("{}", tokens.len()));
                        save(json!({ "domain_denylist": tokens }));
                    }
                },
            );
        }
    };
    let edit_denylist_handle = edit_denylist.clone();
    denylist_button.connect_clicked(move |_| edit_denylist_handle());
    denylist_row.connect_activated({
        let edit = edit_denylist.clone();
        move |_| edit()
    });

    let save_autostart = Rc::clone(&save);
    autostart.connect_active_notify(move |row| {
        save_autostart(json!({ "autostart": row.is_active() }));
    });

    let save_connections = Rc::clone(&save);
    max_connections.connect_value_notify(move |row| {
        let value = row.value().round().clamp(1.0, 32.0) as u32;
        save_connections(json!({ "max_connections": value }));
    });

    let save_speed = Rc::clone(&save);
    speed_limit.connect_value_notify(move |row| {
        let kib = row.value().max(0.0).round() as u64;
        save_speed(json!({ "global_speed_limit_bps": Settings::speed_limit_bps(kib) }));
    });

    let save_retries = Rc::clone(&save);
    retries.connect_value_notify(move |row| {
        let value = row.value().max(0.0).round() as u32;
        save_retries(json!({ "max_retries": value }));
    });

    let save_local = Rc::clone(&save);
    local.connect_active_notify(move |row| {
        save_local(json!({ "allow_local_downloads": row.is_active() }));
    });

    // Download directory: portal folder picker, then `settings.update`.
    let parent_window = parent.clone();
    let row_for_picker = download_dir.clone();
    let save_dir = Rc::clone(&save);
    download_dir.connect_activated(move |_| {
        let chooser = gtk::FileDialog::builder()
            .title(tr(locale(), Msg::ChooseDownloadFolder))
            .modal(true)
            .build();
        let row = row_for_picker.clone();
        let save = Rc::clone(&save_dir);
        chooser.select_folder(
            Some(&parent_window),
            None::<&gtk::gio::Cancellable>,
            move |result| {
                let Ok(file) = result else {
                    return;
                };
                let Some(path) = file.path() else {
                    return;
                };
                let path = path.to_string_lossy().to_string();
                row.set_subtitle(&path);
                save(json!({ "download_dir": path }));
            },
        );
    });

    website.connect_activated(|_| {
        let _ = gtk::gio::AppInfo::launch_default_for_uri(
            "https://github.com/wilkin-barban/Segmenta",
            None::<&gtk::gio::AppLaunchContext>,
        );
    });

    let status_bus = RequestBus::new();
    status_bus.spawn(&client, "system.status", json!({}));
    let status_daemon_row = daemon_row.clone();
    let status_open = Rc::clone(&open);
    let status_bus_poll = status_bus;
    gtk::glib::timeout_add_local(Duration::from_millis(150), move || {
        if !status_open.get() {
            return gtk::glib::ControlFlow::Break;
        }
        if let Some((_, result)) = status_bus_poll.take().into_iter().next() {
            let subtitle = match result {
                Ok(value) => format!(
                    "{} · {} s activo · {} conexiones · secretos: {}",
                    value
                        .get("daemon_version")
                        .and_then(|version| version.as_str())
                        .unwrap_or("?"),
                    value
                        .get("uptime_seconds")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    value
                        .get("active_connections")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    value
                        .get("secret_backend")
                        .and_then(|backend| backend.as_str())
                        .unwrap_or("?"),
                ),
                Err(err) => err.ui_message(),
            };
            status_daemon_row.set_subtitle(&subtitle);
            return gtk::glib::ControlFlow::Break;
        }
        gtk::glib::ControlFlow::Continue
    });

    dialog.present(Some(parent));
}

/// Split a free-form list into trimmed tokens, dropping empties and duplicates.
pub fn parse_tokens(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for token in text.split(['\n', ',', ' ']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if !tokens.iter().any(|existing| existing == token) {
            tokens.push(token.to_string());
        }
    }
    tokens
}

/// Dialog with one multi-line editor for a token list.
fn edit_list(
    parent: &adw::PreferencesDialog,
    title: &str,
    hint: &str,
    current: &[String],
    apply: impl Fn(Vec<String>) + 'static,
) {
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(480)
        .build();
    let view = gtk::TextView::new();
    view.buffer().set_text(&current.join("\n"));
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(240)
        .child(&view)
        .build();
    let hint_label = gtk::Label::builder()
        .label(hint)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label"])
        .build();
    let save = gtk::Button::with_label(tr(locale(), Msg::SaveAction));
    save.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label(tr(locale(), Msg::Cancel));
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&save);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.append(&hint_label);
    content.append(&scroll);
    content.append(&buttons);
    dialog.set_child(Some(&content));

    let apply = std::rc::Rc::new(apply);
    let buffer = view.buffer();
    let save_dialog = dialog.clone();
    save.connect_clicked(move |_| {
        let (start, end) = buffer.bounds();
        let text = buffer.text(&start, &end, false).to_string();
        apply(parse_tokens(&text));
        let _ = save_dialog.close();
    });
    let cancel_dialog = dialog.clone();
    cancel.connect_clicked(move |_| {
        cancel_dialog.close();
    });
    dialog.present(Some(parent));
}

/// One browser row derived from an `extension.status` answer.
///
/// The row is plain data so the page can be asserted without a daemon: the
/// daemon owns the `state`, this layer only turns it into a sentence that says
/// what exists and what is still missing (E1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserRow {
    /// Stable daemon id (`chrome`, `firefox`, …).
    pub id: String,
    /// Human name.
    pub name: String,
    /// `chromium` or `firefox`: the family the prepare step works on.
    pub family: String,
    /// First line of `--version`, when the daemon could run it.
    pub version: Option<String>,
    /// The state the daemon reported.
    pub state: BrowserState,
    /// Full sentence the row shows. Never a bare "ready".
    pub status: String,
    /// Which of the three contract actions make sense for this row.
    pub actions: BrowserActions,
    /// Folder `extension.prepare` left ready, when it exists.
    pub prepared_path: Option<String>,
}

/// The `state` of one browser, as `extension.status` defines it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserState {
    /// The binary lives in a snap/flatpak sandbox: the native host cannot run.
    Unsupported,
    /// No `command` on this system.
    NotInstalled,
    /// The browser is installed but neither manifest nor extension is there.
    NativeMissing,
    /// A native manifest exists; the extension does not.
    NativeReady,
    /// The browser profile shows the extension.
    ExtensionInstalled,
}

impl BrowserState {
    /// Reads the daemon `state`, falling back to the boolean fields instead of
    /// guessing readiness from a partial payload.
    fn from_status(browser: &Value) -> BrowserState {
        match browser.get("state").and_then(Value::as_str) {
            Some("unsupported") => return BrowserState::Unsupported,
            Some("not-installed") => return BrowserState::NotInstalled,
            Some("native-missing") => return BrowserState::NativeMissing,
            Some("native-ready") => return BrowserState::NativeReady,
            Some("extension-installed") => return BrowserState::ExtensionInstalled,
            _ => {}
        }
        let flag = |key: &str| browser.get(key).and_then(Value::as_bool).unwrap_or(false);
        if browser.get("unsupported").and_then(Value::as_str).is_some() {
            BrowserState::Unsupported
        } else if browser.get("command").and_then(Value::as_str).is_none() {
            BrowserState::NotInstalled
        } else if flag("extension_installed") {
            BrowserState::ExtensionInstalled
        } else if flag("manifest_present") {
            BrowserState::NativeReady
        } else {
            BrowserState::NativeMissing
        }
    }
}

/// Which of the three contract actions the row offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BrowserActions {
    /// `extension.prepare` for the browser family.
    pub prepare: bool,
    /// `extension.open` for this browser.
    pub open: bool,
    /// Copy the prepared folder, only when the daemon reported one.
    pub copy_path: bool,
}

/// The sentence one browser state shows.
fn browser_status_text(locale: Locale, state: BrowserState, unsupported: Option<&str>) -> String {
    match state {
        BrowserState::Unsupported => tr(locale, Msg::BrowserStatusUnsupported)
            .replace("{reason}", unsupported.unwrap_or("snap/flatpak")),
        BrowserState::NotInstalled => tr(locale, Msg::BrowserStatusNotInstalled).to_string(),
        BrowserState::NativeMissing => tr(locale, Msg::BrowserStatusNoManifest).to_string(),
        BrowserState::NativeReady => tr(locale, Msg::BrowserStatusManifestOnly).to_string(),
        BrowserState::ExtensionInstalled => tr(locale, Msg::BrowserStatusInstalled).to_string(),
    }
}

/// Maps an `extension.status` answer to the rows the page paints.
///
/// Browsers the daemon did not find are skipped: a missing binary is not a row,
/// but an unsupported one is, because its limit is what E1 has to state.
pub fn browser_rows(locale: Locale, status: &Value) -> Vec<BrowserRow> {
    let Some(list) = status.get("browsers").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|browser| {
            let state = BrowserState::from_status(browser);
            if state == BrowserState::NotInstalled {
                return None;
            }
            let unsupported = browser.get("unsupported").and_then(Value::as_str);
            // A sandboxed binary can neither run the host nor be opened for the
            // extension page: offering either action would only fail.
            let usable = state != BrowserState::Unsupported;
            let prepared_path = browser
                .get("prepared_path")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(BrowserRow {
                id: browser
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                name: browser
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                family: browser
                    .get("family")
                    .and_then(Value::as_str)
                    .unwrap_or("chromium")
                    .to_string(),
                version: browser
                    .get("version")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                state,
                status: browser_status_text(locale, state, unsupported),
                actions: BrowserActions {
                    prepare: usable,
                    open: usable,
                    copy_path: usable && prepared_path.is_some(),
                },
                prepared_path,
            })
        })
        .collect()
}

/// Numbered how-to for loading an unpacked (Chromium) or temporary (Firefox)
/// extension. Empty for a family the page knows nothing about.
pub fn load_steps(locale: Locale, family: &str) -> Vec<String> {
    let steps: &[Msg] = match family {
        "chromium" => &[
            Msg::BrowserStepPrepare,
            Msg::BrowserStepChromiumPage,
            Msg::BrowserStepChromiumLoad,
            Msg::BrowserStepChromiumVerify,
        ],
        "firefox" => &[
            Msg::BrowserStepPrepare,
            Msg::BrowserStepFirefoxPage,
            Msg::BrowserStepFirefoxLoad,
        ],
        _ => &[],
    };
    steps
        .iter()
        .map(|step| tr(locale, *step).to_string())
        .collect()
}

/// Callbacks the Browsers page needs.
///
/// Keeping them out of the widget code lets the page be built in a test with
/// plain closures instead of an IPC client.
#[derive(Clone)]
pub struct BrowserHandlers {
    /// `extension.prepare` with the browser family.
    pub prepare: Rc<dyn Fn(&str)>,
    /// `extension.open` with the browser id.
    pub open: Rc<dyn Fn(&str)>,
    /// Copy the prepared folder into the clipboard.
    pub copy_path: Rc<dyn Fn(&str)>,
    /// `extension.install-native`.
    pub install_native: Rc<dyn Fn()>,
}

/// One painted browser row and its buttons.
pub struct BrowserRowView {
    /// The data the row was painted from.
    pub data: BrowserRow,
    /// The row itself, so a repaint can remove it from the group.
    pub row: adw::ActionRow,
    /// `Preparar extensión`, when the state allows it.
    pub prepare: Option<gtk::Button>,
    /// `Abrir página de extensiones`, when the state allows it.
    pub open: Option<gtk::Button>,
    /// `Copiar ruta`, only when a prepared folder exists.
    pub copy_path: Option<gtk::Button>,
}

/// The whole Navegadores page.
pub struct BrowsersPage {
    /// The page to add to the dialog.
    pub page: adw::PreferencesPage,
    /// The group that holds the browser rows.
    pub group: adw::PreferencesGroup,
    /// Painted browser rows, in daemon order.
    pub rows: Vec<BrowserRowView>,
    /// Numbered step rows of the how-to.
    pub steps: Vec<adw::ActionRow>,
    /// Notes under the list: manifest is not an install, no store, no signature.
    pub hints: Vec<gtk::Label>,
}

fn hint_label(locale: Locale, msg: Msg) -> gtk::Label {
    gtk::Label::builder()
        .label(tr(locale, msg))
        .xalign(0.0)
        .wrap(true)
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(6)
        .css_classes(["dim-label"])
        .build()
}

/// Paints the browser rows of one `extension.status` answer into `group`.
pub fn fill_browser_rows(
    group: &adw::PreferencesGroup,
    locale: Locale,
    status: &Value,
    handlers: &BrowserHandlers,
) -> Vec<BrowserRowView> {
    let mut views = Vec::new();
    for data in browser_rows(locale, status) {
        let row = adw::ActionRow::builder()
            .title(&data.name)
            .subtitle(&data.status)
            .build();
        if let Some(version) = &data.version {
            row.add_suffix(
                &gtk::Label::builder()
                    .label(version)
                    .css_classes(["dim-label"])
                    .build(),
            );
        }

        let mut prepare = None;
        if data.actions.prepare {
            let button = gtk::Button::with_label(tr(locale, Msg::ExtensionPrepare));
            let handler = Rc::clone(&handlers.prepare);
            let family = data.family.clone();
            button.connect_clicked(move |_| handler(&family));
            row.add_suffix(&button);
            prepare = Some(button);
        }

        let mut open = None;
        if data.actions.open {
            let button = gtk::Button::with_label(tr(locale, Msg::ExtensionOpen));
            let handler = Rc::clone(&handlers.open);
            let browser_id = data.id.clone();
            button.connect_clicked(move |_| handler(&browser_id));
            row.add_suffix(&button);
            open = Some(button);
        }

        let mut copy_path = None;
        if let (true, Some(path)) = (data.actions.copy_path, data.prepared_path.clone()) {
            let button = gtk::Button::with_label(tr(locale, Msg::ExtensionCopyPath));
            let handler = Rc::clone(&handlers.copy_path);
            button.connect_clicked(move |_| handler(&path));
            row.add_suffix(&button);
            copy_path = Some(button);
        }

        group.add(&row);
        views.push(BrowserRowView {
            data,
            row,
            prepare,
            open,
            copy_path,
        });
    }
    views
}

/// Builds the Navegadores page: install row, state rows, how-to and the two
/// limits (a manifest is not an installation; there is no store and no
/// signature).
pub fn browsers_page(locale: Locale, status: &Value, handlers: BrowserHandlers) -> BrowsersPage {
    let page = adw::PreferencesPage::builder()
        .title(tr(locale, Msg::PrefsBrowsers))
        .icon_name("applications-internet-symbolic")
        .build();
    let group = adw::PreferencesGroup::builder()
        .title(tr(locale, Msg::PrefsBrowsers))
        .description(tr(locale, Msg::BrowserInstallHint))
        .build();

    let install_row = adw::ActionRow::builder()
        .title(tr(locale, Msg::InstallNative))
        .build();
    let install_button = gtk::Button::with_label(tr(locale, Msg::InstallNative));
    install_button.add_css_class("suggested-action");
    install_row.add_suffix(&install_button);
    install_row.set_activatable_widget(Some(&install_button));
    {
        let handler = Rc::clone(&handlers.install_native);
        install_button.connect_clicked(move |_| handler());
    }
    group.add(&install_row);
    let rows = fill_browser_rows(&group, locale, status, &handlers);
    page.add(&group);

    // The how-to is the manual: two families, numbered steps, no store needed.
    let how_to = adw::PreferencesGroup::builder()
        .title(tr(locale, Msg::BrowserHowTo))
        .build();
    let mut steps = Vec::new();
    for (family, header) in [
        ("chromium", Msg::BrowserHowToChromium),
        ("firefox", Msg::BrowserHowToFirefox),
    ] {
        let head = adw::ActionRow::builder().title(tr(locale, header)).build();
        head.add_css_class("heading");
        how_to.add(&head);
        steps.push(head);
        for step in load_steps(locale, family) {
            let row = adw::ActionRow::builder().title(step).build();
            how_to.add(&row);
            steps.push(row);
        }
    }
    page.add(&how_to);

    let hints = vec![
        hint_label(locale, Msg::BrowserInstallNotAnInstallation),
        hint_label(locale, Msg::BrowserNoStore),
    ];
    let notes = adw::PreferencesGroup::new();
    for hint in &hints {
        notes.add(hint);
    }
    page.add(&notes);

    let local_group = adw::PreferencesGroup::builder()
        .title(tr(locale, Msg::ExtensionLocalTitle))
        .build();
    local_group.add(
        &adw::ActionRow::builder()
            .title(tr(locale, Msg::ExtensionLocalHint))
            .subtitle("/usr/share/segmenta/extensions")
            .build(),
    );
    page.add(&local_group);

    BrowsersPage {
        page,
        group,
        rows,
        steps,
        hints,
    }
}

/// Antivirus section: the engine is external, so the switch, the timeout and the
/// quarantine folder are the only things the GUI owns (OpenSpec 12 §5).
pub struct AntivirusPage {
    pub page: adw::PreferencesPage,
    pub group: adw::PreferencesGroup,
    pub enabled: adw::SwitchRow,
    pub timeout: adw::SpinRow,
    pub quarantine: adw::ActionRow,
}

/// Builds the Antivirus page.
///
/// `save` receives the `settings.update` patch for one control; the caller decides how to
/// send it, so the page stays testable without a daemon.
pub fn antivirus_page(
    _parent: &adw::ApplicationWindow,
    settings: &Settings,
    save: std::rc::Rc<dyn Fn(serde_json::Value)>,
) -> AntivirusPage {
    let page = adw::PreferencesPage::builder()
        .title(tr(locale(), Msg::PrefsAntivirus))
        .icon_name("security-high-symbolic")
        .build();
    let group = adw::PreferencesGroup::builder()
        .title(tr(locale(), Msg::PrefsAntivirus))
        .description(tr(locale(), Msg::AntivirusHint))
        .build();

    let enabled = adw::SwitchRow::builder()
        .title(tr(locale(), Msg::AntivirusEnabled))
        .subtitle(tr(locale(), Msg::AntivirusEnabledHint))
        .active(settings.antivirus_enabled)
        .build();
    {
        let save = std::rc::Rc::clone(&save);
        enabled.connect_active_notify(move |row| {
            save(serde_json::json!({ "antivirus_enabled": row.is_active() }));
        });
    }
    group.add(&enabled);

    let timeout = adw::SpinRow::with_range(1.0, 3600.0, 1.0);
    timeout.set_title(tr(locale(), Msg::AntivirusTimeout));
    timeout.set_subtitle(tr(locale(), Msg::AntivirusTimeoutHint));
    timeout.set_value(settings.antivirus_timeout_secs.clamp(1, 3600) as f64);
    {
        let save = std::rc::Rc::clone(&save);
        timeout.connect_value_notify(move |row| {
            let seconds = row.value().clamp(1.0, 3600.0) as u64;
            save(serde_json::json!({ "antivirus_timeout_secs": seconds }));
        });
    }
    group.add(&timeout);

    let quarantine = adw::ActionRow::builder()
        .title(tr(locale(), Msg::AntivirusQuarantineDir))
        .subtitle(settings.antivirus_quarantine_dir.clone())
        .activatable(true)
        .build();
    quarantine.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
    group.add(&quarantine);

    page.add(&group);
    AntivirusPage {
        page,
        group,
        enabled,
        timeout,
        quarantine,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_tokens;

    #[test]
    fn tokens_split_on_commas_spaces_and_newlines() {
        assert_eq!(
            parse_tokens("mp4, PDF\nr0*  mp4"),
            vec!["mp4".to_string(), "PDF".to_string(), "r0*".to_string()]
        );
        assert!(parse_tokens("  \n , ").is_empty());
    }
}
