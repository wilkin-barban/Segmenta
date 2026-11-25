//! Remote pairing dialog: mint a session, show it once, revoke it.
//!
//! The token is the only credential the remote API accepts, so the dialog is deliberately
//! explicit: the URL and the token are shown once, with a warning, and the only way back is
//! revoking and pairing again. The QR is drawn from the `qrcode` crate's matrix, with no
//! system tool and no image file on disk.

use crate::adw::prelude::*;
use crate::client::{Client, RequestBus};
use crate::{adw, gtk};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Widgets and last answers of the pairing dialog.
pub(crate) struct RemoteDialog {
    pub(crate) window: adw::Window,
    pub(crate) status: gtk::Label,
    pub(crate) url_row: gtk::Entry,
    pub(crate) token_row: gtk::Entry,
    pub(crate) warning: gtk::Label,
    pub(crate) picture: gtk::Picture,
    pub(crate) pair: gtk::Button,
    pub(crate) revoke: gtk::Button,
    /// Last pairing URL, shown once.
    pub(crate) pairing: Rc<RefCell<Option<String>>>,
    /// Last token, shown once.
    pub(crate) token: Rc<RefCell<Option<String>>>,
    /// Modules of the last QR matrix; zero when none was drawn.
    pub(crate) qr_modules: Rc<std::cell::Cell<u32>>,
}

impl RemoteDialog {
    /// Build the dialog over `parent`.
    pub(crate) fn build(parent: &adw::ApplicationWindow) -> RemoteDialog {
        let window = adw::Window::builder()
            .transient_for(parent)
            .modal(false)
            .title("Control remoto")
            .default_width(380)
            .build();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_margin_top(18);
        body.set_margin_bottom(18);
        body.set_margin_start(18);
        body.set_margin_end(18);

        let status = gtk::Label::new(Some("Consultando el estado…"));
        status.set_xalign(0.0);
        body.append(&status);

        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_size_request(220, 220);
        picture.set_visible(false);
        body.append(&picture);

        let url_row = gtk::Entry::new();
        url_row.set_editable(false);
        url_row.set_visible(false);
        body.append(&url_row);

        let token_row = gtk::Entry::new();
        token_row.set_editable(false);
        token_row.set_visible(false);
        body.append(&token_row);

        let warning = gtk::Label::new(Some(
            "Este token se muestra una sola vez. Guardalo ahora: no se puede volver a leer.",
        ));
        warning.set_xalign(0.0);
        warning.set_wrap(true);
        warning.add_css_class("dim-label");
        warning.set_visible(false);
        body.append(&warning);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let pair = gtk::Button::with_label("Emparejar");
        let revoke = gtk::Button::with_label("Revocar");
        revoke.add_css_class("destructive-action");
        actions.append(&pair);
        actions.append(&revoke);
        body.append(&actions);

        let close = gtk::Button::with_label("Cerrar");
        body.append(&close);

        window.set_content(Some(&body));

        let dialog = RemoteDialog {
            window: window.clone(),
            status,
            url_row,
            token_row,
            warning,
            picture,
            pair,
            revoke,
            pairing: Rc::new(RefCell::new(None)),
            token: Rc::new(RefCell::new(None)),
            qr_modules: Rc::new(std::cell::Cell::new(0)),
        };
        let handle = window.clone();
        close.connect_clicked(move |_| handle.close());
        dialog
    }
}

/// Draw `payload` as a QR into `picture`, returning the matrix side in modules.
pub(crate) fn paint_qr(picture: &gtk::Picture, payload: &str) -> u32 {
    let Ok(code) = qrcode::QrCode::new(payload.as_bytes()) else {
        picture.set_visible(false);
        return 0;
    };
    let width = code.width();
    // One pixel per module plus a quiet zone, which is what scanners expect.
    let quiet = 4usize;
    let side = width + quiet * 2;
    let mut pixels = vec![255u8; side * side * 4];
    let colors = code.to_colors();
    for (index, color) in colors.iter().enumerate() {
        if *color != qrcode::Color::Dark {
            continue;
        }
        let x = index % width + quiet;
        let y = index / width + quiet;
        let offset = (y * side + x) * 4;
        pixels[offset] = 0;
        pixels[offset + 1] = 0;
        pixels[offset + 2] = 0;
        pixels[offset + 3] = 255;
    }
    let bytes = gtk::glib::Bytes::from(&pixels);
    let texture = gtk::gdk::MemoryTexture::new(
        side as i32,
        side as i32,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        side * 4,
    );
    picture.set_paintable(Some(&texture));
    picture.set_visible(true);
    width as u32
}

/// Ask the daemon for the current state and paint the dialog.
pub(crate) fn refresh(dialog: &RemoteDialog, client: &Arc<dyn Client>) {
    let bus = RequestBus::new();
    bus.spawn(client, "remote.status", json!({}));
    let status = dialog.status.clone();
    let revoke = dialog.revoke.clone();
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
        if let Some((_, Ok(value))) = bus.take().into_iter().next() {
            apply_status(&status, &revoke, &value);
            return gtk::glib::ControlFlow::Break;
        }
        gtk::glib::ControlFlow::Continue
    });
}

fn apply_status(status: &gtk::Label, revoke: &gtk::Button, value: &Value) {
    let enabled = value["enabled"].as_bool().unwrap_or(false);
    let paired = value["paired"].as_bool().unwrap_or(false);
    let port = value["port"].as_u64().unwrap_or(0);
    let text = match (enabled, paired) {
        (true, true) => format!("Control remoto activo en el puerto {port}."),
        (true, false) => "Control remoto activo, sin dispositivo emparejado.".to_string(),
        _ => "Control remoto apagado.".to_string(),
    };
    status.set_label(&text);
    revoke.set_sensitive(paired);
}
