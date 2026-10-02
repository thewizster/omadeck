//! "Choose an application" dialog listing installed desktop apps.

use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

pub fn find(id: &str) -> Option<gio::AppInfo> {
    gio::AppInfo::all().into_iter().find(|a| a.id().is_some_and(|i| i == id))
}

fn installed() -> Vec<gio::AppInfo> {
    let mut apps: Vec<_> = gio::AppInfo::all().into_iter().filter(|a| a.should_show()).collect();
    apps.sort_by_key(|a| a.display_name().to_lowercase());
    apps
}

pub fn pick(parent: &impl IsA<gtk::Widget>, on_pick: impl Fn(gio::AppInfo) + 'static) {
    let apps = Rc::new(installed());
    let haystacks: Rc<Vec<String>> = Rc::new(
        apps.iter()
            .map(|a| {
                format!("{} {} {}", a.display_name(), a.description().unwrap_or_default(), a.id().unwrap_or_default())
                    .to_lowercase()
            })
            .collect(),
    );

    let search = gtk::SearchEntry::builder().placeholder_text("Search applications").hexpand(true).build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_start(12)
        .margin_end(12)
        .margin_top(6)
        .margin_bottom(12)
        .valign(gtk::Align::Start)
        .build();

    for a in apps.iter() {
        let row = adw::ActionRow::builder()
            .title(a.display_name().as_str())
            .subtitle(a.description().unwrap_or_default().as_str())
            .subtitle_lines(1)
            .use_markup(false)
            .activatable(true)
            .build();
        let icon = match a.icon() {
            Some(gicon) => gtk::Image::from_gicon(&gicon),
            None => gtk::Image::from_icon_name("application-x-executable"),
        };
        icon.set_pixel_size(32);
        row.add_prefix(&icon);
        list.append(&row);
    }

    let s = search.clone();
    let hay = haystacks.clone();
    list.set_filter_func(move |row| {
        let q = s.text().to_lowercase();
        q.is_empty() || hay.get(row.index() as usize).is_some_and(|h| q.split_whitespace().all(|w| h.contains(w)))
    });
    let l = list.clone();
    search.connect_search_changed(move |_| l.invalidate_filter());

    let header = adw::HeaderBar::new();
    let search_bar = gtk::Box::builder().margin_start(12).margin_end(12).margin_bottom(6).build();
    search_bar.append(&search);
    let scroller =
        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&list).build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.add_top_bar(&search_bar);
    view.set_content(Some(&scroller));

    let dialog = adw::Dialog::builder()
        .title("Choose an application")
        .content_width(480)
        .content_height(620)
        .child(&view)
        .build();

    let on_pick = Rc::new(on_pick);
    let d = dialog.clone();
    let a = apps.clone();
    let cb = on_pick.clone();
    list.connect_row_activated(move |_, row| {
        if let Some(app) = a.get(row.index() as usize) {
            d.close();
            cb(app.clone());
        }
    });
    // Enter in the search box picks the first match.
    let d = dialog.clone();
    let l = list.clone();
    search.connect_activate(move |_| {
        let mut i = 0;
        while let Some(row) = l.row_at_index(i) {
            if row.is_child_visible() {
                if let Some(app) = apps.get(i as usize) {
                    d.close();
                    on_pick(app.clone());
                }
                return;
            }
            i += 1;
        }
    });

    dialog.present(Some(parent));
    search.grab_focus();
}
