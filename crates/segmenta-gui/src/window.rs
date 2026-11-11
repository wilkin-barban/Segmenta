//! Main window: header bar, filter sidebar, download list and detail page.
//!
//! The window owns no socket: every daemon call goes through [`Client`] on a
//! worker thread and the results are applied by a `glib` timeout tick, so the
//! GTK main loop never blocks.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use adw::prelude::*;

use crate::client::{Client, ClientError, ClientEvent, RequestBus};
use segmenta_i18n::{tr, Msg};

use crate::model::{
    DownloadItem, DownloadStatus, Filter, ProgressUpdate, SegmentItem, SeqStatus, Settings,
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
    view_state: RefCell<ViewState>,
    add_dialog: RefCell<Option<adw::Dialog>>,
    confirm_dialog: RefCell<Option<adw::AlertDialog>>,
    last_error: RefCell<Option<String>>,
    settings: RefCell<Settings>,
    bus: RequestBus,
    pending: RefCell<Vec<(u64, Pending)>>,
    detail: RefCell<Option<DetailPage>>,
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
            view_state: RefCell::new(ViewState::Loading),
            add_dialog: RefCell::new(None),
            confirm_dialog: RefCell::new(None),
            last_error: RefCell::new(None),
            settings: RefCell::new(Settings::default()),
            bus: RequestBus::new(),
            pending: RefCell::new(Vec::new()),
            detail: RefCell::new(None),
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

    /// Whether the search bar is open.
    pub fn search_active(&self) -> bool {
        self.0.search_bar.is_search_mode()
    }

    /// Detail actions currently offered for the selected task.
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
        if !item.final_path.is_empty() {
            actions.push("open_folder".to_string());
        }
        actions
    }

    /// Ask for confirmation to delete the selected task (does not mutate anything).
    pub fn begin_delete(&self) {
        self.0.begin_delete_selected();
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
        let spinner = gtk::Spinner::new();
        spinner.start();
        let loading = gtk::Box::new(gtk::Orientation::Vertical, 12);
        loading.set_valign(gtk::Align::Center);
        loading.set_halign(gtk::Align::Center);
        loading.append(&spinner);
        let loading_label = gtk::Label::new(Some(tr(locale(), Msg::Loading)));
        loading_label.add_css_class("dim-label");
        loading.append(&loading_label);

        self.stack
            .set_transition_type(gtk::StackTransitionType::Crossfade);
        self.stack.add_named(&loading, Some("loading"));
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
            let name = row
                .and_then(|row| row.child())
                .and_then(|child| child.downcast::<gtk::Box>().ok())
                .and_then(|line| line.first_child())
                .and_then(|label| label.downcast::<gtk::Label>().ok())
                .map(|label| label.label().to_string());
            *inner.queue.borrow_mut() = name;
            inner.refill();
        });

        scrolled.set_child(Some(&container));
        scrolled.upcast()
    }

    fn build_list_page(self: &Rc<Self>) -> gtk::Widget {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, object| {
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 6);
            list_item.set_child(Some(&placeholder));
            list_item.set_activatable(true);
        });
        factory.connect_bind(|_, object| {
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
            if let Some(child) = list_item.child() {
                fill_row(&child, &item);
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
        self.list_view.set_single_click_activate(true);

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
        scrolled.upcast()
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
    }

    fn install_shortcuts(self: &Rc<Self>) {
        self.app.set_accels_for_action("win.add", &["<Control>n"]);
        self.app
            .set_accels_for_action("win.search", &["<Control>f"]);
        self.app
            .set_accels_for_action("win.preferences", &["<Control>comma"]);
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
                        self.store.borrow_mut().reload(items);
                        self.refill();
                        self.update_detail();
                    }
                    Err(err) => self.set_error(
                        tr(locale(), Msg::InvalidListResponse).replace("{error}", &err.to_string()),
                    ),
                },
                Err(err) => self.set_error(err.ui_message()),
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
                self.notify("Descarga fallida", &filename);
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
        self.notify("Descarga completada", &filename);
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
        self.stack.set_visible_child_name("loading");
    }

    fn set_error(&self, message: String) {
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
        self.stack.set_visible_child_name("error");
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
    fn refill(&self) {
        let filter = self.filter.get();
        let query = self.query.borrow().clone();
        let queue = self.queue.borrow().clone();
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

        let counts = self.store.borrow().counts();
        for (filter, label) in self.count_labels.borrow().iter() {
            label.set_label(&counts.of(*filter).to_string());
        }
        self.refresh_queues();

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
        *self.view_state.borrow_mut() = state;
    }

    /// Replaces one row in place when it stays visible, else rebuilds.
    fn sync_row(&self, task_id: &str) {
        let Some(item) = self.store.borrow().get(task_id).cloned() else {
            return;
        };
        let filter = self.filter.get();
        let query = self.query.borrow().clone();
        let queue = self.queue.borrow().clone();
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
    }

    fn refresh_queues(&self) {
        let mut names: Vec<String> = self
            .store
            .borrow()
            .items()
            .iter()
            .map(|item| item.queue_name.clone())
            .filter(|name| !name.is_empty())
            .collect();
        names.sort();
        names.dedup();
        self.queue_group.set_visible(!names.is_empty());

        while let Some(child) = self.queue_list.first_child() {
            self.queue_list.remove(&child);
        }
        let current = self.queue.borrow().clone();
        if names.is_empty() {
            *self.queue.borrow_mut() = None;
            return;
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

fn fill_row(widget: &gtk::Widget, item: &DownloadItem) {
    let Some(row) = widget.downcast_ref::<gtk::Box>() else {
        return;
    };
    while let Some(child) = row.first_child() {
        row.remove(&child);
    }
    row.set_orientation(gtk::Orientation::Vertical);
    row.set_spacing(6);
    row.set_margin_top(10);
    row.set_margin_bottom(10);
    row.set_margin_start(12);
    row.set_margin_end(12);

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

    let meta = gtk::Label::new(Some(&item.progress_line()));
    meta.set_xalign(0.0);
    meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
    meta.add_css_class("caption");
    meta.add_css_class("dim-label");

    row.append(&title);
    row.append(&status);
    row.append(&bar);
    row.append(&meta);
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
