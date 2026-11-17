//! "Add download" dialog.
//!
//! Every visible string comes from `segmenta-i18n` (S21).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use adw::prelude::*;

use crate::client::{Client, ClientError, RequestBus};
use crate::model::{filename_from_url, looks_like_http_url, Settings};
use crate::{adw, gtk};
use segmenta_i18n::{tr, Msg};

fn locale() -> segmenta_i18n::Locale {
    segmenta_i18n::current()
}

/// Defaults taken from `settings.get`.
pub struct AddDefaults {
    pub destination_dir: String,
    pub connections: u32,
    /// `settings.get.clip_monitor_enabled`: the switch for the clipboard monitor.
    pub clip_monitor_enabled: bool,
}

impl Default for AddDefaults {
    fn default() -> Self {
        AddDefaults {
            destination_dir: String::new(),
            connections: Settings::default().max_connections.max(16),
            clip_monitor_enabled: Settings::default().clip_monitor_enabled,
        }
    }
}

pub struct Form {
    pub dialog: adw::Dialog,
    pub url: adw::EntryRow,
    pub filename: adw::EntryRow,
    pub destination: adw::ActionRow,
    pub connections: adw::SpinRow,
    pub schedule: adw::SwitchRow,
    pub schedule_when: adw::ComboRow,
    pub schedule_summary: adw::ActionRow,
    pub days_ahead: adw::SpinRow,
    pub schedule_hour: adw::SpinRow,
    pub schedule_minute: adw::SpinRow,
    pub schedule_hint: gtk::Label,
    pub error: gtk::Label,
    pub add_button: gtk::Button,
}

/// Builds the form widgets and layout.
pub fn build_form(defaults: &AddDefaults) -> Form {
    let dialog = adw::Dialog::builder()
        .title(tr(locale(), Msg::AddDownloadTitle))
        .content_width(540)
        .follows_content_size(true)
        .build();

    let header = adw::HeaderBar::new();
    let cancel_button = gtk::Button::with_label(tr(locale(), Msg::Cancel));
    header.pack_start(&cancel_button);
    let add_button = gtk::Button::with_label(tr(locale(), Msg::Start));
    add_button.add_css_class("suggested-action");
    header.pack_end(&add_button);

    let group = adw::PreferencesGroup::builder()
        .title(tr(locale(), Msg::AddDownloadSection))
        .description(tr(locale(), Msg::AddDownloadHint))
        .build();

    let url = adw::EntryRow::builder()
        .title(tr(locale(), Msg::UrlLabel))
        .show_apply_button(false)
        .build();
    let filename = adw::EntryRow::builder()
        .title(tr(locale(), Msg::Filename))
        .show_apply_button(false)
        .build();

    let destination = adw::ActionRow::builder()
        .title(tr(locale(), Msg::Destination))
        .activatable(true)
        .build();
    let folder_icon = gtk::Image::from_icon_name("folder-open-symbolic");
    destination.add_suffix(&folder_icon);
    destination.set_subtitle(&defaults.destination_dir);

    let connections = adw::SpinRow::with_range(1.0, 32.0, 1.0);
    connections.set_title(tr(locale(), Msg::Connections));
    connections.set_subtitle(tr(locale(), Msg::ConnectionsHint));
    connections.set_value(f64::from(defaults.connections.clamp(1, 32)));

    let schedule = adw::SwitchRow::builder()
        .title(tr(locale(), Msg::Schedule))
        .subtitle(tr(locale(), Msg::ScheduleHint))
        .active(false)
        .build();

    let items = [
        tr(locale(), Msg::ScheduleIn30Min),
        tr(locale(), Msg::ScheduleIn1Hour),
        tr(locale(), Msg::ScheduleIn2Hours),
        tr(locale(), Msg::ScheduleTonight),
        tr(locale(), Msg::ScheduleTomorrow),
        tr(locale(), Msg::ScheduleExactDateTime),
    ];
    let string_list = gtk::StringList::new(&items);
    let schedule_when = adw::ComboRow::builder()
        .title(tr(locale(), Msg::ScheduleWhen))
        .model(&string_list)
        .selected(1)
        .visible(false)
        .build();

    let schedule_summary = adw::ActionRow::builder().visible(false).build();

    let now = gtk::glib::DateTime::now_local().unwrap_or_else(|_| {
        gtk::glib::DateTime::from_unix_local(0).expect("fallback local DateTime")
    });
    let default_exact = round_up_5_minutes(&now.add_hours(1).unwrap_or_else(|_| now.clone()));

    // An offset in days plus a time cannot express the past, which a calendar always can:
    // picking an earlier day used to be corrected silently behind the user's back.
    let days_ahead = adw::SpinRow::with_range(0.0, 60.0, 1.0);
    days_ahead.set_title(tr(locale(), Msg::ScheduleInDays));
    days_ahead.set_value(0.0);
    days_ahead.set_visible(false);

    let schedule_hour = adw::SpinRow::with_range(0.0, 23.0, 1.0);
    schedule_hour.set_title(tr(locale(), Msg::Hour));
    schedule_hour.set_value(f64::from(default_exact.hour()));
    schedule_hour.set_visible(false);

    let schedule_minute = adw::SpinRow::with_range(0.0, 59.0, 1.0);
    schedule_minute.set_title(tr(locale(), Msg::Minute));
    schedule_minute.set_value(f64::from(default_exact.minute()));
    schedule_minute.set_visible(false);

    let schedule_hint = gtk::Label::builder()
        .label(tr(locale(), Msg::ScheduleOnlyFutureAllowed))
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label"])
        .visible(false)
        .build();

    let updating = Rc::new(std::cell::Cell::new(false));

    let f_sched = schedule.clone();
    let f_when = schedule_when.clone();
    let f_sum = schedule_summary.clone();
    let f_cal = days_ahead.clone();
    let f_hour = schedule_hour.clone();
    let f_min = schedule_minute.clone();
    let f_hint = schedule_hint.clone();
    let upd = Rc::clone(&updating);

    let sync = move || {
        if upd.get() {
            return;
        }
        upd.set(true);

        let active = f_sched.is_active();
        f_when.set_visible(active);
        f_sum.set_visible(active);

        if !active {
            f_cal.set_visible(false);
            f_hour.set_visible(false);
            f_min.set_visible(false);
            f_hint.set_visible(false);
            upd.set(false);
            return;
        }

        let is_exact = f_when.selected() == 5;
        f_cal.set_visible(is_exact);
        f_hour.set_visible(is_exact);
        f_min.set_visible(is_exact);

        let now = gtk::glib::DateTime::now_local().unwrap_or_else(|_| {
            gtk::glib::DateTime::from_unix_local(0).expect("fallback local DateTime")
        });
        // An offset cannot express the past; when it is zero the time is clamped to the
        // earliest valid instant so the summary never lies about what will be sent.
        if is_exact && (f_cal.value() as i32) == 0 {
            let earliest = round_up_5_minutes(&now.add_minutes(5).unwrap_or_else(|_| now.clone()));
            let (want_h, want_m) = (f_hour.value() as i32, f_min.value() as i32);
            if (want_h, want_m) < (earliest.hour(), earliest.minute()) {
                f_hour.set_value(f64::from(earliest.hour()));
                f_min.set_value(f64::from(earliest.minute()));
            }
        }
        f_hint.set_visible(false);

        let dt = resolve_moment(f_when.selected(), &f_cal, &f_hour, &f_min);
        let summary = format_summary_line(locale(), &dt);
        f_sum.set_title(&summary);

        upd.set(false);
    };

    let sync_rc = Rc::new(sync);

    let s = Rc::clone(&sync_rc);
    schedule.connect_active_notify(move |_| s());

    let s = Rc::clone(&sync_rc);
    schedule_when.connect_selected_notify(move |_| s());

    let s = Rc::clone(&sync_rc);
    days_ahead.connect_value_notify(move |_| s());

    let s = Rc::clone(&sync_rc);
    schedule_hour.connect_value_notify(move |_| s());

    let s = Rc::clone(&sync_rc);
    schedule_minute.connect_value_notify(move |_| s());

    // Initial sync of the summary text
    sync_rc();

    let error = gtk::Label::new(None);
    error.add_css_class("error");
    error.set_wrap(true);
    error.set_xalign(0.0);
    error.set_visible(false);

    group.add(&url);
    group.add(&filename);
    group.add(&destination);
    group.add(&connections);
    group.add(&schedule);
    group.add(&schedule_when);
    group.add(&schedule_summary);
    group.add(&days_ahead);
    group.add(&schedule_hour);
    group.add(&schedule_minute);
    group.add(&schedule_hint);
    group.add(&error);

    let page = adw::PreferencesPage::new();
    page.add(&group);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    let close_dialog = dialog.clone();
    cancel_button.connect_clicked(move |_| {
        close_dialog.close();
    });

    Form {
        dialog,
        url,
        filename,
        destination,
        connections,
        schedule,
        schedule_when,
        schedule_summary,
        days_ahead,
        schedule_hour,
        schedule_minute,
        schedule_hint,
        error,
        add_button,
    }
}

fn round_up_5_minutes(dt: &gtk::glib::DateTime) -> gtk::glib::DateTime {
    let unix = dt.to_unix();
    let rem = unix.rem_euclid(300);
    let rounded_unix = if rem == 0 { unix } else { unix + (300 - rem) };
    gtk::glib::DateTime::from_unix_local(rounded_unix).unwrap_or_else(|_| dt.clone())
}

fn day_name(locale: segmenta_i18n::Locale, day_of_week: i32) -> &'static str {
    let idx = (day_of_week.clamp(1, 7) - 1) as usize;
    match locale {
        segmenta_i18n::Locale::Es => ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"][idx],
        segmenta_i18n::Locale::PtBr => ["seg", "ter", "qua", "qui", "sex", "sáb", "dom"][idx],
        segmenta_i18n::Locale::En => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][idx],
    }
}

fn format_summary_line(locale: segmenta_i18n::Locale, dt: &gtk::glib::DateTime) -> String {
    let day = day_name(locale, dt.day_of_week());
    let date = format!("{:02}/{:02}", dt.day_of_month(), dt.month());
    let time = format!("{:02}:{:02}", dt.hour(), dt.minute());
    tr(locale, Msg::ScheduleSummary)
        .replace("{day}", day)
        .replace("{date}", &date)
        .replace("{time}", &time)
}

fn resolve_moment(
    selected_option: u32,
    days_ahead: &adw::SpinRow,
    hour_spin: &adw::SpinRow,
    minute_spin: &adw::SpinRow,
) -> gtk::glib::DateTime {
    let now = gtk::glib::DateTime::now_local().unwrap_or_else(|_| {
        gtk::glib::DateTime::from_unix_local(0).expect("fallback local DateTime")
    });
    match selected_option {
        0 => now.add_minutes(30).unwrap_or(now),
        1 => now.add_hours(1).unwrap_or(now),
        2 => now.add_hours(2).unwrap_or(now),
        3 => {
            let base = if now.hour() < 22 {
                now.clone()
            } else {
                now.add_days(1).unwrap_or_else(|_| now.clone())
            };
            gtk::glib::DateTime::from_local(
                base.year(),
                base.month(),
                base.day_of_month(),
                22,
                0,
                0.0,
            )
            .unwrap_or(now)
        }
        4 => {
            let tomorrow = now.add_days(1).unwrap_or_else(|_| now.clone());
            gtk::glib::DateTime::from_local(
                tomorrow.year(),
                tomorrow.month(),
                tomorrow.day_of_month(),
                8,
                0,
                0.0,
            )
            .unwrap_or(now)
        }
        _ => {
            // Offset in days from today plus a time: the result is future by construction.
            let day = days_ahead.value().max(0.0) as i32;
            let base = now.add_days(day).unwrap_or_else(|_| now.clone());
            let hour = hour_spin.value() as i32;
            let minute = minute_spin.value() as i32;
            let picked = gtk::glib::DateTime::from_local(
                base.year(),
                base.month(),
                base.day_of_month(),
                hour,
                minute,
                0.0,
            )
            .unwrap_or_else(|_| now.clone());
            let earliest = round_up_5_minutes(&now.add_minutes(5).unwrap_or_else(|_| now.clone()));
            if picked.to_unix() < earliest.to_unix() {
                earliest
            } else {
                picked
            }
        }
    }
}

pub fn resolve_scheduled_moment(form: &Form) -> gtk::glib::DateTime {
    resolve_moment(
        form.schedule_when.selected(),
        &form.days_ahead,
        &form.schedule_hour,
        &form.schedule_minute,
    )
}

/// Builds the `download.add` request parameters from the form state.
pub fn build_add_params(form: &Form) -> Value {
    let url = form.url.text().trim().to_string();
    let destination = form.destination.subtitle().unwrap_or_default();
    let filename = form.filename.text().trim().to_string();
    let filename = if filename.is_empty() {
        filename_from_url(&url).unwrap_or_else(|| "descarga".to_string())
    } else {
        filename
    };

    let start_immediately = !form.schedule.is_active();
    let mut params = json!({
        "url": url,
        "connections": form.connections.value() as u32,
        "start_immediately": start_immediately,
        "queue_name": "main",
        "priority": 0,
        "filename": filename,
    });
    if !destination.trim().is_empty() {
        params["destination_dir"] = Value::String(destination.to_string());
    }

    if form.schedule.is_active() {
        let dt = resolve_scheduled_moment(form);
        let scheduled_str = if let Ok(utc) = dt.to_utc() {
            if let Ok(iso) = utc.format("%Y-%m-%dT%H:%M:%SZ") {
                iso.to_string()
            } else {
                format!(
                    "{:04}-{:02}-{:02}T{:02}:{:02}:00Z",
                    dt.year(),
                    dt.month(),
                    dt.day_of_month(),
                    dt.hour(),
                    dt.minute()
                )
            }
        } else {
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:00Z",
                dt.year(),
                dt.month(),
                dt.day_of_month(),
                dt.hour(),
                dt.minute()
            )
        };
        params["scheduled_at"] = Value::String(scheduled_str);
    }
    params
}

/// Opens the dialog over `parent`.
pub fn present(
    parent: &adw::ApplicationWindow,
    client: Arc<dyn Client>,
    defaults: AddDefaults,
    on_added: Rc<dyn Fn(String)>,
) -> adw::Dialog {
    let form = Rc::new(build_form(&defaults));
    let directory = Rc::new(RefCell::new(defaults.destination_dir.clone()));

    // Clipboard prefill: only when `clip_monitor_enabled` is on and the text already
    // looks like a download URL.
    prefill_from_clipboard(&form, defaults.clip_monitor_enabled);

    // Filename suggestion: the URL basename fills the entry until the user
    // types a name of their own.
    let suggested: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let touched = Rc::new(std::cell::Cell::new(false));
    let suggestion_state = Rc::clone(&suggested);
    let touched_state = Rc::clone(&touched);
    form.filename.connect_changed(move |row| {
        let text = row.text().to_string();
        if !text.is_empty() && text != *suggestion_state.borrow() {
            touched_state.set(true);
        }
    });
    let suggestion_state = Rc::clone(&suggested);
    let suggestion_form = Rc::downgrade(&form);
    form.url.connect_changed(move |row| {
        let Some(form) = suggestion_form.upgrade() else {
            return;
        };
        let url = row.text().to_string();
        if touched.get() || url.is_empty() {
            return;
        }
        let derived = filename_from_url(&url).unwrap_or_default();
        *suggestion_state.borrow_mut() = derived.clone();
        form.filename.set_text(&derived);
    });

    // Destination folder picker (native portal dialog, async).
    let form_handle = Rc::downgrade(&form);
    let parent_window = parent.clone();
    let directory_handle = Rc::clone(&directory);
    form.destination.connect_activated(move |_| {
        let Some(form) = form_handle.upgrade() else {
            return;
        };
        let chooser = gtk::FileDialog::builder()
            .title(tr(locale(), Msg::ChooseFolderTitle))
            .modal(true)
            .build();
        let form = Rc::clone(&form);
        let directory = Rc::clone(&directory_handle);
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
                *directory.borrow_mut() = path.clone();
                form.destination.set_subtitle(&path);
            },
        );
    });

    let submit_form = Rc::clone(&form);
    let submit_client = Arc::clone(&client);
    let submit_added = Rc::clone(&on_added);
    form.add_button.connect_clicked(move |_| {
        submit(
            &submit_form,
            Arc::clone(&submit_client),
            Rc::clone(&submit_added),
        );
    });

    // Enter on the entries submits.
    let enter_form = Rc::clone(&form);
    let enter_client = Arc::clone(&client);
    let enter_added = Rc::clone(&on_added);
    form.url.connect_entry_activated(move |_| {
        submit(
            &enter_form,
            Arc::clone(&enter_client),
            Rc::clone(&enter_added),
        );
    });

    form.dialog.present(Some(parent));
    form.dialog.clone()
}

/// One semantic, documented once: with `clip_monitor_enabled` off the dialog never
/// pre-fills from the clipboard; with it on the prefill only fills an empty entry and
/// only when the clipboard already holds an http(s) URL.
fn should_prefill_from_clipboard(
    clip_monitor_enabled: bool,
    current_url: &str,
    text: &str,
) -> bool {
    clip_monitor_enabled && current_url.trim().is_empty() && looks_like_http_url(text)
}

fn prefill_from_clipboard(form: &Rc<Form>, clip_monitor_enabled: bool) {
    if !clip_monitor_enabled {
        return;
    }
    let clipboard = form.dialog.clipboard();
    let form = Rc::downgrade(form);
    gtk::glib::spawn_future_local(async move {
        let Ok(text) = clipboard.read_text_future().await else {
            return;
        };
        let Some(text) = text else {
            return;
        };
        let Some(form) = form.upgrade() else {
            return;
        };
        if !should_prefill_from_clipboard(clip_monitor_enabled, &form.url.text(), &text) {
            return;
        }
        form.url.set_text(text.trim());
        if form.filename.text().is_empty() {
            if let Some(name) = filename_from_url(text.trim()) {
                form.filename.set_text(&name);
            }
        }
    });
}

fn submit(form: &Rc<Form>, client: Arc<dyn Client>, on_added: Rc<dyn Fn(String)>) {
    let url = form.url.text().trim().to_string();
    if !looks_like_http_url(&url) {
        show_error(form, tr(locale(), Msg::InvalidUrl));
        return;
    }

    let params = build_add_params(form);
    let filename = params["filename"]
        .as_str()
        .unwrap_or("descarga")
        .to_string();

    form.error.set_visible(false);
    form.add_button.set_sensitive(false);

    let bus = RequestBus::new();
    bus.spawn(&client, "download.add", params);

    let form = Rc::downgrade(form);
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    gtk::glib::timeout_add_local(Duration::from_millis(120), move || {
        let Some(form) = form.upgrade() else {
            return gtk::glib::ControlFlow::Break;
        };
        if let Some((_, result)) = bus.take().into_iter().next() {
            match result {
                Ok(value) => {
                    let name = value
                        .get("filename")
                        .and_then(Value::as_str)
                        .unwrap_or(&filename)
                        .to_string();
                    form.dialog.close();
                    on_added(name);
                }
                Err(err) => report_error(&form, &err),
            }
            return gtk::glib::ControlFlow::Break;
        }
        if std::time::Instant::now() >= deadline {
            report_error(
                &form,
                &ClientError::Unavailable("el daemon no respondió a tiempo".to_string()),
            );
            return gtk::glib::ControlFlow::Break;
        }
        gtk::glib::ControlFlow::Continue
    });
}

fn show_error(form: &Form, message: &str) {
    form.error.set_label(message);
    form.error.set_visible(true);
}

fn report_error(form: &Form, error: &ClientError) {
    show_error(form, &error.ui_message());
    form.add_button.set_sensitive(true);
}

#[cfg(test)]
mod tests {
    use super::should_prefill_from_clipboard;

    const URL: &str = "https://example.com/file.iso";

    #[test]
    fn the_clipboard_prefill_honours_clip_monitor_enabled() {
        // The setting is the switch for the whole clipboard monitor, so with it off
        // the dialog never pre-fills, even for a perfect URL.
        assert!(!should_prefill_from_clipboard(false, "", URL));
        assert!(should_prefill_from_clipboard(true, "", URL));
        // With it on, the prefill keeps its old guards: the entry stays empty and
        // the clipboard text has to look like an http(s) URL already.
        assert!(!should_prefill_from_clipboard(
            true,
            "https://other.test/x",
            URL
        ));
        assert!(!should_prefill_from_clipboard(true, "", "not a url"));
    }
}
