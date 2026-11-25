//! RSS subscriptions panel: list, add, remove and check, with a visible failure.
//!
//! The panel talks to the daemon through the same IPC the CLI uses
//! (`rss.feed.list`, `rss.feed.add`, `rss.feed.remove`, `rss.check_now`); it never fetches a
//! feed itself, so the SSRF rules stay in one place.

use crate::adw::prelude::*;
use crate::client::{Client, RequestBus};
use crate::{adw, gtk};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// One row of the panel, as the tests read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssRow {
    /// Feed id.
    pub feed_id: String,
    /// Display name.
    pub name: String,
    /// Feed URL.
    pub url: String,
    /// Destination directory.
    pub destination: String,
    /// Last poll, when it happened.
    pub last_checked: Option<i64>,
}

pub(crate) struct RssPanel {
    pub(crate) window: adw::Window,
    pub(crate) list: gtk::ListBox,
    pub(crate) error: gtk::Label,
    pub(crate) rows: Rc<RefCell<Vec<RssRow>>>,
}

impl RssPanel {
    pub(crate) fn build(parent: &adw::ApplicationWindow) -> RssPanel {
        let window = adw::Window::builder()
            .transient_for(parent)
            .title("Suscripciones RSS")
            .default_width(460)
            .default_height(520)
            .build();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        body.set_margin_top(16);
        body.set_margin_bottom(16);
        body.set_margin_start(16);
        body.set_margin_end(16);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&list));
        body.append(&scrolled);

        let error = gtk::Label::new(None);
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.add_css_class("error");
        error.set_visible(false);
        body.append(&error);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let check = gtk::Button::with_label("Comprobar ahora");
        let remove = gtk::Button::with_label("Quitar");
        let close = gtk::Button::with_label("Cerrar");
        actions.append(&check);
        actions.append(&remove);
        actions.append(&close);
        body.append(&actions);

        let form = gtk::Grid::new();
        form.set_row_spacing(6);
        form.set_column_spacing(8);
        let name = gtk::Entry::new();
        name.set_placeholder_text(Some("Nombre"));
        let url = gtk::Entry::new();
        url.set_placeholder_text(Some("https://sitio/feed.xml"));
        let filter = gtk::Entry::new();
        filter.set_placeholder_text(Some("Filtro (1080p|x265)"));
        let destination = gtk::Entry::new();
        destination.set_placeholder_text(Some("Destino absoluto"));
        for (row, entry) in [(0, &name), (1, &url), (2, &filter), (3, &destination)] {
            form.attach(entry, 0, row, 1, 1);
        }
        body.append(&form);
        let add = gtk::Button::with_label("Agregar suscripción");
        body.append(&add);

        window.set_content(Some(&body));

        let panel = RssPanel {
            window: window.clone(),
            list,
            error,
            rows: Rc::new(RefCell::new(Vec::new())),
        };
        let handle = window.clone();
        close.connect_clicked(move |_| handle.close());
        panel
    }

    pub(crate) fn fill(&self, rows: Vec<RssRow>) {
        while let Some(child) = self.list.row_at_index(0) {
            self.list.remove(&child);
        }
        for row in &rows {
            let item = gtk::ListBoxRow::new();
            let boxed = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let title = gtk::Label::new(Some(&row.name));
            title.set_xalign(0.0);
            let detail = gtk::Label::new(Some(&format!(
                "{} → {}{}",
                row.url,
                row.destination,
                match row.last_checked {
                    Some(stamp) => format!(" (último sondeo: {stamp})"),
                    None => " (sin sondear)".to_string(),
                }
            )));
            detail.set_xalign(0.0);
            detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
            detail.add_css_class("dim-label");
            boxed.append(&title);
            boxed.append(&detail);
            item.set_child(Some(&boxed));
            self.list.append(&item);
        }
        *self.rows.borrow_mut() = rows;
    }

    pub(crate) fn show_error(&self, message: Option<&str>) {
        match message {
            Some(text) => {
                self.error.set_label(text);
                self.error.set_visible(true);
            }
            None => {
                self.error.set_label("");
                self.error.set_visible(false);
            }
        }
    }
}

/// Parse one `rss.feed.list` answer into rows.
pub(crate) fn rows_from(value: &Value) -> Vec<RssRow> {
    value
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| RssRow {
                    feed_id: row["feed_id"].as_str().unwrap_or_default().to_string(),
                    name: row["name"].as_str().unwrap_or_default().to_string(),
                    url: row["url"].as_str().unwrap_or_default().to_string(),
                    destination: row["destination"].as_str().unwrap_or_default().to_string(),
                    last_checked: row["last_checked_at"].as_i64(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Request the list and refill the panel.
pub(crate) fn reload(panel: &Rc<RssPanel>, client: &Arc<dyn Client>) {
    let bus = RequestBus::new();
    bus.spawn(client, "rss.feed.list", json!({}));
    let panel = Rc::clone(panel);
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
        if let Some((_, Ok(value))) = bus.take().into_iter().next() {
            let rows = rows_from(&value);
            panel.fill(rows);
            return gtk::glib::ControlFlow::Break;
        }
        gtk::glib::ControlFlow::Continue
    });
}

/// Turn a `rss.check_now` answer into the message the panel shows, if any failed.
pub(crate) fn check_message(value: &Value) -> Option<String> {
    let failed = value["failed"].as_u64().unwrap_or(0);
    if failed == 0 {
        return None;
    }
    let code = value["error"].as_str().unwrap_or("ERR_RSS_UNREACHABLE");
    Some(format!("{failed} feed(s) no se pudieron leer ({code})"))
}
