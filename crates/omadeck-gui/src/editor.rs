//! The right-hand panel: what a key does and how it looks.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use omadeck_core::{Action, KeyConfig, Rgb, action, paths};

use crate::app::App;
use crate::{apps, media};

pub fn build(app: &Rc<App>, i: u8) -> gtk::Widget {
    let key = app.config.borrow().key(i).cloned().unwrap_or_else(|| KeyConfig::new(i));
    let deck = app.deck_info();
    let page = adw::PreferencesPage::new();

    // --- action ---------------------------------------------------------------------------
    let group = adw::PreferencesGroup::builder()
        .title(format!("Key {}", i + 1))
        .description(format!("Row {} · Column {}", i / deck.cols + 1, i % deck.cols + 1))
        .build();

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let test = gtk::Button::builder()
        .icon_name("media-playback-start-symbolic")
        .tooltip_text("Try it now")
        .valign(gtk::Align::Center)
        .sensitive(key.action != Action::None)
        .css_classes(["flat"])
        .build();
    let a = app.clone();
    test.connect_clicked(move |_| {
        let act = a.config.borrow().key(i).map(|k| k.action.clone()).unwrap_or_default();
        if let Err(e) = action::run(&act) {
            a.toast(&format!("{e:#}"));
        }
    });
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

    let kinds = gtk::StringList::new(&Action::KINDS);
    let kind_row = adw::ComboRow::builder().title("When pressed").model(&kinds).build();
    kind_row.set_selected(key.action.kind_index() as u32);
    let a = app.clone();
    kind_row.connect_selected_notify(move |row| {
        let sel = row.selected() as usize;
        let cur = a.config.borrow().key(i).map(|k| k.action.kind_index()).unwrap_or(0);
        if sel != cur {
            a.edit(i, |k| k.action = Action::empty_of_kind(sel));
            a.rebuild_editor();
        }
    });
    group.add(&kind_row);
    action_rows(app, i, &key, &group);
    page.add(&group);

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

fn action_rows(app: &Rc<App>, i: u8, key: &KeyConfig, group: &adw::PreferencesGroup) {
    match &key.action {
        Action::None => {}

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
                apps::pick(&a.window, move |info| {
                    let command = info
                        .id()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| info.commandline().map(|c| c.to_string_lossy().into()).unwrap_or_default());
                    let icon = media::import_app_icon(&info);
                    let name = info.display_name().to_string();
                    a2.edit(i, |k| {
                        k.action = Action::App { command };
                        k.label = name;
                        if icon.is_some() {
                            k.icon = icon;
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
                a.edit(i, |k| k.action = Action::App { command });
            });
            group.add(&cmd);
        }

        Action::Script { path, terminal } => {
            let row = adw::EntryRow::builder().title("Script").text(path).build();
            let a = app.clone();
            let t = *terminal;
            row.connect_changed(move |e| {
                let path = e.text().to_string();
                a.edit(i, |k| k.action = Action::Script { path, terminal: t });
            });
            row.add_suffix(&browse_button(app, "Choose a script", None, move |a, p| {
                let path = paths::portable(&p);
                a.edit(i, |k| k.action = Action::Script { path, terminal: t });
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
                    if let Action::Script { terminal, .. } = &mut k.action {
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
                a.edit(i, |k| k.action = Action::Sound { file, volume: v });
            });
            row.add_suffix(&browse_button(app, "Choose a sound", Some(("Audio", "audio/*")), move |a, p| {
                let file = paths::portable(&p);
                a.edit(i, |k| k.action = Action::Sound { file, volume: v });
            }));
            group.add(&row);

            let vol = adw::SpinRow::with_range(0.0, 150.0, 5.0);
            vol.set_title("Volume (%)");
            vol.set_value((volume.unwrap_or(1.0) * 100.0).round() as f64);
            let a = app.clone();
            vol.connect_value_notify(move |r| {
                let pct = r.value() as f32 / 100.0;
                a.edit(i, |k| {
                    if let Action::Sound { volume, .. } = &mut k.action {
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
                a.edit(i, |k| k.action = Action::Url { url });
            });
            group.add(&row);

            let fetch_row = adw::ActionRow::builder()
                .title("Site icon")
                .subtitle("Asks the website for its icon. If it has none, DuckDuckGo's icon service is used instead")
                .build();
            let fetch = gtk::Button::builder().label("Fetch").valign(gtk::Align::Center).build();
            let a = app.clone();
            fetch.connect_clicked(move |btn| {
                let url = match a.config.borrow().key(i).map(|k| k.action.clone()) {
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
                            a.edit(i, |k| {
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
                a.edit(i, |k| k.action = Action::Hyprland { dispatch });
            });
            group.add(&row);
        }
    }
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
        glib::spawn_future_local(async move {
            let Ok(file) = dialog.open_future(Some(&a.window)).await else { return };
            let Some(path) = file.path() else { return };
            match media::import_icon(&path) {
                Ok(rel) => {
                    a.edit(i, |k| k.icon = Some(rel));
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
    title: &'static str,
    filter: Option<(&'static str, &'static str)>,
    on_file: impl Fn(&Rc<App>, std::path::PathBuf) + Clone + 'static,
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
        glib::spawn_future_local(async move {
            if let Ok(file) = dialog.open_future(Some(&a.window)).await
                && let Some(p) = file.path()
            {
                on_file(&a, p);
                a.rebuild_editor();
            }
        });
    });
    btn
}
