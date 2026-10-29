//! Main window: a life-size deck on the left, the key editor on the right.

use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use omadeck_core::config::{self, RemovedPage};
use omadeck_core::ipc::{self, Command as DaemonCommand, Event};
use omadeck_core::paths::FileStamp;
use omadeck_core::{Action, Config, DeckInfo, HOME, KeyConfig, Keys, Page, Renderer, device, paths};

use crate::{bar, editor, media};

/// Previews are rendered larger than displayed so they stay crisp on HiDPI screens.
const PREVIEW_PX: u32 = 192;

/// The key a new page starts with, so a page opened from a key works like a folder.
fn back_key() -> KeyConfig {
    KeyConfig { label: "Back".into(), glyph: Some("\u{f004d}".into()), action: Action::Back, ..KeyConfig::new(0) }
}

/// A key on a particular page. Work that finishes later (fetching an icon, a picker)
/// holds one of these, so the result lands on the key it was started for even if the
/// window has moved to another page since.
#[derive(Clone)]
pub struct KeyRef {
    page: Option<String>,
    index: u8,
}

enum Msg {
    DaemonUp(ipc::Sender),
    DaemonDown,
    Event(Event),
}

pub struct App {
    pub window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    title: adw::WindowTitle,
    banner: adw::Banner,
    config_banner: adw::Banner,
    body: gtk::Box,
    settings_btn: gtk::MenuButton,
    grid: gtk::Grid,
    model_label: gtk::Label,
    page_list: gtk::StringList,
    page_dropdown: gtk::DropDown,
    page_actions: gtk::Box,
    /// Warns that the page being edited has no way back on the deck.
    dead_end: gtk::Box,
    /// Set while the page dropdown is updated from code, so it doesn't count as a choice.
    syncing_pages: Cell<bool>,
    /// The page being edited: `None` is home.
    page: RefCell<Option<String>>,
    daemon: RefCell<Option<ipc::Sender>>,
    editor_slot: adw::Bin,
    keys: RefCell<Vec<(gtk::Button, gtk::Image)>>,
    pub config: RefCell<Config>,
    pub renderer: RefCell<Renderer>,
    deck: RefCell<DeckInfo>,
    connected: Cell<bool>,
    /// A deck the daemon found but can't open (usually the udev rule is missing).
    cant_open: RefCell<Option<String>>,
    daemon_up: Cell<bool>,
    selected: Cell<Option<u8>>,
    save_source: RefCell<Option<glib::SourceId>>,
    theme_css: gtk::CssProvider,
    theme_monitor: RefCell<Option<gio::FileMonitor>>,
    /// config.toml as this window last read or wrote it (`None`: no file). Anything else
    /// on disk was written by someone else, and is never overwritten.
    disk: RefCell<Option<String>>,
    stamp: Cell<Option<FileStamp>>,
    /// Why config.toml couldn't be parsed. Saving is off until it's fixed.
    locked: RefCell<Option<String>>,
}

impl App {
    pub fn build(application: &adw::Application) -> Rc<Self> {
        // A copy of the config as it was when the app opened, in case an edit goes wrong.
        if let Err(e) = config::backup() {
            log::warn!("backing up config.toml: {e:#}");
        }
        let file = paths::config_file();
        let stamp = FileStamp::of(&file);
        let (config, mut disk, locked) = match std::fs::read_to_string(&file) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (crate::starter::layout(), None, None),
            // An unreadable or broken file is left exactly as it is; the window opens read-only.
            Err(e) => (Config::default(), None, Some(format!("can't read it: {e}"))),
            Ok(src) => match Config::parse(&src) {
                Ok(c) => (c, Some(src), None),
                Err(e) => (Config::default(), Some(src), Some(format!("{e:#}"))),
            },
        };
        let first_run = stamp.is_none() && locked.is_none();
        if first_run {
            match config.save() {
                Ok(body) => disk = Some(body),
                Err(e) => log::warn!("saving starter layout: {e:#}"),
            }
        }
        let renderer = Renderer::new(&config.style);
        let deck = device::detect().into_iter().next().unwrap_or_else(DeckInfo::fallback);

        let display = gdk::Display::default().expect("a display");
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let theme_css = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(&display, &theme_css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

        // --- header ----------------------------------------------------------------------
        let title = adw::WindowTitle::new("omadeck", "");
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));
        let settings_btn =
            gtk::MenuButton::builder().icon_name("open-menu-symbolic").tooltip_text("Deck settings").build();
        header.pack_end(&settings_btn);

        let banner = adw::Banner::builder()
            .title("The omadeck service isn't running, so your deck won't respond")
            .button_label("Start service")
            .revealed(false)
            .build();
        let config_banner = adw::Banner::builder().button_label("Open folder").revealed(false).build();

        // --- deck ------------------------------------------------------------------------
        let grid = gtk::Grid::builder().row_spacing(14).column_spacing(14).build();
        let model_label = gtk::Label::builder().halign(gtk::Align::End).css_classes(["deck-model"]).build();
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 12);
        frame.add_css_class("deck-frame");
        frame.append(&grid);
        frame.append(&model_label);
        let page_list = gtk::StringList::new(&[HOME]);
        let page_dropdown = gtk::DropDown::builder().model(&page_list).tooltip_text("The page you're editing").build();
        let new_page = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text("New page").build();
        let rename_page =
            gtk::Button::builder().icon_name("document-edit-symbolic").tooltip_text("Rename this page").build();
        let delete_page =
            gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text("Delete this page").build();
        let page_actions = gtk::Box::builder().css_classes(["linked"]).visible(false).build();
        page_actions.append(&rename_page);
        page_actions.append(&delete_page);
        let page_bar = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).build();
        page_bar.append(&page_dropdown);
        page_bar.append(&new_page);
        page_bar.append(&page_actions);

        let hint = gtk::Label::builder()
            .label("Click a key to edit it · drag keys to rearrange · press a key on your deck to jump to it")
            .css_classes(["deck-hint", "caption"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        let deck_area = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .hexpand(true)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(24)
            .margin_end(24)
            .build();
        deck_area.append(&page_bar);
        let dead_end = gtk::Box::builder().spacing(8).halign(gtk::Align::Center).visible(false).build();
        dead_end.append(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
        dead_end.append(
            &gtk::Label::builder()
                .label("No Back key: the deck gets stuck on this page")
                .wrap(true)
                .css_classes(["warning"])
                .build(),
        );
        let add_back = gtk::Button::builder().label("Add Back key").build();
        dead_end.append(&add_back);
        deck_area.append(&dead_end);
        deck_area.append(&frame);
        deck_area.append(&hint);

        let editor_slot = adw::Bin::new();
        let editor_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(430)
            .child(&editor_slot)
            .build();

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let separator = gtk::Separator::new(gtk::Orientation::Vertical);
        body.append(&deck_area);
        body.append(&separator);
        body.append(&editor_scroller);

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&body));
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.add_top_bar(&banner);
        view.add_top_bar(&config_banner);
        view.set_content(Some(&toasts));

        let window = adw::ApplicationWindow::builder()
            .application(application)
            .title("omadeck")
            .default_width(1180)
            .default_height(700)
            .content(&view)
            .width_request(380)
            .height_request(560)
            .build();

        // Omarchy tiles windows, so a half-screen window is common: stack the deck above
        // the editor when there isn't room for both side by side.
        let narrow =
            adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 1050sp").expect("valid condition"));
        narrow.add_setter(&body, "orientation", Some(&gtk::Orientation::Vertical.to_value()));
        narrow.add_setter(&separator, "orientation", Some(&gtk::Orientation::Horizontal.to_value()));
        narrow.add_setter(&editor_scroller, "width-request", Some(&(-1i32).to_value()));
        narrow.add_setter(&editor_scroller, "vexpand", Some(&true.to_value()));
        narrow.add_setter(&deck_area, "margin-top", Some(&12i32.to_value()));
        narrow.add_setter(&deck_area, "margin-bottom", Some(&12i32.to_value()));
        window.add_breakpoint(narrow);

        let app = Rc::new(Self {
            window,
            toasts,
            title,
            banner,
            config_banner,
            body,
            settings_btn,
            grid,
            model_label,
            page_list,
            page_dropdown,
            page_actions,
            dead_end,
            syncing_pages: Cell::new(false),
            page: RefCell::default(),
            daemon: RefCell::default(),
            editor_slot,
            keys: RefCell::default(),
            config: RefCell::new(config),
            renderer: RefCell::new(renderer),
            deck: RefCell::new(deck),
            connected: Cell::new(false),
            cant_open: RefCell::new(None),
            daemon_up: Cell::new(false),
            selected: Cell::new(None),
            save_source: RefCell::default(),
            theme_css,
            theme_monitor: RefCell::default(),
            disk: RefCell::new(disk),
            stamp: Cell::new(FileStamp::of(&file)),
            locked: RefCell::new(locked),
        });

        app.settings_btn.set_popover(Some(&app.settings_popover()));
        let weak = Rc::downgrade(&app);
        app.page_dropdown.connect_selected_notify(move |dd| {
            let Some(app) = weak.upgrade() else { return };
            if app.syncing_pages.get() {
                return;
            }
            let page = match dd.selected() {
                0 | gtk::INVALID_LIST_POSITION => None,
                n => app.config.borrow().pages.get(n as usize - 1).map(|p| p.name.clone()),
            };
            app.show_page(page, true);
        });
        let weak = Rc::downgrade(&app);
        add_back.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.add_back_key();
            }
        });
        let weak = Rc::downgrade(&app);
        new_page.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.ask_new_page();
            }
        });
        let weak = Rc::downgrade(&app);
        rename_page.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.ask_rename_page();
            }
        });
        let weak = Rc::downgrade(&app);
        delete_page.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.ask_delete_page();
            }
        });
        let weak = Rc::downgrade(&app);
        app.banner.connect_button_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.start_daemon();
            }
        });
        app.config_banner.connect_button_clicked(|_| {
            let uri = gio::File::for_path(paths::config_dir()).uri();
            let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
        });
        let weak = Rc::downgrade(&app);
        app.window.connect_close_request(move |_| {
            if let Some(app) = weak.upgrade() {
                app.flush_save();
            }
            glib::Propagation::Proceed
        });

        // The window owns the app state; callbacks only hold weak references.
        let keep = RefCell::new(Some(app.clone()));
        app.window.connect_destroy(move |_| drop(keep.take()));

        app.apply_theme_css();
        app.watch_theme();
        app.sync_page_list();
        app.rebuild_grid();
        // OMADECK_SELECT=<n> opens with a key selected (handy for screenshots and scripting).
        app.select(std::env::var("OMADECK_SELECT").ok().and_then(|v| v.parse().ok()));
        app.update_title();
        app.listen();
        app.watch_config();
        app.show_lock();
        app.window.present();
        app.setup_bar_widget();
        app
    }

    // --- deck grid ------------------------------------------------------------------------

    fn key_display_px(&self) -> i32 {
        let d = self.deck.borrow();
        match d.cols {
            0..=5 => 96,
            6..=8 => 80,
            _ => 64,
        }
    }

    fn rebuild_grid(self: &Rc<Self>) {
        while let Some(child) = self.grid.first_child() {
            self.grid.remove(&child);
        }
        self.keys.borrow_mut().clear();
        let deck = self.deck.borrow().clone();
        let px = self.key_display_px();
        for i in 0..deck.key_count() {
            let image = gtk::Image::builder().pixel_size(px).build();
            let button =
                gtk::Button::builder().child(&image).css_classes(["deck-key"]).overflow(gtk::Overflow::Hidden).build();
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.select(Some(i));
                }
            });
            self.add_dnd(&button, &image, i);
            self.grid.attach(&button, (i % deck.cols) as i32, (i / deck.cols) as i32, 1, 1);
            self.keys.borrow_mut().push((button, image));
        }
        self.model_label.set_label(&deck.model.to_uppercase());
        if self.selected.get().is_some_and(|s| s >= deck.key_count()) {
            self.select(None);
        }
        self.refresh_all();
    }

    fn add_dnd(self: &Rc<Self>, button: &gtk::Button, image: &gtk::Image, i: u8) {
        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::MOVE);
        let img = image.clone();
        source.connect_prepare(move |src, _, _| {
            if let Some(p) = img.paintable() {
                src.set_icon(Some(&p), 48, 48);
            }
            Some(gdk::ContentProvider::for_value(&(i as u32).to_value()))
        });
        button.add_controller(source);

        let target = gtk::DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
        let b = button.clone();
        target.connect_enter(move |_, _, _| {
            b.add_css_class("drop-target");
            gdk::DragAction::MOVE
        });
        let b = button.clone();
        target.connect_leave(move |_| b.remove_css_class("drop-target"));
        let weak = Rc::downgrade(self);
        let b = button.clone();
        target.connect_drop(move |_, value, _, _| {
            b.remove_css_class("drop-target");
            let (Some(app), Ok(from)) = (weak.upgrade(), value.get::<u32>()) else { return false };
            if from as u8 != i {
                app.with_page(|keys| keys.swap(from as u8, i));
                app.refresh_key(from as u8);
                app.refresh_key(i);
                app.select(Some(i));
                app.schedule_save();
            }
            true
        });
        button.add_controller(target);
    }

    pub fn refresh_key(&self, i: u8) {
        self.update_dead_end();
        let keys = self.keys.borrow();
        let Some((button, image)) = keys.get(i as usize) else { return };
        let cfg = self.config.borrow();
        let key = cfg.key(self.page.borrow().as_deref(), i);
        let face = self.renderer.borrow_mut().render(key, &cfg.style, PREVIEW_PX, false);
        image.set_paintable(Some(&media::texture(face)));
        let tip = match key.filter(|k| !k.is_blank()) {
            Some(k) if !k.label.is_empty() => format!("{} — {}", k.label, k.action.summary()),
            Some(k) => k.action.summary(),
            None => format!("Key {} — empty", i + 1),
        };
        button.set_tooltip_text(Some(&tip));
    }

    fn update_dead_end(&self) {
        let ok = self.config.borrow().has_way_out(self.page.borrow().as_deref());
        self.dead_end.set_visible(!ok);
    }

    /// Put a Back key in the first free slot of the page being edited.
    fn add_back_key(self: &Rc<Self>) {
        let n = self.deck.borrow().key_count();
        let Some(i) = (0..n).find(|&i| self.key(i).is_none_or(|k| k.is_blank())) else {
            self.toast("This page is full. Turn one of its keys into a Back key.");
            return;
        };
        self.with_page(|keys| *keys.get_mut(i) = KeyConfig { index: i, ..back_key() });
        self.refresh_key(i);
        self.schedule_save();
        self.select(Some(i));
    }

    pub fn refresh_all(&self) {
        let n = self.keys.borrow().len() as u8;
        for i in 0..n {
            self.refresh_key(i);
        }
    }

    pub fn select(self: &Rc<Self>, i: Option<u8>) {
        self.selected.set(i);
        for (n, (button, _)) in self.keys.borrow().iter().enumerate() {
            if Some(n as u8) == i {
                button.add_css_class("selected");
            } else {
                button.remove_css_class("selected");
            }
        }
        let widget: gtk::Widget = match i {
            Some(i) => editor::build(self, i),
            None => adw::StatusPage::builder()
                .icon_name("input-keyboard-symbolic")
                .title("Pick a key")
                .description("Click a key on the deck, or press one on your Stream Deck.")
                .vexpand(true)
                .build()
                .upcast(),
        };
        self.editor_slot.set_child(Some(&widget));
    }

    /// Rebuild the editor for the selected key on the next idle (safe from signal handlers
    /// of widgets that are about to be replaced).
    pub fn rebuild_editor(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(app) = weak.upgrade() {
                app.select(app.selected.get());
            }
        });
    }

    pub fn deck_info(&self) -> DeckInfo {
        self.deck.borrow().clone()
    }

    // --- editing & saving -----------------------------------------------------------------

    /// A key on the page being edited.
    pub fn key(&self, i: u8) -> Option<KeyConfig> {
        self.config.borrow().key(self.page.borrow().as_deref(), i).cloned()
    }

    /// Change the keys of the page being edited.
    fn with_page<R>(&self, f: impl FnOnce(&mut Keys) -> R) -> R {
        let page = self.page.borrow();
        let mut cfg = self.config.borrow_mut();
        let cfg = &mut *cfg;
        f(match cfg.page_mut(page.as_deref()) {
            Some(keys) => keys,
            // The page vanished under us; edit home rather than lose the change.
            None => &mut cfg.keys,
        })
    }

    /// Mutate a key, refresh its preview and queue a save.
    pub fn edit(self: &Rc<Self>, i: u8, f: impl FnOnce(&mut KeyConfig)) {
        self.with_page(|keys| f(keys.get_mut(i)));
        self.refresh_key(i);
        self.schedule_save();
    }

    /// The key `i` of the page being edited, to edit later with [`App::edit_ref`].
    pub fn key_ref(&self, i: u8) -> KeyRef {
        KeyRef { page: self.page.borrow().clone(), index: i }
    }

    /// Like [`App::edit`], for a key that may no longer be on screen. Returns false (and
    /// says so) when its page has gone.
    pub fn edit_ref(self: &Rc<Self>, at: &KeyRef, f: impl FnOnce(&mut KeyConfig)) -> bool {
        if self.locked.borrow().is_some() {
            return false;
        }
        match self.config.borrow_mut().page_mut(at.page.as_deref()) {
            Some(keys) => f(keys.get_mut(at.index)),
            None => {
                self.toast("That key's page was deleted, so the change was dropped");
                return false;
            }
        }
        if *self.page.borrow() == at.page {
            self.refresh_key(at.index);
        }
        self.schedule_save();
        true
    }

    pub fn clear_key(self: &Rc<Self>, i: u8) {
        let at = self.key_ref(i);
        let old = self.key(i).filter(|k| !k.is_blank());
        self.with_page(|keys| keys.clear(i));
        self.refresh_key(i);
        self.schedule_save();
        self.rebuild_editor();
        let Some(old) = old else { return };
        self.toast_undo(&format!("Cleared key {}", i + 1), move |app| {
            if app.config.borrow().key(at.page.as_deref(), at.index).is_some_and(|k| !k.is_blank()) {
                app.toast("That key has been used again, so it wasn't restored");
                return;
            }
            let old = old.clone();
            if app.edit_ref(&at, move |k| *k = old) {
                app.rebuild_editor();
            }
        });
    }

    pub fn schedule_save(self: &Rc<Self>) {
        if let Some(id) = self.save_source.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if let Some(app) = weak.upgrade() {
                app.save_source.borrow_mut().take();
                app.save_now();
            }
        });
        *self.save_source.borrow_mut() = Some(id);
    }

    fn flush_save(self: &Rc<Self>) {
        if let Some(id) = self.save_source.borrow_mut().take() {
            id.remove();
            self.save_now();
        }
    }

    fn save_now(self: &Rc<Self>) {
        if self.locked.borrow().is_some() {
            return;
        }
        // Someone edited config.toml since we last looked: theirs wins.
        let on_disk = std::fs::read_to_string(paths::config_file()).ok();
        let changed = on_disk != *self.disk.borrow();
        if changed {
            self.reload_from_disk(true);
            return;
        }
        let saved = self.config.borrow().save();
        match saved {
            Ok(body) => {
                *self.disk.borrow_mut() = Some(body);
                self.stamp.set(FileStamp::of(&paths::config_file()));
            }
            Err(e) => self.toast(&format!("Couldn't save: {e:#}")),
        }
    }

    // --- external edits -------------------------------------------------------------------

    /// Pick up edits made to config.toml outside this window (a text editor, a dotfile
    /// sync, a restored backup). Polling the file's stamp also covers symlinked configs.
    fn watch_config(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
            let font = app.renderer.borrow_mut().font_changed();
            if font {
                app.refresh_all();
                app.rebuild_editor();
            }
            let stamp = FileStamp::of(&paths::config_file());
            if stamp != app.stamp.get() {
                app.stamp.set(stamp);
                let on_disk = std::fs::read_to_string(paths::config_file()).ok();
                // A deleted file is left alone; the next edit here writes it back.
                let changed = on_disk.is_some() && on_disk != *app.disk.borrow();
                if changed {
                    let pending = app.save_source.borrow().is_some();
                    app.reload_from_disk(pending);
                }
            }
            glib::ControlFlow::Continue
        });
    }

    /// Replace the in-memory config with what's on disk. `dropping` says an unsaved edit
    /// made here is being discarded in favour of the file.
    fn reload_from_disk(self: &Rc<Self>, dropping: bool) {
        if let Some(id) = self.save_source.borrow_mut().take() {
            id.remove();
        }
        let file = paths::config_file();
        self.stamp.set(FileStamp::of(&file));
        let Ok(src) = std::fs::read_to_string(&file) else { return };
        match Config::parse(&src) {
            Ok(cfg) => {
                {
                    let mut r = self.renderer.borrow_mut();
                    r.update_style(&cfg.style);
                    r.clear_cache();
                }
                *self.config.borrow_mut() = cfg;
                *self.locked.borrow_mut() = None;
                let page = self.page.borrow().clone().filter(|p| self.config.borrow().has_page(p));
                *self.page.borrow_mut() = page;
                self.settings_btn.set_popover(Some(&self.settings_popover()));
                self.sync_page_list();
                self.refresh_all();
                self.rebuild_editor();
                self.toast(if dropping {
                    "config.toml was changed outside omadeck, so your last change here was discarded"
                } else {
                    "Loaded changes made to config.toml outside omadeck"
                });
            }
            Err(e) => *self.locked.borrow_mut() = Some(format!("{e:#}")),
        }
        *self.disk.borrow_mut() = Some(src);
        self.show_lock();
    }

    /// While config.toml can't be parsed, say why and stop editing, so nothing here can
    /// overwrite the user's file.
    fn show_lock(&self) {
        let locked = self.locked.borrow();
        if let Some(e) = locked.as_deref() {
            let first = e.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(e);
            self.config_banner.set_title(&glib::markup_escape_text(&format!(
                "config.toml has a mistake ({first}). Fix it in a text editor; omadeck won't change it until then."
            )));
            log::warn!("config.toml can't be parsed, editing is off: {e}");
        }
        self.config_banner.set_revealed(locked.is_some());
        self.body.set_sensitive(locked.is_none());
        self.settings_btn.set_sensitive(locked.is_none());
    }

    // --- pages ----------------------------------------------------------------------------

    pub fn current_page(&self) -> Option<String> {
        self.page.borrow().clone()
    }

    /// Edit another page. With `tell_daemon` the deck shows it too, so edits land live.
    pub fn show_page(self: &Rc<Self>, page: Option<String>, tell_daemon: bool) {
        let page = page.filter(|p| self.config.borrow().has_page(p));
        let changed = *self.page.borrow() != page;
        *self.page.borrow_mut() = page.clone();
        if changed {
            self.sync_page_list();
            self.select(None);
            self.refresh_all();
        }
        if tell_daemon {
            self.send(DaemonCommand::ShowPage { page });
        }
    }

    /// Mirror the config's pages in the page switcher.
    fn sync_page_list(&self) {
        self.syncing_pages.set(true);
        let cfg = self.config.borrow();
        let names = cfg.page_names();
        let mut items: Vec<&str> = vec![HOME];
        items.extend(names.iter().map(String::as_str));
        self.page_list.splice(0, self.page_list.n_items(), &items);
        let page = self.page.borrow();
        let pos = page.as_deref().and_then(|p| names.iter().position(|n| n == p)).map(|n| n + 1).unwrap_or(0);
        self.page_dropdown.set_selected(pos as u32);
        self.page_actions.set_visible(page.is_some());
        self.syncing_pages.set(false);
    }

    /// Add a page that starts with a Back key, so it works as a folder. Returns its name.
    pub fn add_page(self: &Rc<Self>, name: &str) -> String {
        let mut cfg = self.config.borrow_mut();
        let name = cfg.unique_page_name(name);
        cfg.pages.push(Page { keys: Keys(vec![back_key()]), ..Page::new(name.clone()) });
        drop(cfg);
        self.sync_page_list();
        self.schedule_save();
        name
    }

    /// A dialog asking for a page name; `done` gets the accepted name.
    fn page_name_dialog(
        self: &Rc<Self>,
        heading: &str,
        accept: &str,
        initial: &str,
        current: Option<String>,
        done: impl Fn(&Rc<Self>, String) + 'static,
    ) {
        let dialog = adw::AlertDialog::new(Some(heading), None);
        dialog.add_responses(&[("cancel", "Cancel"), ("ok", accept)]);
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let entry = gtk::Entry::builder().text(initial).activates_default(true).build();
        dialog.set_extra_child(Some(&entry));
        let weak = Rc::downgrade(self);
        let d = dialog.clone();
        let check = move |e: &gtk::Entry| {
            let Some(app) = weak.upgrade() else { return };
            let name = e.text();
            let problem = if current.as_deref() == Some(name.trim()) {
                None
            } else {
                app.config.borrow().page_name_problem(&name)
            };
            d.set_body(problem.unwrap_or(""));
            d.set_response_enabled("ok", problem.is_none());
        };
        check(&entry);
        entry.connect_changed(check);
        let weak = Rc::downgrade(self);
        let e = entry.clone();
        dialog.connect_response(Some("ok"), move |_, _| {
            if let Some(app) = weak.upgrade() {
                done(&app, e.text().trim().to_string());
            }
        });
        dialog.present(Some(&self.window));
        entry.grab_focus();
    }

    fn ask_new_page(self: &Rc<Self>) {
        let suggestion = self.config.borrow().unique_page_name("Page");
        self.page_name_dialog("New page", "Create", &suggestion, None, |app, name| {
            let name = app.add_page(&name);
            app.show_page(Some(name), true);
            app.toast("New page — give a key the “Open page” action to reach it from your deck");
        });
    }

    fn ask_rename_page(self: &Rc<Self>) {
        let Some(old) = self.current_page() else { return };
        self.page_name_dialog("Rename page", "Rename", &old.clone(), Some(old.clone()), move |app, new| {
            if new == old {
                return;
            }
            app.config.borrow_mut().rename_page(&old, &new);
            *app.page.borrow_mut() = Some(new.clone());
            app.sync_page_list();
            app.refresh_all();
            app.schedule_save();
            app.rebuild_editor();
            app.send(DaemonCommand::ShowPage { page: Some(new) });
        });
    }

    fn ask_delete_page(self: &Rc<Self>) {
        let Some(name) = self.current_page() else { return };
        let links = self.config.borrow().links_to(&name);
        let body = match links {
            0 => "Its keys will be deleted too.".to_string(),
            1 => "Its keys will be deleted too, and the key that opens it will stop doing anything.".to_string(),
            n => format!("Its keys will be deleted too, and the {n} keys that open it will stop doing anything."),
        };
        let dialog = adw::AlertDialog::new(Some(&format!("Delete “{name}”?")), Some(&body));
        dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(Some("delete"), move |_, _| {
            if let Some(app) = weak.upgrade() {
                app.delete_page(&name);
            }
        });
        dialog.present(Some(&self.window));
    }

    fn delete_page(self: &Rc<Self>, name: &str) {
        // Back up first: undo only lasts as long as the toast.
        self.flush_save();
        if let Err(e) = config::backup() {
            log::warn!("backing up config.toml: {e:#}");
        }
        let removed = self.config.borrow_mut().remove_page(name);
        self.schedule_save();
        self.show_page(None, true);
        self.sync_page_list();
        let Some(removed) = removed else { return };
        let slot = RefCell::new(Some(removed));
        self.toast_undo(&format!("Deleted “{name}”"), move |app| {
            if let Some(removed) = slot.take() {
                app.undo_remove_page(removed);
            }
        });
    }

    fn undo_remove_page(self: &Rc<Self>, removed: RemovedPage) {
        if self.locked.borrow().is_some() {
            return;
        }
        let name = removed.page.name.clone();
        let restored = self.config.borrow_mut().restore_page(removed);
        if !restored {
            self.toast("There's a page with that name again, so it wasn't restored");
            return;
        }
        self.schedule_save();
        self.sync_page_list();
        self.show_page(Some(name), true);
    }

    fn send(&self, cmd: DaemonCommand) {
        if let Some(sender) = self.daemon.borrow_mut().as_mut()
            && let Err(e) = sender.send(&cmd)
        {
            log::warn!("telling omadeckd: {e}");
        }
    }

    pub fn toast(&self, msg: &str) {
        let t = adw::Toast::new(&glib::markup_escape_text(msg));
        t.set_timeout(4);
        self.toasts.add_toast(t);
    }

    /// A toast with an Undo button, for edits that throw something away.
    /// An action from a newer omadeck on key `i` was just replaced; `restore` puts it back.
    pub fn offer_undo_replaced(self: &Rc<Self>, i: u8, restore: impl Fn(&mut KeyConfig) + 'static) {
        let at = self.key_ref(i);
        self.toast_undo("Replaced an action from a newer omadeck", move |app| {
            if app.edit_ref(&at, &restore) {
                app.rebuild_editor();
            }
        });
    }

    fn toast_undo(self: &Rc<Self>, msg: &str, undo: impl Fn(&Rc<Self>) + 'static) {
        let t = adw::Toast::builder().title(glib::markup_escape_text(msg)).button_label("Undo").timeout(10).build();
        let weak = Rc::downgrade(self);
        t.connect_button_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                undo(&app);
            }
        });
        self.toasts.add_toast(t);
    }

    // --- theme ----------------------------------------------------------------------------

    fn apply_theme_css(&self) {
        let r = self.renderer.borrow();
        let t = &r.theme;
        self.theme_css.load_from_string(&format!("@define-color deck_accent {};", t.accent.hex()));
    }

    /// Follow `omarchy theme set` live, like the daemon does.
    fn watch_theme(self: &Rc<Self>) {
        let file = gio::File::for_path(paths::omarchy_current_dir().join("theme.name"));
        let Ok(monitor) = file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) else { return };
        let weak = Rc::downgrade(self);
        monitor.connect_changed(move |_, _, _, ev| {
            if !matches!(ev, gio::FileMonitorEvent::ChangesDoneHint | gio::FileMonitorEvent::Created) {
                return;
            }
            if let Some(app) = weak.upgrade() {
                app.renderer.borrow_mut().reload_theme();
                app.apply_theme_css();
                app.refresh_all();
                app.rebuild_editor();
            }
        });
        *self.theme_monitor.borrow_mut() = Some(monitor);
    }

    // --- settings popover -----------------------------------------------------------------

    fn settings_popover(self: &Rc<Self>) -> gtk::Popover {
        let cfg = self.config.borrow();
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();

        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 5.0, 100.0, 5.0);
        scale.set_value(cfg.brightness as f64);
        scale.set_hexpand(true);
        scale.set_width_request(170);
        let weak = Rc::downgrade(self);
        scale.connect_value_changed(move |s| {
            if let Some(app) = weak.upgrade() {
                app.config.borrow_mut().brightness = s.value().round() as u8;
                app.schedule_save();
            }
        });
        let bright = adw::ActionRow::builder().title("Brightness").build();
        bright.add_suffix(&scale);
        list.append(&bright);

        let follow = adw::SwitchRow::builder()
            .title("Match Omarchy theme")
            .subtitle("Key colours follow your current theme")
            .active(cfg.style.follow_theme)
            .build();
        let weak = Rc::downgrade(self);
        follow.connect_active_notify(move |row| {
            if let Some(app) = weak.upgrade() {
                app.config.borrow_mut().style.follow_theme = row.is_active();
                app.refresh_all();
                app.schedule_save();
                app.rebuild_editor();
            }
        });
        list.append(&follow);

        let mut bar_row = None;
        if bar::available() {
            let row = adw::SwitchRow::builder()
                .title("Show in Omarchy bar")
                .subtitle("Your deck's status on the bar; click it to open omadeck")
                .sensitive(false)
                .build();
            // Set while the switch is moved by code rather than by the user.
            let quiet = Rc::new(Cell::new(true));
            let (weak, q) = (Rc::downgrade(self), quiet.clone());
            row.connect_active_notify(move |r| {
                if q.get() {
                    return;
                }
                let Some(app) = weak.upgrade() else { return };
                let (r2, q2, on) = (r.clone(), q.clone(), r.is_active());
                app.set_bar_widget(
                    on,
                    Some(Box::new(move || {
                        q2.set(true);
                        r2.set_active(!on);
                        q2.set(false);
                    })),
                );
            });
            list.append(&row);
            bar_row = Some((row, quiet));
        }

        let folder =
            adw::ButtonRow::builder().title("Open config folder").end_icon_name("folder-open-symbolic").build();
        folder.connect_activated(|_| {
            let _ = std::fs::create_dir_all(paths::config_dir());
            let uri = gio::File::for_path(paths::config_dir()).uri();
            let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
        });
        list.append(&folder);

        let backups =
            adw::ButtonRow::builder().title("Open config backups").end_icon_name("folder-open-symbolic").build();
        backups.set_tooltip_text(Some("A copy is kept each time omadeck opens and before a page is deleted"));
        backups.connect_activated(|_| {
            let _ = std::fs::create_dir_all(paths::backups_dir());
            let uri = gio::File::for_path(paths::backups_dir()).uri();
            let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
        });
        list.append(&backups);

        let about = adw::ButtonRow::builder().title("About omadeck").build();
        let weak: Weak<Self> = Rc::downgrade(self);
        about.connect_activated(move |_| {
            if let Some(app) = weak.upgrade() {
                app.show_about();
            }
        });
        list.append(&about);

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .width_request(380)
            .build();
        content.append(&list);
        let popover = gtk::Popover::builder().child(&content).build();
        if let Some((row, quiet)) = bar_row {
            // The widget can be turned off in Omarchy too, so look each time the menu opens.
            popover.connect_show(move |_| {
                let (row, quiet) = (row.clone(), quiet.clone());
                glib::spawn_future_local(async move {
                    let on = gio::spawn_blocking(|| bar::enabled(&bar::run_command)).await;
                    quiet.set(true);
                    if let Ok(Ok(on)) = on {
                        row.set_active(on);
                    }
                    row.set_sensitive(true);
                    quiet.set(false);
                });
            });
        }
        popover
    }

    // --- Omarchy bar widget ---------------------------------------------------------------

    /// Keep an installed widget's files current after an upgrade (without changing whether it's
    /// on), and on the first run on Omarchy, ask once whether to add it.
    fn setup_bar_widget(self: &Rc<Self>) {
        if !bar::available() {
            return;
        }
        if bar::plugin_dir().is_dir()
            && let Err(e) = bar::install(&bar::plugin_dir(), &bar::daemon_path())
        {
            log::warn!("updating the bar widget: {e:#}");
        }
        let marker = bar::asked_marker();
        if !bar::should_ask(true, &marker) {
            return;
        }
        let dialog = adw::AlertDialog::builder()
            .heading("Add omadeck to your Omarchy bar?")
            .body(
                "A small icon shows whether your Stream Deck is connected and which page it's on. \
                 Click it to open omadeck. You can turn it off any time in omadeck's settings.",
            )
            .build();
        dialog.add_responses(&[("later", "Not now"), ("add", "Add to bar")]);
        dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("add"));
        dialog.set_close_response("later");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            bar::remember_asked(&marker);
            if response == "add"
                && let Some(app) = weak.upgrade()
            {
                app.set_bar_widget(true, None);
            }
        });
        dialog.present(Some(&self.window));
    }

    /// Turn the bar widget on or off without blocking the window. `undo` puts the settings
    /// switch back if it didn't work.
    fn set_bar_widget(self: &Rc<Self>, on: bool, undo: Option<Box<dyn Fn()>>) {
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let res = gio::spawn_blocking(move || {
                if on {
                    bar::install(&bar::plugin_dir(), &bar::daemon_path())?;
                    bar::enable(&bar::run_command)
                } else {
                    bar::disable(&bar::run_command)
                }
            })
            .await;
            let Some(app) = weak.upgrade() else { return };
            match res {
                Ok(Ok(())) => app.toast(if on { "Added omadeck to your bar" } else { "Removed omadeck from your bar" }),
                Ok(Err(e)) => {
                    app.toast(&format!("Couldn't change the bar: {e:#}"));
                    if let Some(undo) = undo {
                        undo();
                    }
                }
                Err(_) => {
                    if let Some(undo) = undo {
                        undo();
                    }
                }
            }
        });
    }

    fn show_about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("omadeck")
            .application_icon(crate::APP_ID)
            .developer_name("Raymond Brady & Claude")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Elgato Stream Deck for Omarchy: launch apps, run scripts, play sounds and open websites — with your theme on every key.")
            .license_type(gtk::License::MitX11)
            .website("https://github.com/thewizster/omadeck")
            .build();
        about.present(Some(&self.window));
    }

    // --- daemon ---------------------------------------------------------------------------

    fn start_daemon(self: &Rc<Self>) {
        let ok = Command::new("systemctl")
            .args(["--user", "enable", "--now", "omadeckd.service"])
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            self.toast("Started the omadeck service");
            return;
        }
        // Not installed as a service (e.g. running from a build tree): start it directly.
        let sibling = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("omadeckd")));
        let bin = sibling.filter(|p| p.exists()).map(|p| p.into_os_string()).unwrap_or_else(|| "omadeckd".into());
        match Command::new(&bin).stdin(std::process::Stdio::null()).spawn() {
            Ok(_) => self.toast("Started omadeckd (run scripts/install.sh to start it automatically at login)"),
            Err(e) => self.toast(&format!("Couldn't start omadeckd: {e}")),
        }
    }

    /// Follow the daemon's event feed on a worker thread.
    fn listen(self: &Rc<Self>) {
        let (tx, rx) = async_channel::unbounded::<Msg>();
        std::thread::Builder::new()
            .name("ipc".into())
            .spawn(move || {
                loop {
                    if let Ok((sender, events)) = ipc::connect() {
                        if tx.send_blocking(Msg::DaemonUp(sender)).is_err() {
                            return;
                        }
                        for e in events {
                            if tx.send_blocking(Msg::Event(e)).is_err() {
                                return;
                            }
                        }
                    }
                    if tx.send_blocking(Msg::DaemonDown).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            })
            .expect("spawn ipc thread");
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                let Some(app) = weak.upgrade() else { break };
                app.handle(msg);
            }
        });
    }

    fn handle(self: &Rc<Self>, msg: Msg) {
        match msg {
            Msg::DaemonUp(sender) => {
                *self.daemon.borrow_mut() = Some(sender);
                self.daemon_up.set(true);
                self.banner.set_revealed(false);
            }
            Msg::DaemonDown => {
                self.daemon.borrow_mut().take();
                self.daemon_up.set(false);
                self.connected.set(false);
                self.banner.set_revealed(true);
            }
            Msg::Event(Event::Connected { deck }) => {
                self.connected.set(true);
                self.cant_open.borrow_mut().take();
                let relayout = {
                    let cur = self.deck.borrow();
                    cur.rows != deck.rows || cur.cols != deck.cols || cur.model != deck.model
                };
                *self.deck.borrow_mut() = deck;
                if relayout {
                    self.rebuild_grid();
                }
            }
            Msg::Event(Event::Disconnected) => {
                self.connected.set(false);
                self.cant_open.borrow_mut().take();
            }
            Msg::Event(Event::CantOpen { model }) => {
                self.connected.set(false);
                *self.cant_open.borrow_mut() = Some(model);
            }
            // Follow the deck: pressing a page key there shows that page here too.
            Msg::Event(Event::PageChanged { page }) => self.show_page(page, false),
            Msg::Event(Event::KeyDown { index, page }) => {
                self.show_page(page, false);
                if let Some((b, _)) = self.keys.borrow().get(index as usize) {
                    b.add_css_class("pressed");
                }
                if self.selected.get() != Some(index) {
                    self.select(Some(index));
                }
            }
            Msg::Event(Event::KeyUp { index }) => {
                if let Some((b, _)) = self.keys.borrow().get(index as usize) {
                    b.remove_css_class("pressed");
                }
            }
            Msg::Event(Event::ActionFailed { index, message }) => {
                self.toast(&format!("Key {}: {message}", index + 1));
            }
        }
        self.update_title();
    }

    fn update_title(&self) {
        let sub = if !self.daemon_up.get() {
            "Service not running".to_string()
        } else if self.connected.get() {
            format!("{} · connected", self.deck.borrow().model)
        } else if let Some(model) = self.cant_open.borrow().as_deref() {
            format!("Can't open the {model}: replug it, or see Troubleshooting in the README")
        } else {
            "Waiting for a Stream Deck…".to_string()
        };
        self.title.set_subtitle(&sub);
    }
}
