//! Link Grabber dialog: paste a list, see what will be queued, queue it.
//!
//! Contract: S3 of `odd/tasks/segmenta-hito2-captura-hosters-remoto.md` (T8).
//!
//! The dialog owns no rules. It sends the pasted text to the daemon (`grabber.preview`) and
//! renders the answer, so "what will be downloaded" has one answer in the product instead of
//! two that can disagree.

use serde::Deserialize;

use crate::adw;
use crate::adw::prelude::*;
use crate::gtk;
use crate::prefs::RpcCall;

/// One refused line, as the daemon reports it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Refused {
    /// The line as pasted.
    pub line: String,
    /// Why it was refused.
    pub rejection: Rejection,
}

/// Why a line was refused.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rejection {
    /// Not a downloadable link.
    NotALink,
    /// A pattern that asks for too many links.
    PatternTooMany {
        /// How many it asked for.
        wanted: usize,
    },
    /// Already in the list.
    Duplicate,
    /// The plan is full.
    OverLimit,
}

impl Rejection {
    /// Sentence shown next to the line.
    pub fn reason(&self) -> String {
        match self {
            Rejection::NotALink => "not a downloadable link".to_string(),
            Rejection::PatternTooMany { wanted } => {
                format!("the pattern asks for {wanted} links, which is too many")
            }
            Rejection::Duplicate => "already in the list".to_string(),
            Rejection::OverLimit => "the list is already full".to_string(),
        }
    }
}

/// What the daemon would queue for a paste.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
pub struct Plan {
    /// Links to queue, in paste order.
    #[serde(default)]
    pub accepted: Vec<String>,
    /// Refused lines with their reason.
    #[serde(default)]
    pub refused: Vec<Refused>,
}

impl Plan {
    /// One-line summary for the dialog.
    pub fn summary(&self) -> String {
        match (self.accepted.len(), self.refused.len()) {
            (0, 0) => "nothing pasted yet".to_string(),
            (accepted, 0) => format!("{accepted} links ready"),
            (0, refused) => format!("no usable links; {refused} refused"),
            (accepted, refused) => format!("{accepted} links ready, {refused} refused"),
        }
    }
}

/// The dialog's widgets, kept public so a test can drive them.
pub struct GrabberDialog {
    /// Window to present.
    pub window: adw::Window,
    /// Pasted text.
    pub text: gtk::TextView,
    /// Links that will be queued.
    pub list: gtk::ListBox,
    /// Counts and status.
    pub summary: gtk::Label,
    /// Refusals, one per line.
    pub refused: gtk::Label,
    /// Queue button.
    pub queue: gtk::Button,
    /// What the daemon answered.
    plan: std::rc::Rc<std::cell::RefCell<Plan>>,
}

/// Build the dialog. `preview` asks the daemon; `queue` adds one link.
pub fn grabber_dialog(preview: RpcCall, enqueue: RpcCall) -> GrabberDialog {
    let window = adw::Window::builder()
        .title("Link Grabber")
        .default_width(560)
        .default_height(520)
        .build();

    let text = gtk::TextView::builder()
        .monospace(true)
        .wrap_mode(gtk::WrapMode::Char)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .min_content_height(160)
        .child(&text)
        .build();

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    let list_scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&list)
        .build();

    let summary = gtk::Label::builder()
        .xalign(0.0)
        .label("nothing pasted yet")
        .build();
    let refused = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label"])
        .build();
    let queue_button = gtk::Button::with_label("Queue");
    queue_button.add_css_class("suggested-action");

    let boxed = gtk::Box::new(gtk::Orientation::Vertical, 8);
    boxed.set_margin_top(12);
    boxed.set_margin_bottom(12);
    boxed.set_margin_start(12);
    boxed.set_margin_end(12);
    boxed.append(&summary);
    boxed.append(&scroller);
    boxed.append(&list_scroller);
    boxed.append(&refused);
    boxed.append(&queue_button);
    window.set_content(Some(&boxed));

    let plan = std::rc::Rc::new(std::cell::RefCell::new(Plan::default()));
    let shown = std::rc::Rc::clone(&plan);
    let preview_for_change = std::rc::Rc::clone(&preview);
    let summary_for_change = summary.clone();
    let refused_for_change = refused.clone();
    let list_for_change = list.clone();
    let queue_for_change = queue_button.clone();
    text.buffer().connect_changed(move |buffer| {
        let (start, end) = buffer.bounds();
        let pasted = buffer.text(&start, &end, false).to_string();
        if pasted.trim().is_empty() {
            *shown.borrow_mut() = Plan::default();
            summary_for_change.set_label("nothing pasted yet");
            refused_for_change.set_label("");
            queue_for_change.set_sensitive(false);
            clear_rows(&list_for_change);
            return;
        }
        preview_for_change("grabber.preview", serde_json::json!({ "text": pasted }));
    });

    let plan_for_queue = std::rc::Rc::clone(&plan);
    let button_for_queue = queue_button.clone();
    queue_button.connect_clicked(move |_| {
        let links = plan_for_queue.borrow().accepted.clone();
        for link in links {
            enqueue("download.add", serde_json::json!({ "url": link }));
        }
    });

    GrabberDialog {
        window,
        text,
        list,
        summary,
        refused,
        queue: button_for_queue,
        plan,
    }
}

/// Repaint the dialog from a `grabber.preview` answer.
pub fn apply_preview(dialog: &GrabberDialog, answer: &serde_json::Value) {
    let plan: Plan = serde_json::from_value(answer.clone()).unwrap_or_default();
    dialog.summary.set_label(&plan.summary());
    dialog.queue.set_sensitive(!plan.accepted.is_empty());
    let reasons: Vec<String> = plan
        .refused
        .iter()
        .map(|item| format!("{}: {}", item.line, item.rejection.reason()))
        .collect();
    dialog.refused.set_label(&reasons.join("\n"));
    clear_rows(&dialog.list);
    for link in &plan.accepted {
        let row = adw::ActionRow::builder().title(link.as_str()).build();
        dialog.list.append(&row);
    }
    *dialog.plan.borrow_mut() = plan;
}

/// The links the dialog would queue right now.
pub fn planned_links(dialog: &GrabberDialog) -> Vec<String> {
    dialog.plan.borrow().accepted.clone()
}

fn clear_rows(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}
