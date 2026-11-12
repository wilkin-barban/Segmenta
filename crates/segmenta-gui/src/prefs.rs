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
use segmenta_i18n::{tr, Msg};

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

    let categories = adw::ExpanderRow::builder()
        .title(tr(locale(), Msg::CategoriesReadOnly))
        .subtitle(tr(locale(), Msg::CategoriesHint))
        .build();
    if settings.category_dirs.is_empty() {
        let empty = adw::ActionRow::builder()
            .title(tr(locale(), Msg::NoCategories))
            .subtitle(tr(locale(), Msg::DownloadFolderHint))
            .build();
        categories.add_row(&empty);
    } else {
        for (category, folder) in &settings.category_dirs {
            let row = adw::ActionRow::builder()
                .title(category.clone())
                .subtitle(folder.clone())
                .build();
            categories.add_row(&row);
        }
    }
    storage.add(&categories);
    general.add(&storage);

    let session = adw::PreferencesGroup::builder().title("Sesión").build();
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
        .title("Modo")
        .subtitle(&settings.proxy_mode)
        .build();
    proxy.add(&proxy_mode);
    let proxy_url = adw::ActionRow::builder()
        .title("Servidor")
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
    let browsers_page = adw::PreferencesPage::builder()
        .title(tr(locale(), Msg::PrefsBrowsers))
        .icon_name("applications-internet-symbolic")
        .build();
    let browsers_group = adw::PreferencesGroup::builder()
        .title(tr(locale(), Msg::PrefsBrowsers))
        .description(tr(locale(), Msg::BrowserInstallHint))
        .build();
    browser_bus.spawn(&client, "extension.status", json!({}));
    let install_row = adw::ActionRow::builder()
        .title(tr(locale(), Msg::InstallNative))
        .build();
    let install_button = gtk::Button::with_label(tr(locale(), Msg::InstallNative));
    install_button.add_css_class("suggested-action");
    install_row.add_suffix(&install_button);
    install_row.set_activatable_widget(Some(&install_button));
    browsers_group.add(&install_row);

    let local_group = adw::PreferencesGroup::builder()
        .title(tr(locale(), Msg::ExtensionLocalTitle))
        .build();
    local_group.add(
        &adw::ActionRow::builder()
            .title(tr(locale(), Msg::ExtensionLocalHint))
            .subtitle("/usr/share/segmenta/extensions")
            .build(),
    );
    browsers_page.add(&browsers_group);
    browsers_page.add(&local_group);

    let install_bus = Rc::clone(&browser_bus);
    let install_client = Arc::clone(&client);
    install_button.connect_clicked(move |_| {
        install_bus.spawn(&install_client, "extension.install-native", json!({}));
    });

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
        .subtitle(format!("Versión {}", env!("CARGO_PKG_VERSION")))
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
    dialog.add(&browsers_page);
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

    // Browsers poll: `extension.status` fills the list, `install-native` reports back.
    // Rows added by the last status answer, so a refresh replaces instead of appends.
    let dynamic_rows: Rc<std::cell::RefCell<Vec<adw::ActionRow>>> =
        Rc::new(std::cell::RefCell::new(Vec::new()));
    let dynamic_rows_poll = Rc::clone(&dynamic_rows);
    let refresh_poll_bus = Rc::clone(&browser_bus);
    let refresh_poll_client = Arc::clone(&client);
    let browser_poll = Rc::clone(&browser_bus);
    let browser_group_poll = browsers_group.clone();
    let browser_dialog_poll = dialog.clone();
    let browser_open_poll = Rc::clone(&open);
    let browser_locale = locale();
    let dialog_for_browser = dialog.clone();
    let client_for_browser = Arc::clone(&client);
    gtk::glib::timeout_add_local(Duration::from_millis(150), move || {
        if !browser_open_poll.get() {
            return gtk::glib::ControlFlow::Break;
        }
        let dialog = dialog_for_browser.clone();
        let client = Arc::clone(&client_for_browser);
        for (_, result) in browser_poll.take() {
            match result {
                Ok(value) => {
                    if let Some(list) = value.get("browsers").and_then(Value::as_array) {
                        for row in dynamic_rows_poll.borrow_mut().drain(..) {
                            browser_group_poll.remove(&row);
                        }
                        for browser in list {
                            let installed =
                                browser.get("command").and_then(Value::as_str).is_some();
                            if !installed {
                                continue;
                            }
                            let name = browser.get("name").and_then(Value::as_str).unwrap_or("?");
                            // State comes from the daemon; the UI never guesses readiness.
                            let state = browser
                                .get("state")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let status = match state {
                                "unsupported" => tr(browser_locale, Msg::BrowserUnsupported),
                                "extension-installed" => {
                                    tr(browser_locale, Msg::ExtensionInstalled)
                                }
                                "native-ready" => &format!(
                                    "{} · {}",
                                    tr(browser_locale, Msg::NativeReady),
                                    tr(browser_locale, Msg::ExtensionMissing)
                                ),
                                "native-missing" => tr(browser_locale, Msg::NativeMissing),
                                _ => tr(browser_locale, Msg::ExtensionMissing),
                            };
                            let row = adw::ActionRow::builder()
                                .title(name)
                                .subtitle(status)
                                .build();
                            if let Some(version) = browser.get("version").and_then(Value::as_str) {
                                row.add_suffix(
                                    &gtk::Label::builder()
                                        .label(version)
                                        .css_classes(["dim-label"])
                                        .build(),
                                );
                            }

                            let browser_id =
                                browser.get("id").and_then(Value::as_str).unwrap_or("");
                            let family =
                                browser.get("family").and_then(Value::as_str).unwrap_or("");
                            let prepared_path = browser
                                .get("prepared_path")
                                .and_then(Value::as_str)
                                .map(str::to_string);

                            let prep_btn =
                                gtk::Button::with_label(tr(browser_locale, Msg::ExtensionPrepare));
                            let prep_bus = browser_bus.clone();
                            let prep_client = Arc::clone(&client);
                            let fam_str = family.to_string();
                            prep_btn.connect_clicked(move |_| {
                                prep_bus.spawn(
                                    &prep_client,
                                    "extension.prepare",
                                    json!({ "family": fam_str }),
                                );
                            });
                            row.add_suffix(&prep_btn);

                            let open_btn =
                                gtk::Button::with_label(tr(browser_locale, Msg::ExtensionOpen));
                            let open_bus = browser_bus.clone();
                            let open_client = Arc::clone(&client);
                            let bid_str = browser_id.to_string();
                            open_btn.connect_clicked(move |_| {
                                open_bus.spawn(
                                    &open_client,
                                    "extension.open",
                                    json!({ "browser_id": bid_str }),
                                );
                            });
                            row.add_suffix(&open_btn);

                            if let Some(path) = prepared_path {
                                let copy_btn = gtk::Button::with_label(tr(
                                    browser_locale,
                                    Msg::ExtensionCopyPath,
                                ));
                                let row_for_clipboard = row.clone();
                                let toast_dialog = dialog.clone();
                                let locale_for_toast = browser_locale;
                                let p_clone = path.clone();
                                copy_btn.connect_clicked(move |_| {
                                    row_for_clipboard.clipboard().set_text(&p_clone);
                                    toast_dialog.add_toast(adw::Toast::new(tr(
                                        locale_for_toast,
                                        Msg::ExtensionPathCopied,
                                    )));
                                });
                                row.add_suffix(&copy_btn);
                            }

                            browser_group_poll.add(&row);
                            dynamic_rows_poll.borrow_mut().push(row);
                        }
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
