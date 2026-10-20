//! The right-hand panel: what a key does and how it looks.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use omadeck_core::{Action, HOME, KeyConfig, Rgb, action, paths};

use crate::app::{App, KeyRef};
use crate::{apps, media};

pub fn build(app: &Rc<App>, i: u8) -> gtk::Widget {
    let key = app.key(i).unwrap_or_else(|| KeyConfig::new(i));
    let deck = app.deck_info();
    let page = adw::PreferencesPage::new();
    let page_name = app.current_page();

    // --- action ---------------------------------------------------------------------------
    let group = adw::PreferencesGroup::builder()
        .title(format!("Key {}", i + 1))
        .description(format!(
            "{} · Row {} · Column {}",
            page_name.as_deref().unwrap_or(HOME),
            i / deck.cols + 1,
            i % deck.cols + 1
        ))
        .build();

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let test = test_button(app, i, Slot::Press, &key.action);
    let clear = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Clear this key")
        .valign(gtk::Align::Center)
        .sensitive(!key.is_blank())
        .css_classes(["flat"])
        .build();
    let a = app.clone();
    clear.connect_clicked(move |_| a.clear_key(i));
    buttons.append(&test);
    buttons.append(&clear);
    group.set_header_suffix(Some(&buttons));

    group.add(&kind_row(app, i, Slot::Press, &key.action));
    action_rows(app, i, Slot::Press, &key.action, &group);
    page.add(&group);

    // --- release --------------------------------------------------------------------------
    let release = key.on_release.clone().unwrap_or_default();
    let held = adw::PreferencesGroup::builder()
        .title("When released")
        .description("For push-to-talk and other keys you hold down")
        .build();
    held.set_header_suffix(Some(&test_button(app, i, Slot::Release, &release)));
    held.add(&kind_row(app, i, Slot::Release, &release));
    action_rows(app, i, Slot::Release, &release, &held);
    page.add(&held);

    // --- appearance -----------------------------------------------------------------------
    let look = adw::PreferencesGroup::builder().title("Appearance").build();

    let label = adw::EntryRow::builder().title("Label").text(&key.label).build();
    let a = app.clone();
    label.connect_changed(move |e| {
        let text = e.text().to_string();
        a.edit(i, |k| k.label = text);
    });
    look.add(&label);

    let show = adw::SwitchRow::builder().title("Show label on key").active(key.show_label).build();
    let a = app.clone();
    show.connect_active_notify(move |r| {
        let on = r.is_active();
        a.edit(i, |k| k.show_label = on);
    });
    look.add(&show);

    look.add(&icon_row(app, i, &key));

    let glyph = adw::EntryRow::builder()
        .title("Symbol (Nerd Font glyph, used when there's no image)")
        .text(key.glyph.as_deref().unwrap_or(""))
        .build();
    let cheat = gtk::Button::builder()
        .icon_name("web-browser-symbolic")
        .tooltip_text("Browse Nerd Font symbols — copy one and paste it here")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    cheat.connect_clicked(|_| {
        let _ =
            gio::AppInfo::launch_default_for_uri("https://www.nerdfonts.com/cheat-sheet", gio::AppLaunchContext::NONE);
    });
    glyph.add_suffix(&cheat);
    let a = app.clone();
    glyph.connect_changed(move |e| {
        let text = e.text().trim().to_string();
        a.edit(i, |k| k.glyph = (!text.is_empty()).then_some(text));
    });
    look.add(&glyph);

    let (bg, fg) = {
        let cfg = app.config.borrow();
        let r = app.renderer.borrow();
        let bg = r.background(Some(&key), &cfg.style);
        (bg, r.label_color(Some(&key), &cfg.style, bg))
    };
    look.add(&color_row(app, i, "Background", bg, key.background.is_some(), |k, c| k.background = c));
    look.add(&color_row(app, i, "Label colour", fg, key.label_color.is_some(), |k, c| k.label_color = c));
    page.add(&look);

    page.upcast()
}

/// Which of a key's two actions an editor row changes.
#[derive(Clone, Copy, PartialEq)]
enum Slot {
    Press,
    Release,
}

impl Slot {
    fn get(self, k: &KeyConfig) -> Action {
        match self {
            Slot::Press => k.action.clone(),
            Slot::Release => k.on_release.clone().unwrap_or_default(),
        }
    }

    fn get_mut(self, k: &mut KeyConfig) -> &mut Action {
        match self {
            Slot::Press => &mut k.action,
            Slot::Release => k.on_release.get_or_insert_with(Action::default),
        }
    }

    fn set(self, k: &mut KeyConfig, a: Action) {
        match self {
            // "Nothing" on release means no release action at all.
            Slot::Release if a == Action::None => k.on_release = None,
            _ => *self.get_mut(k) = a,
        }
    }

    /// How many of `Action::KINDS` this slot offers: a release can't change pages.
    fn kinds(self) -> usize {
        match self {
            Slot::Press => Action::KINDS.len(),
            Slot::Release => Action::KINDS.len() - 2,
        }
    }
}

/// ▶: try the slot's action from the editor.
fn test_button(app: &Rc<App>, i: u8, slot: Slot, current: &Action) -> gtk::Button {
    let test = gtk::Button::builder()
        .icon_name("media-playback-start-symbolic")
        .tooltip_text(match current {
            Action::Back => "Back only works on the deck",
            Action::Unknown { .. } => "This action needs a newer omadeck",
            _ => "Try it now",
        })
        .valign(gtk::Align::Center)
        .sensitive(match current {
            Action::None | Action::Back | Action::Unknown { .. } => false,
            Action::Page { page } => app.config.borrow().has_page(page),
            _ => true,
        })
        .css_classes(["flat"])
        .build();
    let a = app.clone();
    test.connect_clicked(move |_| {
        let act = a.key(i).map(|k| slot.get(&k)).unwrap_or_default();
        if let Action::Page { page } = act {
            a.show_page(Some(page), true);
        } else if let Err(e) = action::run(&act) {
            a.toast(&format!("{e:#}"));
        }
    });
    test
}

/// The "When pressed" / "When released" picker.
fn kind_row(app: &Rc<App>, i: u8, slot: Slot, current: &Action) -> adw::ComboRow {
    let offered = slot.kinds();
    let kinds = gtk::StringList::new(&Action::KINDS[..offered]);
    let shown = match current {
        Action::Unknown { kind, .. } => {
            // Listed last so the row can show it.
            kinds.append(kind);
            offered
        }
        a => a.kind_index(),
    };
    let title = if slot == Slot::Press { "When pressed" } else { "Action" };
    let row = adw::ComboRow::builder().title(title).model(&kinds).build();
    row.set_selected(shown as u32);
    match current {
        Action::Page { .. } => row.set_subtitle("Shows another page of keys on the deck"),
        Action::Back => row.set_subtitle("Returns to the page shown before this one, or Home"),
        _ => {}
    }
    let a = app.clone();
    row.connect_selected_notify(move |row| {
        let sel = row.selected() as usize;
        if sel == shown || sel >= offered {
            return;
        }
        let old = a.key(i).map(|k| slot.get(&k)).filter(|old| matches!(old, Action::Unknown { .. }));
        let mut action = Action::empty_of_kind(sel);
        if let Action::Page { page } = &mut action {
            // Nothing to choose from yet: make the page, like creating a folder.
            let others = a.config.borrow().pages.iter().any(|p| Some(&p.name) != a.current_page().as_ref());
            if !others {
                *page = new_folder(&a, i);
            }
        }
        a.edit(i, |k| slot.set(k, action));
        a.rebuild_editor();
        if let Some(old) = old {
            a.offer_undo_replaced(i, move |k| slot.set(k, old.clone()));
        }
    });
    row
}

fn action_rows(app: &Rc<App>, i: u8, slot: Slot, current: &Action, group: &adw::PreferencesGroup) {
    match current {
        Action::None => {}

        Action::Unknown { kind, .. } => {
            let row = adw::ActionRow::builder()
                .title("From a newer omadeck")
                .subtitle(format!(
                    "This key's action, “{kind}”, comes from a newer version of omadeck. It's kept as is; \
                     choose another action above to replace it."
                ))
                .subtitle_lines(0)
                .use_markup(false)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name("software-update-available-symbolic"));
            group.add(&row);
        }

        Action::App { command } => {
            let found = apps::find(command);
            let row =
                adw::ActionRow::builder()
                    .title("Application")
                    .subtitle(found.as_ref().map(|a| a.display_name().to_string()).unwrap_or_else(|| {
                        if command.is_empty() { "None chosen".into() } else { "Custom command".into() }
                    }))
                    .use_markup(false)
                    .build();
            if let Some(gicon) = found.as_ref().and_then(|a| a.icon()) {
                let img = gtk::Image::from_gicon(&gicon);
                img.set_pixel_size(32);
                row.add_prefix(&img);
            }
            let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
            let a = app.clone();
            choose.connect_clicked(move |_| {
                let a2 = a.clone();
                let at = a.key_ref(i);
                apps::pick(&a.window, move |info| {
                    let command = info
                        .id()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| info.commandline().map(|c| c.to_string_lossy().into()).unwrap_or_default());
                    let icon = media::import_app_icon(&info);
                    let name = info.display_name().to_string();
                    a2.edit_ref(&at, |k| {
                        slot.set(k, Action::App { command });
                        // The key's face follows what it launches when pressed, not on release.
                        if slot == Slot::Press {
                            k.label = name;
                            if icon.is_some() {
                                k.icon = icon;
                            }
                        }
                    });
                    a2.rebuild_editor();
                });
            });
            row.add_suffix(&choose);
            row.set_activatable_widget(Some(&choose));
            group.add(&row);

            let cmd = adw::EntryRow::builder().title("Desktop entry or command").text(command).build();
            let a = app.clone();
            cmd.connect_changed(move |e| {
                let command = e.text().to_string();
                a.edit(i, |k| slot.set(k, Action::App { command }));
            });
            group.add(&cmd);
        }

        Action::Script { path, terminal } => {
            let row = adw::EntryRow::builder().title("Script").text(path).build();
            let a = app.clone();
            let t = *terminal;
            row.connect_changed(move |e| {
                let path = e.text().to_string();
                a.edit(i, |k| slot.set(k, Action::Script { path, terminal: t }));
            });
            row.add_suffix(&browse_button(app, i, "Choose a script", None, move |a, at, p| {
                let path = paths::portable(&p);
                a.edit_ref(at, |k| slot.set(k, Action::Script { path, terminal: t }));
            }));
            group.add(&row);

            let term = adw::SwitchRow::builder()
                .title("Run in a terminal")
                .subtitle("Opens your terminal so you can watch the output")
                .active(*terminal)
                .build();
            let a = app.clone();
            term.connect_active_notify(move |r| {
                let on = r.is_active();
                a.edit(i, |k| {
                    if let Action::Script { terminal, .. } = slot.get_mut(k) {
                        *terminal = on;
                    }
                });
                a.rebuild_editor();
            });
            group.add(&term);
        }

        Action::Sound { file, volume } => {
            let row = adw::EntryRow::builder().title("Sound file").text(file).build();
            let a = app.clone();
            let v = *volume;
            row.connect_changed(move |e| {
                let file = e.text().to_string();
                a.edit(i, |k| slot.set(k, Action::Sound { file, volume: v }));
            });
            row.add_suffix(&browse_button(app, i, "Choose a sound", Some(("Audio", "audio/*")), move |a, at, p| {
                let file = paths::portable(&p);
                a.edit_ref(at, |k| slot.set(k, Action::Sound { file, volume: v }));
            }));
            group.add(&row);

            let vol = adw::SpinRow::with_range(0.0, 150.0, 5.0);
            vol.set_title("Volume (%)");
            vol.set_value((volume.unwrap_or(1.0) * 100.0).round() as f64);
            let a = app.clone();
            vol.connect_value_notify(move |r| {
                let pct = r.value() as f32 / 100.0;
                a.edit(i, |k| {
                    if let Action::Sound { volume, .. } = slot.get_mut(k) {
                        *volume = if (pct - 1.0).abs() < f32::EPSILON { None } else { Some(pct) };
                    }
                });
            });
            group.add(&vol);
        }

        Action::Url { url } => {
            let row = adw::EntryRow::builder().title("Website").text(url).input_purpose(gtk::InputPurpose::Url).build();
            let a = app.clone();
            row.connect_changed(move |e| {
                let url = e.text().to_string();
                a.edit(i, |k| slot.set(k, Action::Url { url }));
            });
            group.add(&row);

            let fetch_row = adw::ActionRow::builder()
                .title("Site icon")
                .subtitle("Asks the website for its icon. If it has none, DuckDuckGo's icon service is used instead")
                .build();
            let fetch = gtk::Button::builder().label("Fetch").valign(gtk::Align::Center).build();
            let a = app.clone();
            fetch.connect_clicked(move |btn| {
                let url = match a.key(i).map(|k| slot.get(&k)) {
                    Some(Action::Url { url }) if !url.trim().is_empty() => url,
                    _ => {
                        a.toast("Enter a website first");
                        return;
                    }
                };
                btn.set_sensitive(false);
                btn.set_label("Fetching…");
                let a = a.clone();
                let btn = btn.clone();
                let at = a.key_ref(i);
                glib::spawn_future_local(async move {
                    let u = url.clone();
                    let res = gio::spawn_blocking(move || media::fetch_favicon(&u)).await;
                    btn.set_sensitive(true);
                    btn.set_label("Fetch");
                    match res {
                        Ok(Ok((icon, source))) => {
                            if source == media::IconSource::DuckDuckGo {
                                a.toast("The site had no usable icon, so this one came from DuckDuckGo");
                            }
                            let name = media::host_of(&url).map(|h| site_name(&h)).unwrap_or_default();
                            a.edit_ref(&at, |k| {
                                k.icon = Some(icon);
                                if k.label.is_empty() {
                                    k.label = name;
                                }
                            });
                            a.rebuild_editor();
                        }
                        Ok(Err(e)) => a.toast(&format!("Couldn't fetch the icon: {e:#}")),
                        Err(_) => a.toast("Couldn't fetch the icon"),
                    }
                });
            });
            fetch_row.add_suffix(&fetch);
            group.add(&fetch_row);
        }

        Action::Hyprland { dispatch } => {
            group.set_description(Some(
                "Runs hyprctl dispatch. Classic syntax like workspace 3 · togglefloating · killactive · exec pavucontrol \
                 is translated for you, or write Lua directly: hl.dsp.focus({ workspace = \"3\" })",
            ));
            let row = adw::EntryRow::builder().title("Dispatcher and arguments").text(dispatch).build();
            let a = app.clone();
            row.connect_changed(move |e| {
                let dispatch = e.text().to_string();
                a.edit(i, |k| slot.set(k, Action::Hyprland { dispatch }));
            });
            group.add(&row);
        }

        Action::Page { page } => {
            let current = app.current_page();
            let names: Vec<String> =
                app.config.borrow().page_names().into_iter().filter(|n| Some(n) != current.as_ref()).collect();
            // A combo row always shows some item, so an unset or missing target gets a placeholder.
            let mut choices: Vec<Option<String>> = names.iter().cloned().map(Some).collect();
            if !names.contains(page) {
                choices.insert(0, None);
            }
            let placeholder = if page.is_empty() { "Choose a page".to_string() } else { format!("{page} (missing)") };
            let labels: Vec<&str> = choices.iter().map(|c| c.as_deref().unwrap_or(&placeholder)).collect();
            let row = adw::ComboRow::builder().title("Page").model(&gtk::StringList::new(&labels)).build();
            row.set_selected(choices.iter().position(|c| c.as_deref() == Some(page.as_str())).unwrap_or(0) as u32);
            let a = app.clone();
            row.connect_selected_notify(move |r| {
                if let Some(Some(name)) = choices.get(r.selected() as usize).cloned() {
                    a.edit(i, |k| slot.set(k, Action::Page { page: name }));
                    a.rebuild_editor();
                }
            });
            group.add(&row);

            let buttons = adw::ActionRow::builder().title("Open it to add keys").build();
            let open = gtk::Button::builder()
                .icon_name("go-next-symbolic")
                .tooltip_text("Edit this page")
                .valign(gtk::Align::Center)
                .sensitive(names.contains(page))
                .css_classes(["flat"])
                .build();
            let a = app.clone();
            let target = page.clone();
            open.connect_clicked(move |_| a.show_page(Some(target.clone()), true));
            let new = gtk::Button::builder().label("New page").valign(gtk::Align::Center).build();
            let a = app.clone();
            new.connect_clicked(move |_| {
                let name = new_folder(&a, i);
                a.edit(i, |k| slot.set(k, Action::Page { page: name }));
                a.rebuild_editor();
            });
            buttons.add_suffix(&new);
            buttons.add_suffix(&open);
            buttons.set_activatable_widget(Some(&open));
            group.add(&buttons);
        }

        Action::Back => {}
    }
}

/// Create a page for key `i` to open, and make the key look like a folder if it's plain.
fn new_folder(app: &Rc<App>, i: u8) -> String {
    let label = app.key(i).map(|k| k.label).unwrap_or_default();
    let name = app.add_page(if label.trim().is_empty() { "Page" } else { &label });
    let n = name.clone();
    app.edit(i, |k| {
        if k.icon.is_none() && k.glyph.is_none() {
            k.glyph = Some("\u{f024b}".into());
        }
        if k.label.is_empty() {
            k.label = n;
        }
    });
    name
}

/// "omarchy.org" → "Omarchy", "www.youtube.com" → "Youtube".
fn site_name(host: &str) -> String {
    let host = host.trim_start_matches("www.");
    let base = host.split('.').next().unwrap_or(host);
    let mut c = base.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn icon_row(app: &Rc<App>, i: u8, key: &KeyConfig) -> adw::ActionRow {
    let subtitle = key
        .icon
        .as_deref()
        .map(|p| paths::resolve(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
        .unwrap_or_else(|| "None".into());
    let row = adw::ActionRow::builder().title("Icon").subtitle(subtitle).use_markup(false).build();

    let preview = gtk::Image::builder().pixel_size(40).css_classes(["icon-preview"]).build();
    if let Some(img) = key.icon.as_deref().and_then(|p| omadeck_core::render::load_icon(&paths::resolve(p), 80)) {
        preview.set_paintable(Some(&media::texture(img)));
    } else {
        preview.set_icon_name(Some("image-x-generic-symbolic"));
    }
    row.add_prefix(&preview);

    if key.icon.is_some() {
        let clear = gtk::Button::builder()
            .icon_name("edit-clear-symbolic")
            .tooltip_text("Remove icon")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let a = app.clone();
        clear.connect_clicked(move |_| {
            a.edit(i, |k| k.icon = None);
            a.rebuild_editor();
        });
        row.add_suffix(&clear);
    }

    let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
    let a = app.clone();
    choose.connect_clicked(move |_| {
        let dialog = file_dialog("Choose an icon", Some(("Images", "image/*")));
        let a = a.clone();
        let at = a.key_ref(i);
        glib::spawn_future_local(async move {
            let Ok(file) = dialog.open_future(Some(&a.window)).await else { return };
            let Some(path) = file.path() else { return };
            match media::import_icon(&path) {
                Ok(rel) => {
                    a.edit_ref(&at, |k| k.icon = Some(rel));
                    a.rebuild_editor();
                }
                Err(e) => a.toast(&format!("{e:#}")),
            }
        });
    });
    row.add_suffix(&choose);
    row
}

fn color_row(
    app: &Rc<App>,
    i: u8,
    title: &str,
    current: Rgb,
    overridden: bool,
    set: fn(&mut KeyConfig, Option<String>),
) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).subtitle(if overridden { "Custom" } else { "From theme" }).build();
    if overridden {
        let reset = gtk::Button::builder()
            .icon_name("edit-undo-symbolic")
            .tooltip_text("Use the theme colour")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let a = app.clone();
        reset.connect_clicked(move |_| {
            a.edit(i, |k| set(k, None));
            a.rebuild_editor();
        });
        row.add_suffix(&reset);
    }
    let button = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::builder().with_alpha(false).build()));
    button.set_valign(gtk::Align::Center);
    button.set_rgba(&gdk::RGBA::new(current.0 as f32 / 255.0, current.1 as f32 / 255.0, current.2 as f32 / 255.0, 1.0));
    let a = app.clone();
    button.connect_rgba_notify(move |b| {
        let c = b.rgba();
        let to = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
        let hex = Rgb(to(c.red()), to(c.green()), to(c.blue())).hex();
        a.edit(i, |k| set(k, Some(hex)));
        a.rebuild_editor();
    });
    row.add_suffix(&button);
    row
}

fn file_dialog(title: &str, filter: Option<(&str, &str)>) -> gtk::FileDialog {
    let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
    if let Some((name, mime)) = filter {
        let f = gtk::FileFilter::new();
        f.set_name(Some(name));
        f.add_mime_type(mime);
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&f);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&f));
    }
    dialog
}

fn browse_button(
    app: &Rc<App>,
    i: u8,
    title: &'static str,
    filter: Option<(&'static str, &'static str)>,
    on_file: impl Fn(&Rc<App>, &KeyRef, std::path::PathBuf) + Clone + 'static,
) -> gtk::Button {
    let btn = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text(title)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let a = app.clone();
    btn.connect_clicked(move |_| {
        let dialog = file_dialog(title, filter);
        let a = a.clone();
        let on_file = on_file.clone();
        let at = a.key_ref(i);
        glib::spawn_future_local(async move {
            if let Ok(file) = dialog.open_future(Some(&a.window)).await
                && let Some(p) = file.path()
            {
                on_file(&a, &at, p);
                a.rebuild_editor();
            }
        });
    });
    btn
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(cmd: &str) -> Action {
        Action::App { command: cmd.into() }
    }

    #[test]
    fn each_slot_edits_its_own_action() {
        let mut k = KeyConfig::new(0);
        Slot::Press.set(&mut k, app("voxtype record start"));
        Slot::Release.set(&mut k, app("voxtype record stop"));
        assert_eq!(k.action, app("voxtype record start"));
        assert_eq!(k.on_release, Some(app("voxtype record stop")));
        assert_eq!(Slot::Release.get(&k), app("voxtype record stop"));
    }

    #[test]
    fn nothing_on_release_removes_the_release_action() {
        let mut k = KeyConfig::new(0);
        Slot::Release.set(&mut k, app("stop"));
        Slot::Release.set(&mut k, Action::None);
        assert_eq!(k.on_release, None, "no empty [key.on_release] left in the file");
        assert!(k.is_blank());
    }

    #[test]
    fn a_release_cant_be_set_to_change_pages() {
        let offered = &Action::KINDS[..Slot::Release.kinds()];
        assert!(!offered.contains(&"Open page") && !offered.contains(&"Go back"), "{offered:?}");
        assert_eq!(Slot::Press.kinds(), Action::KINDS.len());
        for kind in 0..Slot::Release.kinds() {
            assert!(!Action::empty_of_kind(kind).is_navigation());
        }
    }
}
