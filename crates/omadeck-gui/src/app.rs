//! Main window: a life-size deck on the left, the key editor on the right.

use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use omadeck_core::ipc::{self, Event};
use omadeck_core::{Config, DeckInfo, KeyConfig, Renderer, device, paths};

use crate::{editor, media};

/// Previews are rendered larger than displayed so they stay crisp on HiDPI screens.
const PREVIEW_PX: u32 = 192;

enum Msg {
    DaemonUp,
    DaemonDown,
    Event(Event),
}

pub struct App {
    pub window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    title: adw::WindowTitle,
    banner: adw::Banner,
    grid: gtk::Grid,
    model_label: gtk::Label,
    editor_slot: adw::Bin,
    keys: RefCell<Vec<(gtk::Button, gtk::Image)>>,
    pub config: RefCell<Config>,
    pub renderer: RefCell<Renderer>,
    deck: RefCell<DeckInfo>,
    connected: Cell<bool>,
    daemon_up: Cell<bool>,
    selected: Cell<Option<u8>>,
    save_source: RefCell<Option<glib::SourceId>>,
    theme_css: gtk::CssProvider,
    theme_monitor: RefCell<Option<gio::FileMonitor>>,
}

impl App {
    pub fn build(application: &adw::Application) -> Rc<Self> {
        let first_run = !paths::config_file().exists();
        let (config, load_error) = match Config::load() {
            Ok(_) if first_run => (crate::starter::layout(), None),
            Ok(c) => (c, None),
            Err(e) => {
                // Keep the broken file around instead of silently overwriting the user's edits.
                let backup = paths::config_file().with_extension("toml.broken");
                let _ = std::fs::rename(paths::config_file(), &backup);
                (Config::default(), Some(format!("{e:#} — moved it to {}", backup.display())))
            }
        };
        if first_run && let Err(e) = config.save() {
            log::warn!("saving starter layout: {e:#}");
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

        // --- deck ------------------------------------------------------------------------
        let grid = gtk::Grid::builder().row_spacing(14).column_spacing(14).build();
        let model_label = gtk::Label::builder().halign(gtk::Align::End).css_classes(["deck-model"]).build();
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 12);
        frame.add_css_class("deck-frame");
        frame.append(&grid);
        frame.append(&model_label);
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
            grid,
            model_label,
            editor_slot,
            keys: RefCell::default(),
            config: RefCell::new(config),
            renderer: RefCell::new(renderer),
            deck: RefCell::new(deck),
            connected: Cell::new(false),
            daemon_up: Cell::new(false),
            selected: Cell::new(None),
            save_source: RefCell::default(),
            theme_css,
            theme_monitor: RefCell::default(),
        });

        settings_btn.set_popover(Some(&app.settings_popover()));
        let weak = Rc::downgrade(&app);
        app.banner.connect_button_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.start_daemon();
            }
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
        app.rebuild_grid();
        // OMADECK_SELECT=<n> opens with a key selected (handy for screenshots and scripting).
        app.select(std::env::var("OMADECK_SELECT").ok().and_then(|v| v.parse().ok()));
        app.update_title();
        app.listen();
        app.window.present();
        if let Some(e) = load_error {
            app.toast(&format!("Couldn't read your config: {e}"));
        }
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
                app.config.borrow_mut().swap_keys(from as u8, i);
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
        let keys = self.keys.borrow();
        let Some((button, image)) = keys.get(i as usize) else { return };
        let cfg = self.config.borrow();
        let key = cfg.key(i);
        let face = self.renderer.borrow_mut().render(key, &cfg.style, PREVIEW_PX, false);
        image.set_paintable(Some(&media::texture(face)));
        let tip = match key.filter(|k| !k.is_blank()) {
            Some(k) if !k.label.is_empty() => format!("{} — {}", k.label, k.action.summary()),
            Some(k) => k.action.summary(),
            None => format!("Key {} — empty", i + 1),
        };
        button.set_tooltip_text(Some(&tip));
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

    /// Mutate a key, refresh its preview and queue a save.
    pub fn edit(self: &Rc<Self>, i: u8, f: impl FnOnce(&mut KeyConfig)) {
        f(self.config.borrow_mut().key_mut(i));
        self.refresh_key(i);
        self.schedule_save();
    }

    pub fn clear_key(self: &Rc<Self>, i: u8) {
        self.config.borrow_mut().clear_key(i);
        self.refresh_key(i);
        self.schedule_save();
        self.rebuild_editor();
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

    fn flush_save(&self) {
        if let Some(id) = self.save_source.borrow_mut().take() {
            id.remove();
            self.save_now();
        }
    }

    fn save_now(&self) {
        if let Err(e) = self.config.borrow().save() {
            self.toast(&format!("Couldn't save: {e:#}"));
        }
    }

    pub fn toast(&self, msg: &str) {
        let t = adw::Toast::new(msg);
        t.set_timeout(4);
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

        let folder =
            adw::ButtonRow::builder().title("Open config folder").end_icon_name("folder-open-symbolic").build();
        folder.connect_activated(|_| {
            let _ = std::fs::create_dir_all(paths::config_dir());
            let uri = gio::File::for_path(paths::config_dir()).uri();
            let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
        });
        list.append(&folder);

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
        gtk::Popover::builder().child(&content).build()
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
                    if let Ok(events) = ipc::subscribe() {
                        if tx.send_blocking(Msg::DaemonUp).is_err() {
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
            Msg::DaemonUp => {
                self.daemon_up.set(true);
                self.banner.set_revealed(false);
            }
            Msg::DaemonDown => {
                self.daemon_up.set(false);
                self.connected.set(false);
                self.banner.set_revealed(true);
            }
            Msg::Event(Event::Connected { deck }) => {
                self.connected.set(true);
                let relayout = {
                    let cur = self.deck.borrow();
                    cur.rows != deck.rows || cur.cols != deck.cols || cur.model != deck.model
                };
                *self.deck.borrow_mut() = deck;
                if relayout {
                    self.rebuild_grid();
                }
            }
            Msg::Event(Event::Disconnected) => self.connected.set(false),
            Msg::Event(Event::KeyDown { index }) => {
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
        } else {
            "Waiting for a Stream Deck…".to_string()
        };
        self.title.set_subtitle(&sub);
    }
}
