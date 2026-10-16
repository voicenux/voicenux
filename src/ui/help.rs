//! The list of keyboard shortcuts, opened with F1 or Ctrl+?.

use adw::prelude::*;

use super::{label, line_icon};
use crate::i18n::{t, t_dyn};

/// A part of the list: its icon, its title, and its keys with what they do.
type Section = (&'static str, &'static str, Vec<(&'static str, &'static str)>);

/// The keys of the app, by where they work. A key like "Ctrl+T" shows as two key caps.
fn sections() -> Vec<Section> {
    vec![
        (
            "keyboard",
            t("Everywhere"),
            vec![
                ("Ctrl+F", t("Search")),
                ("Ctrl+1 … 4", t("Go to a page")),
                ("Ctrl+T", t("Voice changer on or off")),
                ("Ctrl+H", t("Hear myself on or off")),
                ("Ctrl+B", t("Background on or off")),
                ("Ctrl+M", t("Mute the microphone")),
                ("Ctrl+R", t("Random voice")),
                ("Ctrl+,", t("Settings")),
                ("F1", t("This list")),
                ("Ctrl+W", t("Close the window")),
                ("Ctrl+Q", t("Quit")),
            ],
        ),
        (
            "mic",
            t("Voices"),
            vec![
                ("↑ ↓ ← →", t("Move between voices")),
                ("Enter", t("Use the voice")),
                ("Menu", t("More for a voice, or right-click it")),
                ("A … Z", t("Type anywhere to search")),
            ],
        ),
        (
            "volume-2",
            t("Soundboard"),
            vec![
                ("Enter", t("Play the sound")),
                ("Esc", t("Stop all sounds")),
                ("Del", t("Remove the sound")),
                ("Alt+← →", t("Move the sound")),
                ("Ctrl+O", t("Add sounds")),
            ],
        ),
        (
            "scissors",
            t("Trim sound"),
            vec![
                ("Spacebar", t("Listen or pause")),
                ("← →", t("Move the playhead, Shift for 1 s, Ctrl for 0.01 s")),
                ("Home End", t("Jump to the start or end")),
                ("+ −", t("Zoom in or out, also Ctrl with the wheel")),
                ("0", t("Show all of the sound")),
                ("I O", t("Set start and end at the playhead")),
                ("Ctrl+Z", t("Undo, Ctrl+Shift+Z redoes")),
                ("Ctrl+S", t("Save")),
            ],
        ),
    ]
}

/// A key as German keyboards print it, like "Strg" for Ctrl. Only names of keys: a letter like
/// "B" stays, even where the same text is a note with another name.
fn key_name(key: &str) -> String {
    match key {
        "Ctrl" | "Spacebar" | "Home" | "End" | "Del" | "Menu" => t_dyn(key),
        _ => key.to_string(),
    }
}

/// Keys as key caps: "+" joins keys pressed together, a space parts keys that do the same.
/// Key names are English in the list and translated here, like "Ctrl" to "Strg".
fn caps(keys: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    b.set_valign(gtk::Align::Center);
    for (i, alt) in keys.split(' ').enumerate() {
        if alt == "…" {
            b.append(&label("…", &["dim-label"]));
            continue;
        }
        if i > 0 {
            b.append(&gtk::Box::new(gtk::Orientation::Horizontal, 0));
        }
        // A plus alone is the key itself.
        let keys: Vec<&str> = if alt == "+" { vec!["+"] } else { alt.split('+').collect() };
        for (j, key) in keys.into_iter().enumerate() {
            if j > 0 {
                b.append(&label("+", &["dim-label", "caption"]));
            }
            b.append(&label(&key_name(key), &["vn-key", "vn-key-big"]));
        }
    }
    b
}

pub fn show(parent: &impl IsA<gtk::Widget>) {
    // Two columns that each keep their own height: everywhere and voices on the left, the
    // soundboard and trimming on the right.
    let grid = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    grid.set_homogeneous(true);
    let columns = [gtk::Box::new(gtk::Orientation::Vertical, 18), gtk::Box::new(gtk::Orientation::Vertical, 18)];
    for c in &columns {
        c.set_valign(gtk::Align::Start);
        grid.append(c);
    }
    for (i, (pic, title, keys)) in sections().into_iter().enumerate() {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 7);
        card.add_css_class("vn-keys-card");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        head.append(&line_icon(pic, 20));
        head.append(&label(title, &["vn-keys-title"]));
        card.append(&head);
        for (k, what) in keys {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let text = label(what, &[]);
            text.set_xalign(0.0);
            text.set_hexpand(true);
            text.set_wrap(true);
            text.set_max_width_chars(26);
            row.append(&text);
            row.append(&caps(k));
            card.append(&row);
        }
        columns[i / 2].append(&card);
    }
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.set_margin_top(6);
    body.set_margin_bottom(22);
    body.set_margin_start(22);
    body.set_margin_end(22);
    body.append(&grid);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&body)
        .build();
    super::smooth_scroll(&scroll);

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(t("Keyboard shortcuts"), "")));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&scroll));
    let dialog = adw::Dialog::builder().content_width(900).child(&view).build();
    dialog.present(Some(parent));
}
