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

/// Opens the preferences dialog over `parent`.
pub fn present(
    parent: &adw::ApplicationWindow,
    client: Arc<dyn Client>,
    settings: Settings,
    on_saved: Rc<dyn Fn(Settings)>,
) {
    let dialog = adw::PreferencesDialog::builder()
        .title("Preferencias")
        .content_width(640)
        .content_height(720)
        .search_enabled(true)
        .build();

    let bus = Rc::new(RequestBus::new());

    // ---------------------------------------------------------------- General
    let general = adw::PreferencesPage::builder()
        .title("General")
        .icon_name("emblem-system-symbolic")
        .build();
    let storage = adw::PreferencesGroup::builder().title("Carpetas").build();

    let download_dir = adw::ActionRow::builder()
        .title("Carpeta de descargas")
        .subtitle(settings.download_dir.clone())
        .activatable(true)
        .build();
    download_dir.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
    storage.add(&download_dir);

    let categories = adw::ExpanderRow::builder()
        .title("Carpetas por categoría")
        .subtitle("Configuradas en el daemon; se muestran en modo lectura")
        .build();
    if settings.category_dirs.is_empty() {
        let empty = adw::ActionRow::builder()
            .title("Sin categorías configuradas")
            .subtitle("El daemon usa la carpeta de descargas para todo")
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
        .title("Iniciar Segmenta con la sesión")
        .subtitle("Activa la unidad systemd --user del daemon")
        .active(settings.autostart)
        .build();
    session.add(&autostart);
    general.add(&session);

    // -------------------------------------------------- Connections and limits
    let network = adw::PreferencesPage::builder()
        .title("Conexiones y límites")
        .icon_name("network-transmit-receive-symbolic")
        .build();
    let limits = adw::PreferencesGroup::builder()
        .title("Transferencia")
        .description("Los valores fuera de rango son rechazados por el daemon")
        .build();

    let max_connections = adw::SpinRow::with_range(1.0, 32.0, 1.0);
    max_connections.set_title("Conexiones por descarga");
    max_connections.set_subtitle("De 1 a 32 conexiones paralelas");
    max_connections.set_value(f64::from(settings.max_connections.clamp(1, 32)));
    limits.add(&max_connections);

    let speed_limit = adw::SpinRow::with_range(0.0, 1_000_000.0, 128.0);
    speed_limit.set_title("Límite global de velocidad");
    speed_limit.set_subtitle("KiB/s · 0 significa sin límite");
    speed_limit.set_value(settings.speed_limit_kib() as f64);
    limits.add(&speed_limit);

    let retries = adw::SpinRow::with_range(0.0, 50.0, 1.0);
    retries.set_title("Reintentos por tarea");
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
        .title("Permitir descargas locales")
        .subtitle("Direcciones de loopback y redes privadas")
        .active(settings.allow_local_downloads)
        .build();
    proxy.add(&local);
    network.add(&proxy);

    // -------------------------------------------------------------- About page
    let about = adw::PreferencesPage::builder()
        .title("Acerca de")
        .icon_name("help-about-symbolic")
        .build();
    let info = adw::PreferencesGroup::new();
    let version = adw::ActionRow::builder()
        .title("Segmenta")
        .subtitle(format!("Versión {}", env!("CARGO_PKG_VERSION")))
        .build();
    info.add(&version);
    let license = adw::ActionRow::builder()
        .title("Licencia")
        .subtitle("GPL-3.0-or-later")
        .build();
    info.add(&license);
    let website = adw::ActionRow::builder()
        .title("Sitio web")
        .subtitle("github.com/wilkin-barban/Segmenta")
        .activatable(true)
        .build();
    info.add(&website);
    let protocol = adw::ActionRow::builder()
        .title("Protocolo")
        .subtitle("JSON-RPC 2.0 sobre NDJSON · versión 1")
        .build();
    info.add(&protocol);
    // `system.status` is informational: it fills this row and never blocks the
    // dialog while the daemon answers.
    let daemon_row = adw::ActionRow::builder()
        .title("Daemon")
        .subtitle("Consultando…")
        .build();
    info.add(&daemon_row);
    about.add(&info);

    dialog.add(&general);
    dialog.add(&network);
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
            .title("Elegir carpeta de descargas")
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
