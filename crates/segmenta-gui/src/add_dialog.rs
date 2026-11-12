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

struct Form {
    dialog: adw::Dialog,
    url: adw::EntryRow,
    filename: adw::EntryRow,
    destination: adw::ActionRow,
    connections: adw::SpinRow,
    start_now: adw::SwitchRow,
    error: gtk::Label,
    add_button: gtk::Button,
}

/// Opens the dialog over `parent`.
pub fn present(
    parent: &adw::ApplicationWindow,
    client: Arc<dyn Client>,
    defaults: AddDefaults,
    on_added: Rc<dyn Fn(String)>,
) -> adw::Dialog {
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
    let directory = Rc::new(RefCell::new(defaults.destination_dir.clone()));
    destination.set_subtitle(&defaults.destination_dir);

    let connections = adw::SpinRow::with_range(1.0, 32.0, 1.0);
    connections.set_title(tr(locale(), Msg::Connections));
    connections.set_subtitle(tr(locale(), Msg::ConnectionsHint));
    connections.set_value(f64::from(defaults.connections.clamp(1, 32)));

    let start_now = adw::SwitchRow::builder()
        .title(tr(locale(), Msg::StartNow))
        .subtitle(tr(locale(), Msg::StartNowHint))
        .active(true)
        .build();

    let error = gtk::Label::new(None);
    error.add_css_class("error");
    error.set_wrap(true);
    error.set_xalign(0.0);
    error.set_visible(false);

    group.add(&url);
    group.add(&filename);
    group.add(&destination);
    group.add(&connections);
    group.add(&start_now);
    group.add(&error);

    let page = adw::PreferencesPage::new();
    page.add(&group);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    let form = Rc::new(Form {
        dialog: dialog.clone(),
        url,
        filename,
        destination,
        connections,
        start_now,
        error,
        add_button: add_button.clone(),
    });

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

    let close_form = Rc::downgrade(&form);
    cancel_button.connect_clicked(move |_| {
        if let Some(form) = close_form.upgrade() {
            form.dialog.close();
        }
    });

    let submit_form = Rc::clone(&form);
    let submit_client = Arc::clone(&client);
    let submit_added = Rc::clone(&on_added);
    add_button.connect_clicked(move |_| {
        submit(
            &submit_form,
            Arc::clone(&submit_client),
            Rc::clone(&submit_added),
        );
    });

    // Enter on the entries submits.
    let enter_form = Rc::clone(&form);
    let enter_client = Arc::clone(&client);
    form.url.connect_entry_activated(move |_| {
        submit(&enter_form, Arc::clone(&enter_client), Rc::clone(&on_added));
    });

    dialog.present(Some(parent));
    dialog.clone()
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

    let mut params = json!({
        "url": url,
        "connections": form.connections.value() as u32,
        "start_immediately": form.start_now.is_active(),
        "queue_name": "main",
        "priority": 0
    });
    let destination = form.destination.subtitle().unwrap_or_default();
    if !destination.trim().is_empty() {
        params["destination_dir"] = Value::String(destination.to_string());
    }
    let filename = form.filename.text().trim().to_string();
    let filename = if filename.is_empty() {
        filename_from_url(&url).unwrap_or_else(|| "descarga".to_string())
    } else {
        filename
    };
    params["filename"] = Value::String(filename.clone());

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
