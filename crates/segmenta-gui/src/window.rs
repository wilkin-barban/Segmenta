//! Main window: header bar, filter sidebar, download list and detail page.
//!
//! The window owns no socket: every daemon call goes through [`Client`] on a
//! worker thread and the results are applied by a `glib` timeout tick, so the
//! GTK main loop never blocks.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use adw::prelude::*;

use crate::client::{Client, ClientError, ClientEvent, RequestBus};
use segmenta_i18n::{tr, Msg};

use crate::model::{
    BatchDeleteReport, CleanCompletedReport, DeleteSelectionSummary, DownloadItem, DownloadStatus,
    Filter, ProgressUpdate, SegmentItem, SeqStatus, Settings,
};
use crate::segment_view::SegmentaSegmentView;
use crate::{add_dialog, adw, gtk, prefs};

/// Application identifier of the desktop entry.
pub const APP_ID: &str = "io.github.wilkin.Segmenta";

/// Interval of the UI tick that drains daemon events (OpenSpec 04).
const TICK: Duration = Duration::from_millis(150);

/// Locale used by every visible string (S21).
fn locale() -> segmenta_i18n::Locale {
    segmenta_i18n::current()
}

/// `download.list` page size. The list is filtered client side so the sidebar
/// counters always describe the same snapshot.
const LIST_LIMIT: u64 = 500;

/// Page size of the counting sweep that learns the real table size when the
/// snapshot comes back full. It is the largest `limit` the daemon accepts
/// (OpenSpec 07 §3.3).
const COUNT_PAGE: u64 = 1000;

/// Bound of the counting sweep: 20 pages cover 20 000 tasks. Past it the notice
/// says the total is a lower bound instead of pretending to know it.
const COUNT_PAGES_MAX: usize = 20;

/// What the content area is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewState {
    /// Waiting for the first answer.
    Loading,
    /// The daemon answered and there is nothing to list.
    Empty,
    /// The daemon could not be reached, or answered with an error.
    Error(String),
    /// At least one row is listed.
    Ready,
}

/// Handle of the main window; cheap to clone.
#[derive(Clone)]
pub struct SegmentaWindow(Rc<Inner>);

struct Inner {
    app: adw::Application,
    client: Arc<dyn Client>,
    window: adw::ApplicationWindow,
    toast_overlay: adw::ToastOverlay,
    nav: adw::NavigationView,
    stack: gtk::Stack,
    list_view: gtk::ListView,
    list_store: gtk::gio::ListStore,
    selection: gtk::SingleSelection,
    filter_list: gtk::ListBox,
    queue_list: gtk::ListBox,
    queue_group: gtk::Box,
    count_labels: RefCell<Vec<(Filter, gtk::Label)>>,
    empty_page: adw::StatusPage,
    error_page: adw::StatusPage,
    search_bar: gtk::SearchBar,
    store: RefCell<crate::model::DownloadStore>,
    filter: Cell<Filter>,
    query: RefCell<String>,
    queue: RefCell<Option<String>>,
    /// Queue names reported by `queue.list`; never derived from the downloads.
    queues: RefCell<Vec<String>>,
    /// Real size of the download table while the snapshot is capped.
    snapshot_total: Cell<Option<usize>>,
    /// Whether `snapshot_total` is exact or only a lower bound.
    snapshot_total_exact: Cell<bool>,
    /// Short notice painted above the list when the snapshot is capped.
    truncation_label: gtk::Label,
    /// Guards the queue sidebar rebuild so a programmatic selection cannot feed
    /// back into `row_selected`.
    rebuilding_queues: Cell<bool>,
    view_state: RefCell<ViewState>,
    loading_page: gtk::Box,
    add_dialog: RefCell<Option<adw::Dialog>>,
    confirm_dialog: RefCell<Option<adw::AlertDialog>>,
    /// The batch confirmation dialog: clean completed or delete the selection.
    ///
    /// It stays open while the request is in flight, so the user never sees the
    /// destructive action close before its result is known.
    batch_dialog: RefCell<Option<BatchDialog>>,
    /// Ids checked in multi-selection mode, kept by task id so a refresh that keeps
    /// the same tasks keeps the selection.
    selected: RefCell<BTreeSet<String>>,
    /// Explicit multi-selection mode (checkbox column), toggled from the menu.
    multi_select: Cell<bool>,
    clean_action: gtk::gio::SimpleAction,
    delete_selection_action: gtk::gio::SimpleAction,
    multi_select_action: gtk::gio::SimpleAction,
    last_clean_report: RefCell<Option<CleanCompletedReport>>,
    last_delete_report: RefCell<Option<BatchDeleteReport>>,
    last_error: RefCell<Option<String>>,
    settings: RefCell<Settings>,
    bus: RequestBus,
    pending: RefCell<Vec<(u64, Pending)>>,
    detail: RefCell<Option<DetailPage>>,
    /// A click is in flight: it focused the list and is selecting a row.
    click_intent: std::cell::Cell<bool>,
    connected: Cell<bool>,
    closed: Cell<bool>,
    daemon_version: RefCell<Option<String>>,
}

/// Requests in flight, matched by [`RequestBus`] id.
#[derive(Debug, Clone)]
enum Pending {
    Hello,
    Reload,
    Settings,
    Status(String),
    Pause(String),
    Resume(String),
    Cancel(String),
    Delete(String),
    /// `download.clean_completed`: only `completed` rows, never a file.
    CleanCompleted,
    /// `download.delete_many`: one result per task of the batch.
    DeleteMany,
    /// `queue.list`: the only source of queue names.
    Queues,
    /// Next page of the counting sweep: rows already known, pages served.
    Total {
        offset: usize,
        pages: usize,
    },
}

/// What the batch confirmation dialog is confirming.
#[derive(Debug, Clone, PartialEq, Eq)]
enum BatchKind {
    /// `download.clean_completed`: no checkbox, never touches a file.
    CleanCompleted { count: usize },
    /// `download.delete_many` over the checked tasks.
    DeleteSelection {
        task_ids: Vec<String>,
        summary: DeleteSelectionSummary,
    },
}

/// Widgets of the batch confirmation dialog.
struct BatchDialog {
    dialog: adw::Dialog,
    body: gtk::Label,
    error: gtk::Label,
    accept: gtk::Button,
    cancel: gtk::Button,
    /// `None` for clean completed: that operation never deletes files.
    delete_files: Option<gtk::CheckButton>,
    checkbox_label: Option<String>,
    kind: BatchKind,
    /// The accept label before the request replaced it with "Deleting…".
    accept_label: String,
    waiting: Cell<bool>,
}

/// Widgets of the detail page of one download.
struct DetailPage {
    task_id: String,
    page: adw::NavigationPage,
    filename: gtk::Label,
    path: gtk::Label,
    state: gtk::Label,
    progress: gtk::ProgressBar,
    percent: gtk::Label,
    speed: gtk::Label,
    eta: gtk::Label,
    size: gtk::Label,
    error: gtk::Label,
    segments: SegmentaSegmentView,
    segment_label: gtk::Label,
    pause: gtk::Button,
    resume: gtk::Button,
    cancel: gtk::Button,
    delete: gtk::Button,
    open_folder: gtk::Button,
}

impl SegmentaWindow {
    /// Builds the window and starts loading from `client`.
    pub fn new(app: &adw::Application, client: Arc<dyn Client>) -> SegmentaWindow {
        let builder = adw::ApplicationWindow::builder()
            .application(app)
            .title("Segmenta")
            .default_width(980)
            .default_height(680);
        let window = builder.build();

        let list_store = gtk::gio::ListStore::new::<gtk::glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::new(Some(list_store.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        let list_view =
            gtk::ListView::new(Some(selection.clone()), None::<gtk::SignalListItemFactory>);

        // No spinner widget at all: the X11 captures kept showing a spinning glyph over a
        // fully loaded list, and a widget that does not exist cannot paint. The state is
        // carried by the label, which is honest and testable.
        let loading = gtk::Box::new(gtk::Orientation::Vertical, 12);
        loading.set_valign(gtk::Align::Center);
        loading.set_halign(gtk::Align::Center);
        let loading_label = gtk::Label::new(Some(tr(locale(), Msg::Loading)));
        loading_label.add_css_class("dim-label");
        loading.append(&loading_label);

        // Nothing in this app sets a busy cursor; an inherited one from the session would
        // paint a ring at the pointer, which is what the X11 capture showed.
        window.set_cursor_from_name(Some("default"));

        let truncation_label = gtk::Label::new(None);
        truncation_label.set_xalign(0.0);
        truncation_label.set_wrap(true);
        truncation_label.set_margin_top(6);
        truncation_label.set_margin_bottom(6);
        truncation_label.set_margin_start(12);
        truncation_label.set_margin_end(12);
        truncation_label.add_css_class("dim-label");
        truncation_label.set_visible(false);

        // Destructive actions start disabled: there is no completed task and no
        // selection until the daemon answers.
        let clean_action = gtk::gio::SimpleAction::new("clean-completed", None);
        clean_action.set_enabled(false);
        let delete_selection_action = gtk::gio::SimpleAction::new("delete-selected", None);
        delete_selection_action.set_enabled(false);
        let multi_select_action = gtk::gio::SimpleAction::new_stateful(
            "multi-select",
            None,
            &gtk::glib::Variant::from(false),
        );

        let inner = Rc::new(Inner {
            app: app.clone(),
            client,
            window: window.clone(),
            toast_overlay: adw::ToastOverlay::new(),
            nav: adw::NavigationView::new(),
            stack: gtk::Stack::new(),
            list_view,
            list_store,
            selection,
            filter_list: gtk::ListBox::new(),
            queue_list: gtk::ListBox::new(),
            queue_group: gtk::Box::new(gtk::Orientation::Vertical, 6),
            count_labels: RefCell::new(Vec::new()),
            empty_page: adw::StatusPage::new(),
            error_page: adw::StatusPage::new(),
            search_bar: gtk::SearchBar::new(),
            store: RefCell::new(crate::model::DownloadStore::default()),
            filter: Cell::new(Filter::All),
            query: RefCell::new(String::new()),
            queue: RefCell::new(None),
            queues: RefCell::new(Vec::new()),
            snapshot_total: Cell::new(None),
            snapshot_total_exact: Cell::new(false),
            truncation_label,
            rebuilding_queues: Cell::new(false),
            view_state: RefCell::new(ViewState::Loading),
            loading_page: loading.clone(),
            add_dialog: RefCell::new(None),
            confirm_dialog: RefCell::new(None),
            batch_dialog: RefCell::new(None),
            selected: RefCell::new(BTreeSet::new()),
            multi_select: Cell::new(false),
            clean_action,
            delete_selection_action,
            multi_select_action,
            last_clean_report: RefCell::new(None),
            last_delete_report: RefCell::new(None),
            last_error: RefCell::new(None),
            settings: RefCell::new(Settings::default()),
            bus: RequestBus::new(),
            pending: RefCell::new(Vec::new()),
            detail: RefCell::new(None),
            click_intent: std::cell::Cell::new(false),
            connected: Cell::new(false),
            closed: Cell::new(false),
            daemon_version: RefCell::new(None),
        });

        inner.build_layout();
        inner.install_actions();
        inner.install_shortcuts();
        inner.start_tick();
        window.present();

        let handle = SegmentaWindow(Rc::clone(&inner));
        inner.initial_load();
        handle
    }

    /// Underlying GTK window.
    pub fn window(&self) -> adw::ApplicationWindow {
        self.0.window.clone()
    }

    /// Rows currently painted by the list, in display order.
    pub fn rows(&self) -> Vec<DownloadItem> {
        list_items(&self.0.list_store)
    }

    /// Sidebar counters computed from the loaded snapshot.
    pub fn counts(&self) -> crate::model::Counts {
        self.0.store.borrow().counts()
    }

    /// Queue names painted in the sidebar, in the daemon's `queue.list` order.
    pub fn queue_names(&self) -> Vec<String> {
        self.0.queue_names()
    }

    /// Whether the queue sidebar is painted at all.
    pub fn queues_visible(&self) -> bool {
        self.0.queue_group.is_visible()
    }

    /// Notice painted when the download snapshot is capped, with the real total.
    pub fn truncation_notice(&self) -> Option<String> {
        self.0.truncation_notice()
    }

    /// State of the content area.
    pub fn view_state(&self) -> ViewState {
        self.0.view_state.borrow().clone()
    }

    /// Drop the connection, reconnect and reload the list (the error page action).
    pub fn retry(&self) {
        self.0.retry_connection();
    }

    /// Text currently shown on the error page (startup tests).
    pub fn error_description(&self) -> Option<String> {
        self.0.error_page.description().map(|text| text.to_string())
    }

    pub fn filter(&self) -> Filter {
        self.0.filter.get()
    }

    pub fn set_filter(&self, filter: Filter) {
        self.0.set_filter(filter, true);
    }

    /// Selects the row of `task_id`, opening its detail page.
    /// Select a row the way a click does: the list takes focus, then the row is
    /// selected, and only then does the detail open. Pointer hover never focuses.
    /// Bring Segmenta forward with one task focused: no filter, no search, detail open.
    ///
    /// This is the entry point the extension, the CLI and `--show-task` share, so a
    /// download added from anywhere is visible instead of hidden behind a filter.
    pub fn focus_task(&self, task_id: &str) -> bool {
        self.set_filter(Filter::All);
        self.set_query("");
        if let Some(entry) = self.0.search_bar.child().and_downcast::<gtk::SearchEntry>() {
            entry.set_text("");
        }
        let focused = self.click_task(task_id);
        self.window().present();
        focused
    }

    pub fn click_task(&self, task_id: &str) -> bool {
        self.0.list_view.grab_focus();
        self.0.click_intent.set(true);
        let selected = self.select_task(task_id);
        self.0.click_intent.set(false);
        selected
    }

    pub fn select_task(&self, task_id: &str) -> bool {
        let Some(index) = self.0.index_of(task_id) else {
            return false;
        };
        self.0.selection.set_selected(index);
        true
    }

    /// Row behind the open detail page.
    pub fn detail(&self) -> Option<DownloadItem> {
        let task_id = self
            .0
            .detail
            .borrow()
            .as_ref()
            .map(|detail| detail.task_id.clone())?;
        self.0.store.borrow().get(&task_id).cloned()
    }

    /// Number of blocks the detail segment bar is painting.
    pub fn detail_segment_count(&self) -> Option<usize> {
        self.0
            .detail
            .borrow()
            .as_ref()
            .map(|detail| detail.segments.segments())
    }

    /// Asks the daemon for a fresh list.
    pub fn reload(&self) {
        self.0.reload_all();
    }

    /// Opens the "add download" dialog.
    pub fn open_add_dialog(&self) {
        let settings = self.0.settings.borrow().clone();
        let defaults = add_dialog::AddDefaults {
            destination_dir: settings.download_dir.clone(),
            connections: settings.max_connections.clamp(1, 32),
            clip_monitor_enabled: settings.clip_monitor_enabled,
        };
        let window = self.0.window.clone();
        let client = Arc::clone(&self.0.client);
        let handle = self.clone();
        let dialog = add_dialog::present(
            &window,
            client,
            defaults,
            Rc::new(move |filename: String| {
                handle
                    .0
                    .toast(&tr(locale(), Msg::ToastAdded).replace("{filename}", &filename));
                handle.0.reload_all();
            }),
        );
        *self.0.add_dialog.borrow_mut() = Some(dialog);
    }

    /// Whether the "add download" dialog is open (used by the action tests).
    pub fn add_dialog_open(&self) -> bool {
        self.0
            .add_dialog
            .borrow()
            .as_ref()
            .map(|dialog| dialog.is_visible())
            .unwrap_or(false)
    }

    /// Close the "add download" dialog without sending anything.
    pub fn close_add_dialog(&self) {
        if let Some(dialog) = self.0.add_dialog.borrow_mut().take() {
            dialog.close();
        }
    }

    /// Set the search text the way the search entry does.
    pub fn set_query(&self, query: &str) {
        *self.0.query.borrow_mut() = query.to_string();
        self.0.refill();
    }

    /// Toggle the search bar (the `win.search` action body).
    pub fn toggle_search(&self) {
        let visible = !self.0.search_bar.is_search_mode();
        self.0.search_bar.set_search_mode(visible);
    }

    /// Whether the loading page is still part of the stack (it is detached once the
    /// first Ready state is painted).
    pub fn loading_page_attached(&self) -> bool {
        self.0.stack.child_by_name("loading").is_some()
    }

    /// Whether the search bar is open.
    pub fn search_active(&self) -> bool {
        self.0.search_bar.is_search_mode()
    }

    /// Detail actions currently offered for the selected task.
    /// Label of the state pill for the open detail, with quarantine as its own state.
    pub fn detail_state(&self) -> Option<String> {
        let item = self.detail()?;
        Some(if item.is_quarantined() {
            tr(locale(), Msg::StatusQuarantined).to_string()
        } else {
            item.status.label(locale()).to_string()
        })
    }

    /// Path shown for the open detail: the quarantine copy when the daemon moved it.
    pub fn detail_path(&self) -> Option<String> {
        self.detail().map(|item| item.display_path().to_string())
    }

    /// Quarantine notice for the open detail, or `None` when nothing was quarantined.
    pub fn detail_quarantine(&self) -> Option<String> {
        let item = self.detail()?;
        if !item.is_quarantined() {
            return None;
        }
        Some(tr(locale(), Msg::QuarantineFile).replacen("{}", item.display_path(), 1))
    }

    pub fn detail_actions(&self) -> Vec<String> {
        let Some(item) = self.detail() else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        if item.status.can_pause() {
            actions.push("pause".to_string());
        }
        if item.status.can_resume() {
            actions.push("resume".to_string());
        }
        if item.status.can_cancel() {
            actions.push("cancel".to_string());
        }
        if item.status.can_delete() {
            actions.push("delete".to_string());
        }
        // A quarantined file is no longer in the downloads folder: offering to open it
        // there would point at something the daemon already moved away.
        if !item.final_path.is_empty() && !item.is_quarantined() {
            actions.push("open_folder".to_string());
        }
        actions
    }

    /// Ask for confirmation to delete the selected task (does not mutate anything).
    pub fn begin_delete(&self) {
        self.0.begin_delete_selected();
    }

    /// Whether "Limpiar completadas" is offered right now.
    ///
    /// It mirrors the real action sensitivity, so a disabled menu entry can never be
    /// reported as available.
    pub fn clean_completed_enabled(&self) -> bool {
        self.0.clean_action.is_enabled()
    }

    /// Ask for confirmation to clean the completed tasks (never deletes a file).
    pub fn begin_clean_completed(&self) {
        self.0.begin_clean_completed();
    }

    /// Whether the batch confirmation dialog is open (clean or delete).
    pub fn confirm_dialog_open(&self) -> bool {
        self.0
            .batch_dialog
            .borrow()
            .as_ref()
            .map(|dialog| dialog.dialog.is_visible())
            .unwrap_or(false)
    }

    /// Body text of the open batch confirmation, for accessibility and tests.
    pub fn confirm_body(&self) -> Option<String> {
        self.0
            .batch_dialog
            .borrow()
            .as_ref()
            .map(|dialog| dialog.body.label().to_string())
    }

    /// Counts shown by the open delete confirmation.
    pub fn confirm_summary(&self) -> Option<DeleteSelectionSummary> {
        let dialog = self.0.batch_dialog.borrow();
        match dialog.as_ref().map(|dialog| &dialog.kind) {
            Some(BatchKind::DeleteSelection { summary, .. }) => Some(*summary),
            _ => None,
        }
    }

    /// State of the "delete the files too" checkbox, or `None` when the operation
    /// has no such checkbox (cleaning completed tasks never deletes files).
    pub fn confirm_delete_files(&self) -> Option<bool> {
        let dialog = self.0.batch_dialog.borrow();
        dialog
            .as_ref()
            .and_then(|dialog| dialog.delete_files.as_ref())
            .map(|check| check.is_active())
    }

    /// Checks or unchecks the "delete the files too" checkbox.
    pub fn set_confirm_delete_files(&self, active: bool) {
        let dialog = self.0.batch_dialog.borrow();
        if let Some(check) = dialog
            .as_ref()
            .and_then(|dialog| dialog.delete_files.as_ref())
        {
            check.set_active(active);
        }
    }

    /// Label of the "delete the files too" checkbox.
    pub fn confirm_checkbox_label(&self) -> Option<String> {
        self.0
            .batch_dialog
            .borrow()
            .as_ref()
            .and_then(|dialog| dialog.checkbox_label.clone())
    }

    /// Answers the open batch confirmation with its accept action.
    ///
    /// Returns `false` when no dialog is open or a request is already in flight.
    pub fn accept_confirmation(&self) -> bool {
        self.0.accept_batch_dialog()
    }

    /// What the keyboard default does: the safe answer, cancelling the dialog.
    pub fn press_confirm_default(&self) {
        self.0.dismiss_batch_dialog();
    }

    /// Whether multi-selection mode is on.
    pub fn multi_select_enabled(&self) -> bool {
        self.0.multi_select.get()
    }

    /// Turns the explicit multi-selection mode on or off.
    pub fn set_multi_select(&self, enabled: bool) {
        self.0.set_multi_select(enabled);
    }

    /// Checks or unchecks one task. Returns `false` for a task that is not listed.
    pub fn set_task_selected(&self, task_id: &str, selected: bool) -> bool {
        self.0.set_task_selected(task_id, selected)
    }

    /// Checked task ids in list order.
    pub fn selected_task_ids(&self) -> Vec<String> {
        self.0.selected_task_ids()
    }

    /// Number of files the selection would touch.
    pub fn selection_file_count(&self) -> usize {
        self.0
            .store
            .borrow()
            .selection_summary(&self.0.selected_task_ids())
            .files
    }

    /// Ask for confirmation to delete every checked task.
    pub fn begin_delete_selection(&self) {
        self.0.begin_delete_selection();
    }

    /// Last answer of `download.clean_completed`.
    pub fn last_clean_report(&self) -> Option<CleanCompletedReport> {
        self.0.last_clean_report.borrow().clone()
    }

    /// Last answer of `download.delete_many`.
    pub fn last_delete_report(&self) -> Option<BatchDeleteReport> {
        self.0.last_delete_report.borrow().clone()
    }

    /// Whether a confirmation dialog is waiting for an answer.
    pub fn confirmation_pending(&self) -> bool {
        self.0
            .confirm_dialog
            .borrow()
            .as_ref()
            .map(|dialog| dialog.is_visible())
            .unwrap_or(false)
    }

    /// Dismiss the pending confirmation without accepting it.
    pub fn dismiss_confirmation(&self) {
        if let Some(dialog) = self.0.confirm_dialog.borrow_mut().take() {
            dialog.force_close();
        }
    }

    /// Actions the window exposes (menu and toolbar wiring).
    pub fn action_names(&self) -> Vec<String> {
        self.0
            .window
            .list_actions()
            .into_iter()
            .map(|name| name.to_string())
            .collect()
    }

    /// Opens the preferences dialog.
    pub fn open_preferences(&self) {
        let settings = self.0.settings.borrow().clone();
        let handle = self.clone();
        prefs::present(
            &self.0.window,
            Arc::clone(&self.0.client),
            settings,
            Rc::new(move |updated: Settings| {
                *handle.0.settings.borrow_mut() = updated;
                handle.0.apply_settings_to_ui();
            }),
        );
    }

    /// Opens the "about" dialog.
    pub fn show_about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("Segmenta")
            .application_icon(APP_ID)
            .developer_name("Wilkin Barban Rosabal")
            .developers(["Wilkin Barban Rosabal"])
            .version(env!("CARGO_PKG_VERSION"))
            .website("https://github.com/wilkin-barban/Segmenta")
            .issue_url("https://github.com/wilkin-barban/Segmenta/issues")
            .license_type(gtk::License::Gpl30)
            .comments("Gestor de descargas segmentadas para GNOME")
            .build();
        about.present(Some(&self.0.window));
    }
}

impl Inner {
    fn build_layout(self: &Rc<Self>) {
        let header = adw::HeaderBar::new();

        let add_button = gtk::Button::with_label(tr(locale(), Msg::AddUrl));
        add_button.add_css_class("suggested-action");
        add_button.set_tooltip_text(Some(tr(locale(), Msg::MenuAddTooltip)));
        header.pack_start(&add_button);
        let add_handle = Rc::downgrade(self);
        add_button.connect_clicked(move |_| {
            if let Some(inner) = add_handle.upgrade() {
                SegmentaWindow(inner).open_add_dialog();
            }
        });

        let search_toggle = gtk::ToggleButton::new();
        search_toggle.set_icon_name("system-search-symbolic");
        search_toggle.set_tooltip_text(Some("Buscar (Ctrl+F)"));
        header.pack_end(&search_toggle);

        let preferences_button = gtk::Button::new();
        preferences_button.set_icon_name("emblem-system-symbolic");
        preferences_button.set_tooltip_text(Some("Preferencias (Ctrl+,)"));
        header.pack_end(&preferences_button);
        let preferences_handle = Rc::downgrade(self);
        preferences_button.connect_clicked(move |_| {
            if let Some(inner) = preferences_handle.upgrade() {
                SegmentaWindow(inner).open_preferences();
            }
        });

        let menu = gtk::gio::Menu::new();
        menu.append(Some(tr(locale(), Msg::AddUrl)), Some("win.add"));
        menu.append(Some(tr(locale(), Msg::Reload)), Some("win.reload"));
        menu.append(Some(tr(locale(), Msg::Search)), Some("win.search"));
        menu.append(
            Some(tr(locale(), Msg::CleanCompleted)),
            Some("win.clean-completed"),
        );
        menu.append(
            Some(tr(locale(), Msg::MultiSelect)),
            Some("win.multi-select"),
        );
        menu.append(
            Some(tr(locale(), Msg::DeleteSelection)),
            Some("win.delete-selected"),
        );
        menu.append(
            Some(tr(locale(), Msg::Preferences)),
            Some("win.preferences"),
        );
        menu.append(Some(tr(locale(), Msg::About)), Some("win.about"));
        menu.append(Some(tr(locale(), Msg::Quit)), Some("app.quit"));
        let menu_button = gtk::MenuButton::new();
        menu_button.set_icon_name("open-menu-symbolic");
        menu_button.set_menu_model(Some(&menu));
        menu_button.set_tooltip_text(Some(tr(locale(), Msg::MenuMain)));
        header.pack_end(&menu_button);

        // Search bar.
        let search_entry = gtk::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Buscar por nombre o URL"));
        self.search_bar.set_child(Some(&search_entry));
        self.search_bar.connect_entry(&search_entry);
        self.search_bar.set_visible(false);
        search_toggle
            .bind_property("active", &self.search_bar, "search-mode-enabled")
            .bidirectional()
            .sync_create()
            .build();

        let search_handle = Rc::downgrade(self);
        search_entry.connect_search_changed(move |entry| {
            if let Some(inner) = search_handle.upgrade() {
                *inner.query.borrow_mut() = entry.text().to_string();
                inner.refill();
            }
        });

        // Content stack: loading, list, empty and error are distinct pages so
        // "daemon unreachable" never looks like "no downloads".
        self.stack
            .set_transition_type(gtk::StackTransitionType::Crossfade);
        self.stack.add_named(&self.loading_page, Some("loading"));
        self.stack.add_named(&self.build_list_page(), Some("list"));
        self.stack
            .add_named(&self.build_empty_page(), Some("empty"));
        self.stack
            .add_named(&self.build_error_page(), Some("error"));
        self.stack.set_visible_child_name("loading");

        let sidebar = self.build_sidebar();
        let sidebar_page = adw::NavigationPage::builder()
            .title(tr(locale(), Msg::WindowTitle))
            .child(&sidebar)
            .build();

        let content_toolbar = adw::ToolbarView::new();
        content_toolbar.add_top_bar(&header);
        content_toolbar.add_bottom_bar(&self.search_bar);
        content_toolbar.set_content(Some(&self.stack));

        let content_page = adw::NavigationPage::builder()
            .title(tr(locale(), Msg::WindowTitle))
            .child(&content_toolbar)
            .build();

        let split = adw::NavigationSplitView::new();
        split.set_sidebar(Some(&sidebar_page));
        split.set_content(Some(&content_page));
        split.set_min_sidebar_width(210.0);
        split.set_sidebar_width_fraction(0.24);
        split.set_max_sidebar_width(320.0);

        // The navigation view owns the split view so a selected row can push a
        // full detail page with its own header bar and back button.
        let root_page = adw::NavigationPage::builder()
            .title(tr(locale(), Msg::WindowTitle))
            .child(&split)
            .build();
        root_page.set_tag(Some("downloads"));
        self.nav.add(&root_page);
        self.nav.set_vexpand(true);
        self.toast_overlay.set_child(Some(&self.nav));
        self.window.set_content(Some(&self.toast_overlay));

        let close_handle = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(inner) = close_handle.upgrade() {
                inner.closed.set(true);
            }
            gtk::glib::Propagation::Proceed
        });
    }

    fn build_sidebar(self: &Rc<Self>) -> gtk::Widget {
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scrolled.set_vexpand(true);

        let container = gtk::Box::new(gtk::Orientation::Vertical, 12);
        container.set_margin_top(12);
        container.set_margin_bottom(12);
        container.set_margin_start(12);
        container.set_margin_end(12);

        self.filter_list
            .set_selection_mode(gtk::SelectionMode::Single);
        self.filter_list.add_css_class("navigation-sidebar");
        let mut labels = Vec::new();
        for (index, filter) in Filter::ALL.into_iter().enumerate() {
            let row = gtk::ListBoxRow::new();
            row.set_activatable(true);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let icon = gtk::Image::from_icon_name(filter.icon_name());
            let label = gtk::Label::new(Some(filter.label(locale())));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            let count = gtk::Label::new(Some("0"));
            count.add_css_class("dim-label");
            count.add_css_class("numeric");
            line.append(&icon);
            line.append(&label);
            line.append(&count);
            row.set_child(Some(&line));
            self.filter_list.append(&row);
            labels.push((filter, count.clone()));
            let _ = index;
        }
        *self.count_labels.borrow_mut() = labels;

        let filter_handle = Rc::downgrade(self);
        self.filter_list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let Some(inner) = filter_handle.upgrade() else {
                return;
            };
            let index = row.index().max(0) as usize;
            if let Some(filter) = Filter::ALL.get(index).copied() {
                inner.set_filter(filter, false);
            }
        });
        self.filter_list
            .select_row(self.filter_list.row_at_index(0).as_ref());

        container.append(&self.filter_list);

        let queue_title = gtk::Label::new(Some(tr(locale(), Msg::Queues)));
        queue_title.set_xalign(0.0);
        queue_title.add_css_class("dim-label");
        queue_title.add_css_class("caption-heading");
        self.queue_list.add_css_class("navigation-sidebar");
        self.queue_list
            .set_selection_mode(gtk::SelectionMode::Single);
        self.queue_group.append(&queue_title);
        self.queue_group.append(&self.queue_list);
        container.append(&self.queue_group);

        let queue_handle = Rc::downgrade(self);
        self.queue_list.connect_row_selected(move |_, row| {
            let Some(inner) = queue_handle.upgrade() else {
                return;
            };
            if inner.rebuilding_queues.get() {
                return;
            }
            let name = row.and_then(queue_row_name);
            *inner.queue.borrow_mut() = name;
            inner.refill();
        });

        scrolled.set_child(Some(&container));
        scrolled.upcast()
    }

    fn build_list_page(self: &Rc<Self>) -> gtk::Widget {
        let factory = gtk::SignalListItemFactory::new();
        let bind_handle = Rc::downgrade(self);
        factory.connect_setup(|_, object| {
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 6);
            list_item.set_child(Some(&placeholder));
            list_item.set_activatable(true);
        });
        factory.connect_bind(move |_, object| {
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(item) = list_item
                .item()
                .and_downcast::<gtk::glib::BoxedAnyObject>()
                .and_then(|boxed| {
                    boxed
                        .try_borrow::<DownloadItem>()
                        .ok()
                        .map(|item| item.clone())
                })
            else {
                return;
            };
            let Some(child) = list_item.child() else {
                return;
            };
            if let Some(inner) = bind_handle.upgrade() {
                inner.fill_row(&child, &item);
            }
        });
        factory.connect_unbind(|_, object| {
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            if let Some(child) = list_item.child().and_downcast::<gtk::Box>() {
                while let Some(orphan) = child.first_child() {
                    child.remove(&orphan);
                }
            }
        });

        self.list_view.set_factory(Some(&factory));
        self.list_view.add_css_class("boxed-list");
        // A single click must not imply "activate on pointer contact": with this
        // enabled GTK activated the row as the pointer entered it, which pushed the
        // detail page on hover (0.1.1 bug). Activation is explicit below.
        self.list_view.set_single_click_activate(false);
        let activate_handle = Rc::downgrade(self);
        self.list_view.connect_activate(move |view, position| {
            let Some(inner) = activate_handle.upgrade() else {
                return;
            };
            let task_id = view
                .model()
                .and_then(|model| model.item(position))
                .and_downcast::<gtk::glib::BoxedAnyObject>()
                .and_then(|boxed| {
                    boxed
                        .try_borrow::<DownloadItem>()
                        .ok()
                        .map(|item| item.task_id.clone())
                });
            if let Some(task_id) = task_id {
                inner.open_detail(&task_id);
            }
        });

        let selection_handle = Rc::downgrade(self);
        self.selection.connect_selected_notify(move |selection| {
            let Some(inner) = selection_handle.upgrade() else {
                return;
            };
            let task_id = selection
                .selected_item()
                .and_downcast::<gtk::glib::BoxedAnyObject>()
                .and_then(|boxed| {
                    boxed
                        .try_borrow::<DownloadItem>()
                        .ok()
                        .map(|item| item.task_id.clone())
                });
            // Hover used to select a row, and every selection opened the detail.
            // Selection without keyboard/mouse focus is never a user intent, so it
            // only paints the row; `close_detail` is the sole teardown path.
            if !inner.click_intent.get() && !inner.list_view.has_focus() {
                return;
            }
            // Deselection happens whenever a row is replaced in place, so it
            // must not tear the detail page down; `close_detail` clears it.
            if let Some(task_id) = task_id {
                inner.open_detail(&task_id);
            }
        });

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&self.list_view));

        // The truncation notice sits above the list so a capped snapshot never
        // looks like the whole table.
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.append(&self.truncation_label);
        page.append(&scrolled);
        page.upcast()
    }

    fn build_empty_page(self: &Rc<Self>) -> gtk::Widget {
        self.empty_page
            .set_icon_name(Some("folder-download-symbolic"));
        self.empty_page.set_title(tr(locale(), Msg::EmptyTitle));
        self.empty_page.set_description(Some(
            "Añade una URL con el botón «Añadir URL» para empezar.\n\
             El motor de descargas corre en el daemon; si no está activo, \
             inícialo con «systemctl --user start segmenta.socket».",
        ));
        let add_button = gtk::Button::with_label(tr(locale(), Msg::AddUrl));
        add_button.add_css_class("suggested-action");
        add_button.set_halign(gtk::Align::Center);
        let handle = Rc::downgrade(self);
        add_button.connect_clicked(move |_| {
            if let Some(inner) = handle.upgrade() {
                SegmentaWindow(inner).open_add_dialog();
            }
        });
        self.empty_page.set_child(Some(&add_button));
        self.empty_page.clone().upcast()
    }

    fn build_error_page(self: &Rc<Self>) -> gtk::Widget {
        self.error_page
            .set_icon_name(Some("dialog-warning-symbolic"));
        self.error_page.set_title(tr(locale(), Msg::ErrorTitle));
        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content_box.set_halign(gtk::Align::Center);
        let detail = gtk::Label::new(Some(""));
        detail.set_wrap(true);
        detail.set_selectable(true);
        detail.set_justify(gtk::Justification::Center);
        detail.set_max_width_chars(60);
        detail.add_css_class("dim-label");
        self.error_page.set_description(Some(
            "El motor de descargas no responde. Comprueba que el servicio de \
             usuario está activo con «systemctl --user start segmenta.socket».",
        ));
        let retry = gtk::Button::with_label(tr(locale(), Msg::Retry));
        retry.add_css_class("suggested-action");
        retry.set_halign(gtk::Align::Center);
        let handle = Rc::downgrade(self);
        retry.connect_clicked(move |_| {
            if let Some(inner) = handle.upgrade() {
                // Drop the stale connection so the worker connects again before the
                // reload; otherwise the click only repaints the same error.
                inner.retry_connection();
            }
        });
        content_box.append(&detail);
        content_box.append(&retry);
        self.error_page.set_child(Some(&content_box));
        // The page stays hidden until `set_error` switches to it; hiding it here made
        // the stack keep showing the loading spinner forever.
        self.error_page.set_visible(true);
        // Keep a handle on the detail label through the page child chain.
        self.error_page.set_widget_name("segmenta-error-page");
        self.error_page.clone().upcast()
    }

    fn install_actions(self: &Rc<Self>) {
        let add = gtk::gio::SimpleAction::new("add", None);
        let handle = Rc::downgrade(self);
        add.connect_activate(move |_, _| {
            if let Some(inner) = handle.upgrade() {
                SegmentaWindow(inner).open_add_dialog();
            }
        });
        self.window.add_action(&add);

        let preferences = gtk::gio::SimpleAction::new("preferences", None);
        let handle = Rc::downgrade(self);
        preferences.connect_activate(move |_, _| {
            if let Some(inner) = handle.upgrade() {
                SegmentaWindow(inner).open_preferences();
            }
        });
        self.window.add_action(&preferences);

        let about = gtk::gio::SimpleAction::new("about", None);
        let handle = Rc::downgrade(self);
        about.connect_activate(move |_, _| {
            if let Some(inner) = handle.upgrade() {
                SegmentaWindow(inner).show_about();
            }
        });
        self.window.add_action(&about);

        let reload = gtk::gio::SimpleAction::new("reload", None);
        let handle = Rc::downgrade(self);
        reload.connect_activate(move |_, _| {
            if let Some(inner) = handle.upgrade() {
                inner.set_loading();
                inner.reload_all();
                inner.load_settings();
            }
        });
        self.window.add_action(&reload);

        let search = gtk::gio::SimpleAction::new("search", None);
        let handle = Rc::downgrade(self);
        let search_bar = self.search_bar.clone();
        search.connect_activate(move |_, _| {
            let _ = handle;
            let visible = !search_bar.is_search_mode();
            search_bar.set_search_mode(visible);
        });
        self.window.add_action(&search);

        let quit = gtk::gio::SimpleAction::new("quit", None);
        let app = self.app.clone();
        quit.connect_activate(move |_, _| {
            app.quit();
        });
        self.app.add_action(&quit);

        let clean = self.clean_action.clone();
        let handle = Rc::downgrade(self);
        clean.connect_activate(move |_, _| {
            if let Some(inner) = handle.upgrade() {
                SegmentaWindow(inner).begin_clean_completed();
            }
        });
        self.window.add_action(&self.clean_action);

        let delete_selection = self.delete_selection_action.clone();
        let handle = Rc::downgrade(self);
        delete_selection.connect_activate(move |_, _| {
            if let Some(inner) = handle.upgrade() {
                SegmentaWindow(inner).begin_delete_selection();
            }
        });
        self.window.add_action(&self.delete_selection_action);

        let multi_select = self.multi_select_action.clone();
        let handle = Rc::downgrade(self);
        multi_select.connect_activate(move |action, _| {
            let current = action
                .state()
                .and_then(|value| value.get::<bool>())
                .unwrap_or(false);
            if let Some(inner) = handle.upgrade() {
                inner.set_multi_select(!current);
            }
        });
        self.window.add_action(&self.multi_select_action);
    }

    fn install_shortcuts(self: &Rc<Self>) {
        self.app.set_accels_for_action("win.add", &["<Control>n"]);
        self.app
            .set_accels_for_action("win.search", &["<Control>f"]);
        self.app
            .set_accels_for_action("win.preferences", &["<Control>comma"]);
        self.app
            .set_accels_for_action("win.delete-selected", &["<Control>Delete"]);
        self.app.set_accels_for_action("app.quit", &["<Control>q"]);
    }

    fn start_tick(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        gtk::glib::timeout_add_local(TICK, move || {
            let Some(inner) = weak.upgrade() else {
                return gtk::glib::ControlFlow::Break;
            };
            inner.tick();
            if inner.closed.get() {
                gtk::glib::ControlFlow::Break
            } else {
                gtk::glib::ControlFlow::Continue
            }
        });
    }

    fn initial_load(self: &Rc<Self>) {
        self.set_loading();
        self.request("system.hello", json!({}), Pending::Hello);
        self.load_settings();
        self.reload_all();
    }

    fn request(&self, method: &str, params: Value, pending: Pending) -> u64 {
        let id = self.bus.spawn(&self.client, method, params);
        self.pending.borrow_mut().push((id, pending));
        id
    }

    fn load_settings(&self) {
        self.request("settings.get", json!({}), Pending::Settings);
    }

    fn reload_all(&self) {
        self.request(
            "download.list",
            json!({ "filter": "all", "limit": LIST_LIMIT, "offset": 0 }),
            Pending::Reload,
        );
        // `queue.list` is the only source of queue names: deriving them from the
        // downloads hides empty queues and invents queues the daemon dropped.
        self.request("queue.list", json!({}), Pending::Queues);
    }

    /// Asks for one more page of `download.list` to learn the real size of the
    /// table when the snapshot page came back full.
    fn count_snapshot(&self, offset: usize, pages: usize) {
        self.request(
            "download.list",
            json!({ "filter": "all", "limit": COUNT_PAGE, "offset": offset }),
            Pending::Total { offset, pages },
        );
    }

    fn set_snapshot_total(&self, total: Option<usize>, exact: bool) {
        self.snapshot_total.set(total);
        self.snapshot_total_exact.set(exact);
    }

    /// What the view says about a capped snapshot, with the real table size.
    fn truncation_notice(&self) -> Option<String> {
        let shown = self.store.borrow().len();
        if (shown as u64) < LIST_LIMIT {
            return None;
        }
        // The i18n catalog has no entry for this yet and is out of this unit's
        // surface, so the text is literal and marked as pending localization.
        match self.snapshot_total.get() {
            Some(total) if total <= shown => None,
            Some(total) if self.snapshot_total_exact.get() => Some(format!(
                "Mostrando las primeras {shown} de {total} descargas."
            )),
            Some(total) => Some(format!(
                "Mostrando las primeras {shown} de más de {total} descargas."
            )),
            None => Some(format!(
                "Mostrando las primeras {shown} descargas; hay más."
            )),
        }
    }

    fn refresh_truncation(&self) {
        let notice = self.truncation_notice();
        self.truncation_label
            .set_label(notice.as_deref().unwrap_or_default());
        self.truncation_label.set_visible(notice.is_some());
    }

    /// The queue filter, dropped when the daemon no longer reports that queue.
    fn queue_filter(&self) -> Option<String> {
        let name = self.queue.borrow().clone()?;
        if self.queues.borrow().contains(&name) {
            Some(name)
        } else {
            *self.queue.borrow_mut() = None;
            None
        }
    }

    fn refresh_task(&self, task_id: &str) {
        self.request(
            "download.status",
            json!({ "task_id": task_id }),
            Pending::Status(task_id.to_string()),
        );
    }

    fn tick(&self) {
        while let Some(event) = self.client.poll_event() {
            match event {
                ClientEvent::Connected => {
                    self.connected.set(true);
                    // The worker just connected (possibly after a retry): if the snapshot
                    // is empty or in error, fetch it now instead of staying on the
                    // spinner. This is what a late socket activation needs.
                    let empty = self.store.borrow().is_empty();
                    let failed = matches!(*self.view_state.borrow(), ViewState::Error(_));
                    if empty || failed {
                        self.set_loading();
                        self.reload_all();
                        self.load_settings();
                    }
                }
                ClientEvent::Disconnected(detail) => {
                    self.connected.set(false);
                    self.set_error(format!("No se pudo conectar con el daemon: {detail}"));
                }
                ClientEvent::Notification { method, params } => {
                    self.handle_notification(&method, params);
                }
            }
        }

        let finished = self.bus.take();
        if crate::client::trace_enabled() {
            eprintln!(
                "segmenta-gui: tick finished={} pending={}",
                finished.len(),
                self.pending.borrow().len()
            );
        }
        if finished.is_empty() {
            return;
        }
        let mut pending = std::mem::take(&mut *self.pending.borrow_mut());
        for (id, result) in finished {
            let Some(position) = pending.iter().position(|(entry, _)| *entry == id) else {
                continue;
            };
            let (_, kind) = pending.remove(position);
            self.finish_request(kind, result);
        }
        // `finish_request` may enqueue follow-ups (reload after pause/delete); keep them.
        self.pending.borrow_mut().splice(0..0, pending);
    }

    fn finish_request(&self, kind: Pending, result: Result<Value, ClientError>) {
        if crate::client::trace_enabled() {
            let label = match &kind {
                Pending::Hello => "hello".to_string(),
                Pending::Settings => "settings".to_string(),
                Pending::Reload => "reload".to_string(),
                Pending::Queues => "queues".to_string(),
                Pending::Total { offset, .. } => format!("total from {offset}"),
                Pending::Status(task) => format!("status {task}"),
                other => format!("{other:?}"),
            };
            match &result {
                Ok(value) => eprintln!(
                    "segmenta-gui: finish {label} ok ({} rows?)",
                    value.as_array().map(Vec::len).unwrap_or(0)
                ),
                Err(err) => eprintln!("segmenta-gui: finish {label} failed: {}", err.ui_message()),
            }
        }
        match kind {
            Pending::Hello => match result {
                Ok(value) => {
                    let version = value
                        .get("daemon_version")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    *self.daemon_version.borrow_mut() = version;
                }
                Err(err) => {
                    // First real diagnosis of a broken connection: surface it.
                    eprintln!("segmenta-gui: system.hello failed: {}", err.ui_message());
                    self.set_error(err.ui_message());
                }
            },
            Pending::Settings => match result {
                Ok(value) => {
                    if let Ok(settings) = serde_json::from_value::<Settings>(value) {
                        *self.settings.borrow_mut() = settings;
                        self.apply_settings_to_ui();
                    }
                }
                Err(err) => self.toast(&err.ui_message()),
            },
            Pending::Reload => match result {
                Ok(value) => match serde_json::from_value::<Vec<DownloadItem>>(value) {
                    Ok(items) => {
                        *self.last_error.borrow_mut() = None;
                        self.connected.set(true);
                        let returned = items.len();
                        // A full page may hide more rows: the snapshot is capped and
                        // painting its size as the total would be a lie.
                        let capped = returned as u64 >= LIST_LIMIT;
                        self.store.borrow_mut().reload(items);
                        if capped {
                            self.set_snapshot_total(None, false);
                            self.count_snapshot(LIST_LIMIT as usize, 1);
                        } else {
                            self.set_snapshot_total(Some(returned), true);
                        }
                        self.refill();
                        self.update_detail();
                    }
                    Err(err) => self.set_error(
                        tr(locale(), Msg::InvalidListResponse).replace("{error}", &err.to_string()),
                    ),
                },
                Err(err) => self.set_error(err.ui_message()),
            },
            Pending::Queues => match result {
                Ok(value) => {
                    let Some(rows) = value.as_array() else {
                        return;
                    };
                    let names: Vec<String> = rows
                        .iter()
                        .filter_map(|row| row.get("name").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect();
                    *self.queues.borrow_mut() = names;
                    // Repaint now: both the sidebar and the queue filter depend on it.
                    self.refill();
                }
                Err(err) => {
                    // A daemon without `queue.list` must not turn the view into an
                    // error page: the snapshot is still usable, only the queue
                    // sidebar stays as it was.
                    if crate::client::trace_enabled() {
                        eprintln!("segmenta-gui: queue.list failed: {}", err.ui_message());
                    }
                }
            },
            Pending::Total { offset, pages } => match result {
                Ok(value) => match serde_json::from_value::<Vec<DownloadItem>>(value) {
                    Ok(items) => {
                        let seen = offset + items.len();
                        let full = items.len() as u64 == COUNT_PAGE;
                        if !full {
                            self.set_snapshot_total(Some(seen), true);
                        } else if pages >= COUNT_PAGES_MAX {
                            // Reporting the bound as exact would be another silent lie.
                            self.set_snapshot_total(Some(seen), false);
                        } else {
                            self.count_snapshot(seen, pages + 1);
                            return;
                        }
                        self.refresh_truncation();
                    }
                    Err(err) => {
                        if crate::client::trace_enabled() {
                            eprintln!("segmenta-gui: download.list count failed: {err}");
                        }
                    }
                },
                Err(err) => {
                    // Without a total the notice says the snapshot has more rows
                    // instead of inventing a number.
                    if crate::client::trace_enabled() {
                        eprintln!(
                            "segmenta-gui: download.list count failed: {}",
                            err.ui_message()
                        );
                    }
                }
            },
            Pending::Status(task_id) => match result {
                Ok(value) => {
                    if let Ok(item) = serde_json::from_value::<DownloadItem>(value) {
                        let segments = item.segments.clone();
                        let total = item.total_bytes.unwrap_or(0);
                        self.store.borrow_mut().upsert(item);
                        self.sync_row(&task_id);
                        self.update_detail();
                        let _ = (segments, total);
                    }
                }
                Err(err) => self.toast(&err.ui_message()),
            },
            Pending::Pause(task_id)
            | Pending::Resume(task_id)
            | Pending::Cancel(task_id)
            | Pending::Delete(task_id) => match result {
                Ok(_) => {
                    self.store.borrow_mut().remove(&task_id);
                    self.selected.borrow_mut().remove(&task_id);
                    if self
                        .detail
                        .borrow()
                        .as_ref()
                        .map(|detail| detail.task_id == task_id)
                        .unwrap_or(false)
                    {
                        self.close_detail();
                    }
                    self.reload_all();
                }
                Err(err) => self.toast(&err.ui_message()),
            },
            Pending::CleanCompleted => match result {
                Ok(value) => match serde_json::from_value::<CleanCompletedReport>(value) {
                    Ok(report) => {
                        // The answer is known: only now does the dialog go away.
                        self.close_batch_dialog();
                        self.toast(&report.summary(locale()));
                        *self.last_clean_report.borrow_mut() = Some(report);
                        self.reload_all();
                    }
                    Err(err) => self.fail_batch_dialog(
                        &tr(locale(), Msg::InvalidDaemonResponse)
                            .replace("{error}", &err.to_string()),
                    ),
                },
                Err(err) => self.fail_batch_dialog(&err.ui_message()),
            },
            Pending::DeleteMany => match result {
                Ok(value) => match serde_json::from_value::<BatchDeleteReport>(value) {
                    Ok(report) => {
                        // Only the tasks the daemon removed leave the list: a failed
                        // one has to stay visible with its error.
                        let removed: Vec<String> = report
                            .results
                            .iter()
                            .filter(|entry| entry.ok)
                            .map(|entry| entry.task_id.clone())
                            .collect();
                        self.store.borrow_mut().remove_many(&removed);
                        {
                            let mut selected = self.selected.borrow_mut();
                            for task_id in &removed {
                                selected.remove(task_id);
                            }
                        }
                        // Repaint before the reload answers: the removed rows must be
                        // gone as soon as the result is known, not a tick later.
                        self.refill();
                        self.update_detail();
                        self.close_batch_dialog();
                        self.toast(&report.summary(locale()));
                        *self.last_delete_report.borrow_mut() = Some(report);
                        self.reload_all();
                    }
                    Err(err) => self.fail_batch_dialog(
                        &tr(locale(), Msg::InvalidDaemonResponse)
                            .replace("{error}", &err.to_string()),
                    ),
                },
                Err(err) => self.fail_batch_dialog(&err.ui_message()),
            },
        }
    }

    fn handle_notification(&self, method: &str, params: Value) {
        match method {
            "download.event" => self.handle_download_event(params),
            "system.resync_required" => {
                // The daemon lost events for this client: reload everything.
                self.reload_all();
                if let Some(task_id) = self
                    .detail
                    .borrow()
                    .as_ref()
                    .map(|detail| detail.task_id.clone())
                {
                    self.refresh_task(&task_id);
                }
            }
            _ => {}
        }
    }

    fn handle_download_event(&self, params: Value) {
        let event = match DownloadEvent::parse(params) {
            Some(event) => event,
            None => return,
        };
        match self.store.borrow_mut().observe_seq(event.seq_id) {
            SeqStatus::Duplicate => return,
            SeqStatus::Gap => {
                // Never patch a gap: reload so no row is lost or duplicated.
                self.reload_all();
                return;
            }
            SeqStatus::InOrder => {}
        }

        let Some(task_id) = event.task_id.clone() else {
            return;
        };
        match event.event_type.as_str() {
            "task_progress" => {
                let Some(progress) = event.progress() else {
                    return;
                };
                if self.store.borrow_mut().apply_progress(&task_id, &progress) {
                    self.sync_row(&task_id);
                    self.update_detail();
                } else {
                    // The event overtook the first list answer: reload instead of
                    // dropping the row.
                    self.reload_all();
                }
            }
            "state_changed" => {
                let status = event
                    .data
                    .get("new_state")
                    .and_then(Value::as_str)
                    .map(DownloadStatus::from_wire)
                    .unwrap_or(DownloadStatus::Unknown);
                if self.store.borrow_mut().apply_state(&task_id, status) {
                    self.sync_row(&task_id);
                    self.update_detail();
                    if status == DownloadStatus::Completed {
                        self.notify_completion(&task_id);
                    }
                    self.refresh_task(&task_id);
                } else {
                    self.reload_all();
                }
            }
            "task_completed" => {
                if self
                    .store
                    .borrow_mut()
                    .apply_state(&task_id, DownloadStatus::Completed)
                {
                    self.sync_row(&task_id);
                    self.update_detail();
                }
                self.notify_completion(&task_id);
                self.refresh_task(&task_id);
            }
            "task_failed" => {
                let code = event
                    .data
                    .get("error_code")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let message = event
                    .data
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error desconocido")
                    .to_string();
                if self
                    .store
                    .borrow_mut()
                    .apply_error(&task_id, code, &message)
                {
                    self.sync_row(&task_id);
                    self.update_detail();
                }
                let filename = self
                    .store
                    .borrow()
                    .get(&task_id)
                    .map(|item| item.filename.clone())
                    .unwrap_or_else(|| task_id.clone());
                self.notify(tr(locale(), Msg::FailedNotification), &filename);
                self.refresh_task(&task_id);
            }
            "task_added" | "download_restarted_due_to_stale_cache" => {
                self.refresh_task(&task_id);
            }
            "segment_started" | "segment_split" | "segment_completed" => {
                let selected = self
                    .detail
                    .borrow()
                    .as_ref()
                    .map(|detail| detail.task_id.clone());
                if selected.as_deref() == Some(task_id.as_str()) {
                    self.refresh_task(&task_id);
                }
            }
            _ => {}
        }
    }

    fn notify_completion(&self, task_id: &str) {
        let filename = self
            .store
            .borrow()
            .get(task_id)
            .map(|item| item.filename.clone())
            .unwrap_or_else(|| task_id.to_string());
        self.notify(tr(locale(), Msg::CompletedNotification), &filename);
    }

    fn notify(&self, title: &str, body: &str) {
        let notification = gtk::gio::Notification::new(title);
        notification.set_body(Some(body));
        notification.set_icon(&gtk::gio::ThemedIcon::new(APP_ID));
        self.app.send_notification(None, &notification);
    }

    fn toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        toast.set_timeout(4);
        self.toast_overlay.add_toast(toast);
    }

    fn set_loading(&self) {
        if crate::client::trace_enabled() {
            eprintln!("segmenta-gui: view -> Loading");
        }
        *self.view_state.borrow_mut() = ViewState::Loading;
        // A reconnect with data already on screen must not cover the list with the
        // spinner: the loading page only belongs to a cold start or an empty list.
        if !self.store.borrow().is_empty() {
            return;
        }
        // The loading page is detached once the first Ready arrives; put it back for a
        // retry or a reconnect.
        if self.stack.child_by_name("loading").is_none() {
            self.stack.add_named(&self.loading_page, Some("loading"));
        }
        self.stack.set_visible_child_name("loading");
    }

    /// Detach the loading page; a widget that is not in the stack cannot paint.
    fn stop_loading(&self) {
        if crate::client::trace_enabled() {
            eprintln!(
                "segmenta-gui: stop_loading (page attached={})",
                self.stack.child_by_name("loading").is_some()
            );
        }
        // Detach the page from the stack: a hidden widget cannot be painted.
        if let Some(child) = self.stack.child_by_name("loading") {
            self.stack.remove(&child);
        }
    }

    fn set_error(&self, message: String) {
        self.stack.set_visible_child_name("error");
        self.stop_loading();
        if crate::client::trace_enabled() {
            eprintln!("segmenta-gui: view -> Error({message})");
        }
        *self.last_error.borrow_mut() = Some(message.clone());
        *self.view_state.borrow_mut() = ViewState::Error(message.clone());
        let here = locale();
        self.error_page.set_title(tr(here, Msg::ErrorTitle));
        self.error_page.set_description(Some(&format!(
            "{message}\n\n{}\n{}",
            tr(here, Msg::ErrorBody),
            tr(here, Msg::ErrorCommand)
        )));
    }

    /// Retry the connection and reload the view; the error button uses this.
    fn retry_connection(&self) {
        self.client.reconnect();
        self.set_loading();
        self.reload_all();
        self.load_settings();
    }

    fn set_filter(self: &Rc<Self>, filter: Filter, update_selection: bool) {
        self.filter.set(filter);
        if update_selection {
            let index = Filter::ALL
                .iter()
                .position(|candidate| *candidate == filter)
                .unwrap_or(0) as i32;
            if let Some(row) = self.filter_list.row_at_index(index) {
                self.filter_list.select_row(Some(&row));
            }
        }
        self.refill();
    }

    fn index_of(&self, task_id: &str) -> Option<u32> {
        list_items(&self.list_store)
            .iter()
            .position(|item| item.task_id == task_id)
            .map(|index| index as u32)
    }

    /// Rebuilds the visible list from the snapshot.
    /// Dumps every spinner-ish widget with its visibility, mapping and parent chain.
    ///
    /// Opt-in with `SEGMENTA_DEBUG_TREE=1`: a painted spinner that no field of this module
    /// claims to own is otherwise invisible to the tests. Prints widget types and nothing
    /// else, so it never leaks a URL or a path.
    fn dump_spinner_tree(&self) {
        if std::env::var_os("SEGMENTA_DEBUG_TREE").is_none() {
            return;
        }
        fn walk(widget: &gtk::Widget, depth: usize) {
            let name = widget.type_().name();
            if name.contains("Spinner") {
                let mut chain = Vec::new();
                let mut current = widget.parent();
                while let Some(parent) = current {
                    chain.push(parent.type_().name().to_string());
                    current = parent.parent();
                }
                eprintln!(
                    "segmenta-gui: spinner {name} visible={} mapped={} depth={depth} parents={chain:?}",
                    widget.is_visible(),
                    widget.is_mapped()
                );
            }
            let mut next = widget.first_child();
            while let Some(child) = next {
                walk(&child, depth + 1);
                next = child.next_sibling();
            }
        }
        walk(self.window.upcast_ref::<gtk::Widget>(), 0);
    }

    fn refill(&self) {
        let filter = self.filter.get();
        let query = self.query.borrow().clone();
        let queue = self.queue_filter();
        let visible: Vec<DownloadItem> = self
            .store
            .borrow()
            .filtered(filter, &query)
            .into_iter()
            .filter(|item| match &queue {
                Some(name) => &item.queue_name == name,
                None => true,
            })
            .collect();

        let objects: Vec<gtk::glib::BoxedAnyObject> = visible
            .iter()
            .cloned()
            .map(gtk::glib::BoxedAnyObject::new)
            .collect();
        let existing = self.list_store.n_items();
        self.list_store.splice(0, existing, &objects);

        // A refresh keeps the selection of every task the daemon still lists and
        // drops the ones it no longer knows; a checked row can never act on a ghost.
        {
            let store = self.store.borrow();
            self.selected
                .borrow_mut()
                .retain(|task_id| store.get(task_id).is_some());
        }

        let counts = self.store.borrow().counts();
        for (filter, label) in self.count_labels.borrow().iter() {
            label.set_label(&counts.of(*filter).to_string());
        }
        self.refresh_actions();
        self.refresh_queues();
        self.refresh_truncation();

        let state = if !visible.is_empty() {
            ViewState::Ready
        } else if let Some(error) = self.last_error.borrow().clone() {
            ViewState::Error(error)
        } else if self.store.borrow().is_empty() {
            ViewState::Empty
        } else {
            // The snapshot has rows but the current filter hides them.
            ViewState::Empty
        };
        self.empty_page.set_title(filter.empty_title());
        match state {
            ViewState::Empty => self.stack.set_visible_child_name("empty"),
            ViewState::Ready => self.stack.set_visible_child_name("list"),
            ViewState::Error(_) => self.stack.set_visible_child_name("error"),
            ViewState::Loading => self.stack.set_visible_child_name("loading"),
        }
        if crate::client::trace_enabled() {
            eprintln!(
                "segmenta-gui: refill state={:?} rows={}",
                state,
                self.store.borrow().len()
            );
        }
        // Leaving Loading must stop the spinner *after* the stack switched, otherwise
        // the stopped glyph stays painted behind the list. A list with rows is the
        // strongest evidence there is nothing to load, so the spinner goes away even if
        // the state machine has not caught up: the X11 capture showed it painted over a
        // fully populated list and through the preferences dialog.
        if !matches!(state, ViewState::Loading) || !self.store.borrow().is_empty() {
            self.stop_loading();
        }
        *self.view_state.borrow_mut() = state;
        self.dump_spinner_tree();
    }

    /// Replaces one row in place when it stays visible, else rebuilds.
    fn sync_row(&self, task_id: &str) {
        let Some(item) = self.store.borrow().get(task_id).cloned() else {
            return;
        };
        let filter = self.filter.get();
        let query = self.query.borrow().clone();
        let queue = self.queue_filter();
        let visible = filter.matches(item.status)
            && item.matches(&query)
            && queue
                .as_deref()
                .map(|name| item.queue_name == name)
                .unwrap_or(true);
        match (visible, self.index_of(task_id)) {
            (true, Some(index)) => {
                let selected = self.selected_task_id();
                let object = gtk::glib::BoxedAnyObject::new(item);
                self.list_store.splice(index, 1, &[object]);
                if let Some(selected) = selected {
                    match self.index_of(&selected) {
                        Some(index) => self.selection.set_selected(index),
                        None => self.selection.set_selected(gtk::INVALID_LIST_POSITION),
                    }
                }
                let counts = self.store.borrow().counts();
                for (filter, label) in self.count_labels.borrow().iter() {
                    label.set_label(&counts.of(*filter).to_string());
                }
                if !matches!(*self.view_state.borrow(), ViewState::Ready) {
                    self.refill();
                }
            }
            _ => self.refill(),
        }
        self.refresh_actions();
    }

    fn refresh_queues(&self) {
        // Queue names come from `queue.list`, never from the downloads: an empty
        // queue must stay visible and a dropped one must disappear.
        let names = self.queues.borrow().clone();
        self.queue_group.set_visible(!names.is_empty());

        let current = self.queue.borrow().clone();
        self.rebuilding_queues.set(true);
        while let Some(child) = self.queue_list.first_child() {
            self.queue_list.remove(&child);
        }
        for name in &names {
            let row = gtk::ListBoxRow::new();
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let label = gtk::Label::new(Some(name));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            let count = self
                .store
                .borrow()
                .items()
                .iter()
                .filter(|item| &item.queue_name == name)
                .count();
            let count_label = gtk::Label::new(Some(&count.to_string()));
            count_label.add_css_class("dim-label");
            line.append(&label);
            line.append(&count_label);
            row.set_child(Some(&line));
            self.queue_list.append(&row);
        }
        if let Some(current) = current {
            if let Some(index) = names.iter().position(|name| *name == current) {
                if let Some(row) = self.queue_list.row_at_index(index as i32) {
                    self.queue_list.select_row(Some(&row));
                }
            }
        }
        self.rebuilding_queues.set(false);
    }

    /// Names painted by the queue sidebar, read from the rows themselves so a test
    /// can assert what the user sees instead of what the state claims.
    fn queue_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut child = self.queue_list.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Ok(row) = widget.downcast::<gtk::ListBoxRow>() {
                if let Some(name) = queue_row_name(&row) {
                    names.push(name);
                }
            }
        }
        names
    }

    fn apply_settings_to_ui(&self) {
        // The header has no settings-bound widget yet beyond the add dialog
        // defaults, but preferences must be visible in the next dialog.
    }

    fn open_detail(self: &Rc<Self>, task_id: &str) {
        if let Some(detail) = self.detail.borrow().as_ref() {
            if detail.task_id == task_id {
                self.update_detail();
                return;
            }
        }
        let Some(item) = self.store.borrow().get(task_id).cloned() else {
            return;
        };
        self.close_detail();

        let detail = self.build_detail_page(&item);
        self.nav.push(&detail.page);
        *self.detail.borrow_mut() = Some(detail);
        self.refresh_task(task_id);

        if let Some(detail) = self.detail.borrow().as_ref() {
            update_detail_widgets(detail, &item);
        }
    }

    fn close_detail(&self) {
        if self.detail.borrow_mut().take().is_some() {
            self.selection.set_selected(gtk::INVALID_LIST_POSITION);
            self.nav.pop_to_tag("downloads");
        }
    }

    fn selected_task_id(&self) -> Option<String> {
        self.selection
            .selected_item()
            .and_downcast::<gtk::glib::BoxedAnyObject>()
            .and_then(|boxed| {
                boxed
                    .try_borrow::<DownloadItem>()
                    .ok()
                    .map(|item| item.task_id.clone())
            })
    }

    fn build_detail_page(self: &Rc<Self>, item: &DownloadItem) -> DetailPage {
        let filename = gtk::Label::new(Some(&item.filename));
        filename.set_xalign(0.0);
        filename.set_wrap(true);
        filename.add_css_class("title-2");
        filename.set_selectable(true);

        let path = gtk::Label::new(Some(&item.final_path));
        path.set_xalign(0.0);
        path.set_selectable(true);
        path.set_wrap(true);
        path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        path.add_css_class("dim-label");
        path.set_tooltip_text(Some(&item.final_path));

        let state = gtk::Label::new(Some(item.status.label(locale())));
        state.add_css_class("pill");
        state.set_halign(gtk::Align::Start);

        let progress = gtk::ProgressBar::new();
        progress.set_fraction(item.progress());
        progress.set_show_text(false);
        progress.set_hexpand(true);

        let percent = gtk::Label::new(Some(&format!("{:.0} %", item.progress() * 100.0)));
        percent.add_css_class("numeric");
        percent.add_css_class("dim-label");

        let speed = gtk::Label::new(Some(&format_speed_label(item.speed_bps)));
        let eta = gtk::Label::new(Some(&eta_label(item.eta_seconds)));
        let size = gtk::Label::new(Some(&item.size_line()));
        for label in [&speed, &eta, &size] {
            label.add_css_class("dim-label");
        }

        let meta_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        meta_row.append(&speed);
        meta_row.append(&eta);
        meta_row.append(&size);

        let segments = SegmentaSegmentView::default();
        segments.set_size_request(-1, 14);
        segments.set_segments(&item.segments, item.total_bytes.unwrap_or(0));
        let segment_label = gtk::Label::new(Some(&segments.accessible_label()));
        segment_label.set_xalign(0.0);
        segment_label.add_css_class("dim-label");
        segment_label.add_css_class("caption");

        let progress_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        progress_row.append(&progress);
        progress_row.append(&percent);

        let error = gtk::Label::new(item.error_message.as_deref());
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.set_selectable(true);
        error.add_css_class("error");
        error.set_visible(item.error_message.is_some());

        let pause = gtk::Button::with_label("Pausar");
        let resume = gtk::Button::with_label("Reanudar");
        let cancel = gtk::Button::with_label("Cancelar");
        cancel.add_css_class("destructive-action");
        let delete = gtk::Button::with_label("Eliminar");
        delete.add_css_class("destructive-action");
        let open_folder = gtk::Button::with_label("Abrir carpeta");

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_margin_top(6);
        for button in [&pause, &resume, &cancel, &delete, &open_folder] {
            actions.append(button);
        }

        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.append(&filename);
        body.append(&path);
        body.append(&state);
        body.append(&progress_row);
        body.append(&meta_row);
        body.append(&segments);
        body.append(&segment_label);
        body.append(&error);
        body.append(&actions);

        let clamp = adw::Clamp::new();
        clamp.set_maximum_size(680);
        clamp.set_child(Some(&body));

        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&clamp));

        let page = adw::NavigationPage::builder()
            .title(item.filename.clone())
            .child(&toolbar)
            .build();
        page.set_tag(Some("detail"));

        let detail = DetailPage {
            task_id: item.task_id.clone(),
            page,
            filename,
            path,
            state,
            progress,
            percent,
            speed,
            eta,
            size,
            error,
            segments,
            segment_label,
            pause,
            resume,
            cancel,
            delete,
            open_folder,
        };

        let handle = Rc::downgrade(self);
        let task_id = item.task_id.clone();
        let button_handle = handle.clone();
        let button_task = task_id.clone();
        detail.pause.connect_clicked(move |_| {
            if let Some(inner) = button_handle.upgrade() {
                inner.request(
                    "download.pause",
                    json!({ "task_id": button_task }),
                    Pending::Pause(button_task.clone()),
                );
            }
        });

        let button_handle = handle.clone();
        let button_task = task_id.clone();
        detail.resume.connect_clicked(move |_| {
            if let Some(inner) = button_handle.upgrade() {
                inner.request(
                    "download.resume",
                    json!({ "task_id": button_task }),
                    Pending::Resume(button_task.clone()),
                );
            }
        });

        let button_handle = handle.clone();
        let button_task = task_id.clone();
        detail.cancel.connect_clicked(move |_| {
            let Some(inner) = button_handle.upgrade() else {
                return;
            };
            inner.confirm(
                tr(locale(), Msg::ConfirmCancelTitle),
                tr(locale(), Msg::ConfirmCancelBodyLong),
                tr(locale(), Msg::Cancel),
                true,
                Rc::new({
                    let inner = Rc::clone(&inner);
                    let task = button_task.clone();
                    move || {
                        inner.request(
                            "download.cancel",
                            json!({ "task_id": task, "delete_file": false }),
                            Pending::Cancel(task.clone()),
                        );
                    }
                }),
            );
        });

        let button_handle = handle.clone();
        let button_task = task_id.clone();
        detail.delete.connect_clicked(move |_| {
            let Some(inner) = button_handle.upgrade() else {
                return;
            };
            inner.confirm(
                tr(locale(), Msg::ConfirmDeleteTitle),
                tr(locale(), Msg::ConfirmDeleteBodyLong),
                tr(locale(), Msg::Delete),
                true,
                Rc::new({
                    let inner = Rc::clone(&inner);
                    let task = button_task.clone();
                    move || {
                        inner.request(
                            "download.delete",
                            json!({ "task_id": task, "delete_file": false }),
                            Pending::Delete(task.clone()),
                        );
                    }
                }),
            );
        });

        let button_handle = handle.clone();
        let button_task = task_id.clone();
        detail.open_folder.connect_clicked(move |_| {
            let Some(inner) = button_handle.upgrade() else {
                return;
            };
            let path = inner
                .store
                .borrow()
                .get(&button_task)
                .map(|item| item.final_path.clone())
                .unwrap_or_default();
            open_folder_of(&inner.window, &path);
        });

        detail
    }

    fn update_detail(&self) {
        let task_id = match self.detail.borrow().as_ref() {
            Some(detail) => detail.task_id.clone(),
            None => return,
        };
        let Some(item) = self.store.borrow().get(&task_id).cloned() else {
            self.close_detail();
            return;
        };
        if let Some(detail) = self.detail.borrow().as_ref() {
            update_detail_widgets(detail, &item);
        }
    }

    fn confirm(
        self: &Rc<Self>,
        heading: &str,
        body: &str,
        accept: &str,
        destructive: bool,
        on_accept: Rc<dyn Fn()>,
    ) {
        let alert = adw::AlertDialog::new(Some(heading), Some(body));
        alert.add_response("cancel", "Volver");
        alert.add_response("accept", accept);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");
        if destructive {
            alert.set_response_appearance("accept", adw::ResponseAppearance::Destructive);
        }
        let window = self.window.clone();
        *self.confirm_dialog.borrow_mut() = Some(alert.clone());
        let pending = Rc::downgrade(self);
        alert.choose(
            Some(&window),
            None::<&gtk::gio::Cancellable>,
            move |response| {
                if let Some(inner) = pending.upgrade() {
                    *inner.confirm_dialog.borrow_mut() = None;
                }
                if response == "accept" {
                    on_accept();
                }
            },
        );
    }

    /// Ask for confirmation before deleting the selected task (the detail button path).
    fn begin_delete_selected(self: &Rc<Self>) {
        let Some(item) = SegmentaWindow(Rc::clone(self)).detail() else {
            return;
        };
        let task = item.task_id.clone();
        let handle = Rc::downgrade(self);
        self.confirm(
            tr(locale(), Msg::ConfirmDeleteTitle),
            tr(locale(), Msg::ConfirmDeleteBodyLong),
            tr(locale(), Msg::Delete),
            true,
            Rc::new(move || {
                if let Some(inner) = handle.upgrade() {
                    inner.request(
                        "download.delete",
                        json!({ "task_id": task, "delete_file": false }),
                        Pending::Delete(task.clone()),
                    );
                }
            }),
        );
    }
    fn fill_row(self: &Rc<Self>, widget: &gtk::Widget, item: &DownloadItem) {
        let Some(row) = widget.downcast_ref::<gtk::Box>() else {
            return;
        };
        while let Some(child) = row.first_child() {
            row.remove(&child);
        }
        row.set_spacing(10);
        row.set_margin_top(10);
        row.set_margin_bottom(10);
        row.set_margin_start(12);
        row.set_margin_end(12);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        body.set_hexpand(true);

        let title = gtk::Label::new(Some(&item.filename));
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title.set_tooltip_text(Some(&item.filename));
        title.add_css_class("heading");

        let status = gtk::Label::new(Some(item.status.label(locale())));
        status.set_xalign(0.0);
        status.add_css_class("caption");
        status.add_css_class("dim-label");

        let bar = gtk::ProgressBar::new();
        bar.set_fraction(item.progress());
        bar.set_show_text(false);
        bar.update_property(&[gtk::accessible::Property::Label(&format!(
            "Progreso de {}: {:.0} %",
            item.filename,
            item.progress() * 100.0
        ))]);

        // Media tasks carry a localized badge in front of the progress line.
        let badge = item.media_badge();
        let meta_text = if badge.is_empty() {
            item.progress_line()
        } else {
            format!("{badge} · {}", item.progress_line())
        };
        let meta = gtk::Label::new(Some(&meta_text));
        meta.set_xalign(0.0);
        meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
        meta.add_css_class("caption");
        meta.add_css_class("dim-label");

        body.append(&title);
        body.append(&status);
        body.append(&bar);
        body.append(&meta);

        if self.multi_select.get() {
            // Explicit, discoverable selection: a checkbox per row instead of a
            // hidden ctrl-click, which no menu entry could ever explain.
            row.set_orientation(gtk::Orientation::Horizontal);
            let check = gtk::CheckButton::new();
            check.set_active(self.selected.borrow().contains(&item.task_id));
            check.set_valign(gtk::Align::Center);
            check.set_tooltip_text(Some(tr(locale(), Msg::MultiSelectHint)));
            check.update_property(&[gtk::accessible::Property::Label(&format!(
                "{}: {}",
                tr(locale(), Msg::MultiSelectHint),
                item.filename
            ))]);
            let handle = Rc::downgrade(self);
            let task_id = item.task_id.clone();
            check.connect_toggled(move |check| {
                if let Some(inner) = handle.upgrade() {
                    inner.set_task_selected(&task_id, check.is_active());
                }
            });
            row.append(&check);
        } else {
            row.set_orientation(gtk::Orientation::Vertical);
        }
        row.append(&body);
    }

    fn set_multi_select(self: &Rc<Self>, enabled: bool) {
        self.multi_select.set(enabled);
        self.multi_select_action
            .set_state(&gtk::glib::Variant::from(enabled));
        // Repaint so the checkbox column appears or disappears; the selection is kept
        // by task id, so toggling the mode never loses it.
        self.refill();
    }

    fn set_task_selected(&self, task_id: &str, selected: bool) -> bool {
        if self.store.borrow().get(task_id).is_none() {
            return false;
        }
        {
            let mut ids = self.selected.borrow_mut();
            if selected {
                ids.insert(task_id.to_string());
            } else {
                ids.remove(task_id);
            }
        }
        self.refresh_actions();
        true
    }

    /// Checked ids in the order the list paints them.
    fn selected_task_ids(&self) -> Vec<String> {
        let selected = self.selected.borrow();
        self.store
            .borrow()
            .items()
            .iter()
            .filter(|item| selected.contains(&item.task_id))
            .map(|item| item.task_id.clone())
            .collect()
    }

    /// Gates the destructive actions on real state: a completed row exists, a task is
    /// checked. A disabled menu entry is a documented state, never a silent no-op.
    fn refresh_actions(&self) {
        let completed = self.store.borrow().counts().completed;
        self.clean_action.set_enabled(completed > 0);
        let has_selection = !self.selected.borrow().is_empty();
        self.delete_selection_action.set_enabled(has_selection);
    }

    /// "Limpiar completadas": only `completed` rows, never a file.
    fn begin_clean_completed(self: &Rc<Self>) {
        let count = self.store.borrow().counts().completed;
        if count == 0 {
            self.toast(tr(locale(), Msg::CleanCompletedNone));
            return;
        }
        let body = tr(locale(), Msg::CleanCompletedBody).replace("{count}", &count.to_string());
        self.present_batch_dialog(
            tr(locale(), Msg::CleanCompletedTitle),
            body,
            tr(locale(), Msg::CleanCompleted),
            None,
            BatchKind::CleanCompleted { count },
        );
    }

    /// "Eliminar" over the checked tasks, with the file checkbox off by default.
    fn begin_delete_selection(self: &Rc<Self>) {
        let task_ids = self.selected_task_ids();
        if task_ids.is_empty() {
            self.toast(tr(locale(), Msg::DeleteSelectionNone));
            return;
        }
        let summary = self.store.borrow().selection_summary(&task_ids);
        self.present_batch_dialog(
            tr(locale(), Msg::DeleteSelectionTitle),
            summary.body(locale()),
            tr(locale(), Msg::Delete),
            Some((
                tr(locale(), Msg::DeleteFilesCheckbox),
                tr(locale(), Msg::DeleteFilesCheckboxHint),
            )),
            BatchKind::DeleteSelection { task_ids, summary },
        );
    }

    fn present_batch_dialog(
        self: &Rc<Self>,
        heading: &str,
        body: String,
        accept_label: &str,
        checkbox: Option<(&str, &str)>,
        kind: BatchKind,
    ) {
        self.close_batch_dialog();
        let dialog = adw::Dialog::builder()
            .title(heading)
            .content_width(460)
            .follows_content_size(true)
            .build();

        let header = adw::HeaderBar::new();
        let cancel = gtk::Button::with_label(tr(locale(), Msg::Cancel));
        header.pack_start(&cancel);
        let accept = gtk::Button::with_label(accept_label);
        accept.add_css_class("destructive-action");
        header.pack_end(&accept);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);

        let title = gtk::Label::new(Some(heading));
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.add_css_class("title-4");
        let body_label = gtk::Label::new(Some(&body));
        body_label.set_xalign(0.0);
        body_label.set_wrap(true);
        content.append(&title);
        content.append(&body_label);

        let delete_files = checkbox.map(|(label, hint)| {
            let check = gtk::CheckButton::with_label(label);
            // Destructive by nature: it always starts unchecked.
            check.set_active(false);
            check.set_tooltip_text(Some(hint));
            content.append(&check);
            let hint_label = gtk::Label::new(Some(hint));
            hint_label.set_xalign(0.0);
            hint_label.set_wrap(true);
            hint_label.add_css_class("caption");
            hint_label.add_css_class("dim-label");
            content.append(&hint_label);
            check
        });

        let error = gtk::Label::new(None);
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.set_selectable(true);
        error.add_css_class("error");
        error.set_visible(false);
        content.append(&error);

        // Keyboard: Enter is the safe answer, so it never confirms a destructive
        // action; Escape closes the dialog through AdwDialog itself.
        let keys = gtk::EventControllerKey::new();
        let key_handle = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, keyval, _, _| {
            if matches!(keyval, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) {
                if let Some(inner) = key_handle.upgrade() {
                    inner.dismiss_batch_dialog();
                }
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        content.add_controller(keys);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&content));
        dialog.set_child(Some(&toolbar));

        let handle = Rc::downgrade(self);
        cancel.connect_clicked(move |_| {
            if let Some(inner) = handle.upgrade() {
                inner.dismiss_batch_dialog();
            }
        });
        let handle = Rc::downgrade(self);
        accept.connect_clicked(move |_| {
            if let Some(inner) = handle.upgrade() {
                inner.accept_batch_dialog();
            }
        });
        let handle = Rc::downgrade(self);
        dialog.connect_closed(move |_| {
            if let Some(inner) = handle.upgrade() {
                inner.batch_dialog.borrow_mut().take();
            }
        });

        let checkbox_label = checkbox.map(|(label, _)| label.to_string());
        *self.batch_dialog.borrow_mut() = Some(BatchDialog {
            dialog: dialog.clone(),
            body: body_label,
            error,
            accept,
            cancel,
            delete_files,
            checkbox_label,
            kind,
            accept_label: accept_label.to_string(),
            waiting: Cell::new(false),
        });
        dialog.present(Some(&self.window));
    }

    /// Answers the open batch dialog with its accept action.
    fn accept_batch_dialog(self: &Rc<Self>) -> bool {
        let (kind, delete_files) = {
            let open = self.batch_dialog.borrow();
            let Some(dialog) = open.as_ref() else {
                return false;
            };
            if dialog.waiting.get() {
                return false;
            }
            let delete_files = dialog
                .delete_files
                .as_ref()
                .is_some_and(|check| check.is_active());
            (dialog.kind.clone(), delete_files)
        };
        if let Some(dialog) = self.batch_dialog.borrow().as_ref() {
            dialog.waiting.set(true);
            // Escape must not hide a destructive request whose outcome is unknown.
            dialog.dialog.set_can_close(false);
            dialog.accept.set_sensitive(false);
            dialog.cancel.set_sensitive(false);
            if let Some(check) = dialog.delete_files.as_ref() {
                check.set_sensitive(false);
            }
            dialog.accept.set_label(match &dialog.kind {
                BatchKind::CleanCompleted { .. } => tr(locale(), Msg::Cleaning),
                BatchKind::DeleteSelection { .. } => tr(locale(), Msg::Deleting),
            });
        }
        match kind {
            BatchKind::CleanCompleted { .. } => {
                self.request(
                    "download.clean_completed",
                    json!({}),
                    Pending::CleanCompleted,
                );
            }
            BatchKind::DeleteSelection { task_ids, .. } => {
                self.request(
                    "download.delete_many",
                    json!({ "task_ids": task_ids, "delete_files": delete_files }),
                    Pending::DeleteMany,
                );
            }
        }
        true
    }

    /// Closes the dialog as the safe answer (nothing is sent).
    fn dismiss_batch_dialog(&self) {
        // The borrow is scoped: `close()` re-enters the `closed` handler, which takes
        // the same cell, and holding the `RefMut` across it aborts the process.
        let taken = self.batch_dialog.borrow_mut().take();
        let Some(dialog) = taken else {
            return;
        };
        if dialog.waiting.get() {
            // A destructive request is in flight: hiding the dialog would hide an
            // outcome nobody knows yet.
            *self.batch_dialog.borrow_mut() = Some(dialog);
            return;
        }
        dialog.dialog.close();
    }

    fn close_batch_dialog(&self) {
        let taken = self.batch_dialog.borrow_mut().take();
        let Some(dialog) = taken else {
            return;
        };
        dialog.dialog.close();
    }

    /// Keeps the dialog open on a failure so the error is readable and retryable.
    fn fail_batch_dialog(&self, message: &str) {
        if let Some(dialog) = self.batch_dialog.borrow().as_ref() {
            dialog.error.set_label(message);
            dialog.error.set_visible(true);
            dialog.waiting.set(false);
            dialog.dialog.set_can_close(true);
            dialog.accept.set_sensitive(true);
            dialog.cancel.set_sensitive(true);
            dialog.accept.set_label(&dialog.accept_label);
            if let Some(check) = dialog.delete_files.as_ref() {
                check.set_sensitive(true);
            }
        }
        self.toast(message);
    }
}

/// Queue name painted by one sidebar row: the first label of its box.
fn queue_row_name(row: &gtk::ListBoxRow) -> Option<String> {
    row.child()
        .and_then(|child| child.downcast::<gtk::Box>().ok())
        .and_then(|line| line.first_child())
        .and_then(|label| label.downcast::<gtk::Label>().ok())
        .map(|label| label.label().to_string())
}

fn format_speed_label(speed: u64) -> String {
    crate::model::format_speed(speed)
}

fn eta_label(seconds: Option<u64>) -> String {
    crate::model::format_eta(seconds)
}

fn update_detail_widgets(detail: &DetailPage, item: &DownloadItem) {
    detail.filename.set_label(&item.filename);
    detail.path.set_label(&item.final_path);
    detail.path.set_tooltip_text(Some(&item.final_path));
    detail.state.set_label(item.status.label(locale()));
    detail.progress.set_fraction(item.progress());
    let percent = format!("{:.0} %", item.progress() * 100.0);
    detail.percent.set_label(&percent);
    detail
        .progress
        .update_property(&[gtk::accessible::Property::Label(&format!(
            "Progreso: {percent}"
        ))]);
    detail.speed.set_label(&format_speed_label(item.speed_bps));
    detail.eta.set_label(&eta_label(item.eta_seconds));
    detail.size.set_label(&item.size_line());
    detail
        .error
        .set_label(item.error_message.as_deref().unwrap_or(""));
    detail.error.set_visible(item.error_message.is_some());
    detail
        .segments
        .set_segments(&item.segments, item.total_bytes.unwrap_or(0));
    detail
        .segment_label
        .set_label(&detail.segments.accessible_label());
    detail.pause.set_visible(item.status.can_pause());
    detail.resume.set_visible(item.status.can_resume());
    detail.cancel.set_visible(item.status.can_cancel());
    detail.delete.set_visible(item.status.can_delete());
    detail.open_folder.set_visible(!item.final_path.is_empty());
}

fn list_items(store: &gtk::gio::ListStore) -> Vec<DownloadItem> {
    let mut items = Vec::new();
    for index in 0..store.n_items() {
        let Some(object) = store.item(index) else {
            continue;
        };
        let Ok(boxed) = object.downcast::<gtk::glib::BoxedAnyObject>() else {
            continue;
        };
        let item = boxed
            .try_borrow::<DownloadItem>()
            .map(|item| item.clone())
            .ok();
        if let Some(item) = item {
            items.push(item);
        }
    }
    items
}

fn open_folder_of(window: &adw::ApplicationWindow, path: &str) {
    if path.is_empty() {
        return;
    }
    let folder = std::path::Path::new(path)
        .parent()
        .map(|parent| parent.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from(path));
    let uri = match gtk::glib::filename_to_uri(&folder, None::<&str>) {
        Ok(uri) => uri.to_string(),
        Err(err) => {
            let toast = adw::Toast::new(&format!("No se pudo abrir la carpeta: {err}"));
            let _ = window;
            let _ = toast;
            return;
        }
    };
    if let Err(err) =
        gtk::gio::AppInfo::launch_default_for_uri(&uri, None::<&gtk::gio::AppLaunchContext>)
    {
        let toast = adw::Toast::new(&format!("No se pudo abrir la carpeta: {err}"));
        let _ = window;
        let _ = toast;
    }
}

/// Minimal parsing of one `download.event` notification.
struct DownloadEvent {
    seq_id: u64,
    event_type: String,
    task_id: Option<String>,
    data: Value,
}

impl DownloadEvent {
    fn parse(params: Value) -> Option<DownloadEvent> {
        Some(DownloadEvent {
            seq_id: params.get("seq_id").and_then(Value::as_u64)?,
            event_type: params.get("event_type")?.as_str()?.to_string(),
            task_id: params
                .get("task_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            data: params.get("data").cloned().unwrap_or(Value::Null),
        })
    }

    fn progress(&self) -> Option<ProgressUpdate> {
        serde_json::from_value(self.data.clone()).ok()
    }
}

impl DownloadStatus {
    /// Parses a state coming from an event payload.
    fn from_wire(state: &str) -> DownloadStatus {
        serde_json::from_value::<DownloadStatus>(Value::String(state.to_string()))
            .unwrap_or(DownloadStatus::Unknown)
    }
}

/// Segments of a status answer, exposed for the detail page and tests.
#[allow(dead_code)]
pub(crate) fn segments_of(item: &DownloadItem) -> &[SegmentItem] {
    &item.segments
}
