//! The soundboard page: big pads that play sounds into your microphone.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use super::visuals::progress_ring;
use super::{animate_in, board, clear, icon, icon_picker, label, line_icon, popup, smooth_scroll, tour, value_field};
use crate::i18n::{fill, t, t_dyn};
use crate::soundboard::{PlayMode, Sound};
use crate::state::{Change, St};

struct Pad {
    child: gtk::FlowBoxChild,
    pad: gtk::Box,
    progress: Rc<Cell<f32>>,
    ring: gtk::DrawingArea,
    /// The shortcut of the sound, as a key cap.
    keys: gtk::Label,
    /// Marks of the mode and the cut, top left. A check takes their place while it is picked.
    badges: gtk::Box,
    check: gtk::Image,
}

/// The soundboard as a shelf with folders, for the grid that `board` shares with the voices.
struct SoundShelf(St);

impl board::Shelf for SoundShelf {
    fn folders(&self) -> Vec<String> {
        self.0.library.borrow().folders.clone()
    }

    fn folder_of(&self, id: &str) -> Option<String> {
        self.0.library.borrow().find(id).and_then(|s| s.folder.clone())
    }

    fn order(&self) -> Vec<String> {
        self.0.sounds_shown().into_iter().map(|s| s.id).collect()
    }

    fn add_folder(&self, name: &str) -> String {
        self.0.add_folder(name)
    }

    fn rename_folder(&self, old: &str, new: &str) -> bool {
        self.0.rename_folder(old, new)
    }

    fn remove_folder(&self, name: &str) {
        self.0.remove_folder(name)
    }

    fn move_folder(&self, name: &str, target: &str, after: bool) {
        self.0.move_folder(name, target, after)
    }

    fn move_to_folder(&self, ids: &[String], folder: Option<&str>) {
        self.0.move_to_folder(ids, folder)
    }

    fn move_items(&self, ids: &[String], target: &str, after: bool) {
        self.0.move_sounds(ids, target, after)
    }

    fn note(&self, text: String) {
        self.0.notify(Change::Notice(text))
    }
}

/// Folders look like pads.
const FOLDER_LOOK: board::TileLook = board::TileLook { card: "sound-pad", size: (136, 140), title: "vn-pad-name" };

pub struct SoundboardPage {
    pub widget: adw::ToolbarView,
    pub search: gtk::SearchEntry,
    /// Opens the file chooser to add sounds.
    pub add: Rc<dyn Fn()>,
    /// Header buttons that drop their text in a narrow window.
    pub shrink: Vec<adw::ButtonContent>,
    /// Opens the MyInstants search with some words.
    pub find_online: Rc<dyn Fn(&str)>,
}

pub fn build(st: &St) -> SoundboardPage {
    let shelf: board::Shared = Rc::new(SoundShelf(st.clone()));
    let header = adw::HeaderBar::new();
    // The tools sit on the left, in line with the page, and the rest of the bar is for dragging.
    header.set_show_title(false);
    let (search_box, search) = super::search_field(t("Search sounds"));
    search_box.set_margin_start(12);
    header.pack_start(&search_box);
    let content = |icon: &str, text: &str| adw::ButtonContent::builder().icon_name(icon).label(text).build();
    let find_content = content("system-search-symbolic", t("Find sounds"));
    // MyInstants has all the sounds people want, so finding them is the main thing here. Own
    // files are in the More menu, and can be dropped on the page.
    let find = gtk::ToggleButton::new();
    find.set_child(Some(&find_content));
    find.add_css_class("suggested-action");
    find.set_tooltip_text(Some(t("Find sounds on MyInstants")));
    header.pack_start(&find);
    let add: Rc<dyn Fn()> = Rc::new({
        let (st, header) = (st.clone(), header.downgrade());
        move || {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(t("Audio files")));
            filter.add_mime_type("audio/*");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder().title(t("Add sounds")).filters(&filters).modal(true).build();
            let st = st.clone();
            let parent = header.upgrade().and_then(|h| h.root()).and_downcast::<gtk::Window>();
            dialog.open_multiple(parent.as_ref(), None::<&gio::Cancellable>, move |res| {
                if let Ok(files) = res {
                    let paths: Vec<_> = files.iter::<gio::File>().flatten().filter_map(|f| f.path()).collect();
                    import(&st, &paths);
                }
            });
        }
    });

    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(16)
        .column_spacing(18)
        .row_spacing(18)
        .valign(gtk::Align::Start)
        .margin_start(20)
        .margin_end(20)
        .margin_top(16)
        .margin_bottom(24)
        .activate_on_single_click(true)
        .build();
    grid.add_css_class("vn-pads");
    let scroll =
        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&grid).build();
    smooth_scroll(&scroll);

    let empty = adw::StatusPage::builder()
        .paintable(&super::texture("speaker-high-volume"))
        .title(t("No sounds yet"))
        .description(t("Add audio files, or drop them here. MP3, OGG, FLAC and WAV work."))
        .build();
    let restore = gtk::Button::with_label(t("Bring back the starter sounds"));
    restore.add_css_class("pill");
    restore.set_halign(gtk::Align::Center);
    empty.set_child(Some(&restore));
    restore.connect_clicked({
        let st = st.clone();
        move |_| st.restore_starter_sounds()
    });

    // Nothing matches the search: maybe MyInstants has it.
    let none = adw::StatusPage::builder()
        .paintable(&super::load_picture("magnifying-glass-tilted-left"))
        .title(t("No sounds found"))
        .build();
    let ask_online = gtk::Button::new();
    ask_online.add_css_class("vn-gradient-button");
    ask_online.set_halign(gtk::Align::Center);
    none.set_child(Some(&ask_online));

    let stack = gtk::Stack::new();
    stack.add_css_class("vn-pad-area");
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.add_named(&scroll, Some("grid"));
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&none, Some("none"));

    let (deck, deck_texts, deck_labels, deck_volume, stop) = deck(st);

    // In a folder: the way back up over the grid. Drop a sound on it to take it out.
    let path = board::path_bar(t("All sounds"), t("Back to all sounds (Backspace)"));
    // And the bar for picked sounds under the grid.
    let picks = Rc::new(board::Picks::default());
    let count = label("", &["heading"]);
    let done = gtk::Button::new();
    let delete = gtk::Button::with_label(t("Delete"));
    delete.add_css_class("destructive-action");
    let move_to = gtk::MenuButton::new();
    move_to.set_label(t("Move to folder"));
    move_to.set_always_show_arrow(true);
    let select_all = gtk::Button::with_label(t("Select all"));
    select_all.set_tooltip_text(Some(&super::with_key(t("Select all"), "A")));
    let picks_bar =
        board::picks_bar(&done, &count, &[select_all.upcast_ref(), move_to.upcast_ref(), delete.upcast_ref()]);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&path.widget);
    body.append(&stack);
    body.append(&deck);
    let deck_widget = deck.clone();
    // The deck drops texts when the page gets too narrow for it, also when Find sounds takes
    // room: first those of the play modes and Stop all (their pictures and tooltips stay), then
    // "For everyone" and "Hear them myself", and the slider gets shorter. How wide it needs to be
    // is measured when it first shows, so other languages and font sizes fit too.
    let bin = adw::BreakpointBin::builder().child(&body).width_request(360).height_request(300).hexpand(true).build();
    let max_width = |px: f64| {
        adw::BreakpointCondition::new_length(adw::BreakpointConditionLengthType::MaxWidth, px, adw::LengthUnit::Px)
    };
    // The last breakpoint that fits wins, so the narrow one comes second and drops all.
    let medium = adw::Breakpoint::new(max_width(0.0));
    let narrow = adw::Breakpoint::new(max_width(0.0));
    for (object, property) in &deck_texts {
        medium.add_setter(object, property, Some(&"".to_value()));
        narrow.add_setter(object, property, Some(&"".to_value()));
    }
    for w in &deck_labels {
        narrow.add_setter(w, "visible", Some(&false.to_value()));
    }
    narrow.add_setter(&deck_volume, "width-request", Some(&160.to_value()));
    // The percent says enough there, the speaker gives its room to the slider.
    if let Some(speaker) = deck_volume.first_child() {
        narrow.add_setter(&speaker, "visible", Some(&false.to_value()));
    }
    bin.add_breakpoint(medium.clone());
    bin.add_breakpoint(narrow.clone());
    let measured = Cell::new(false);
    bin.connect_map(move |_| {
        if measured.replace(true) {
            return;
        }
        let natural = || deck.measure(gtk::Orientation::Horizontal, -1).1 as f64;
        let full = natural();
        let saved: Vec<glib::Value> = deck_texts.iter().map(|(o, p)| o.property_value(p)).collect();
        for (o, p) in &deck_texts {
            o.set_property_from_value(p, &"".to_value());
        }
        let without = natural();
        for ((o, p), v) in deck_texts.iter().zip(saved) {
            o.set_property_from_value(p, &v);
        }
        medium.set_condition(Some(&max_width(full + 4.0)));
        narrow.set_condition(Some(&max_width(without + 4.0)));
    });

    // Finding sounds online slides in on the right, so new sounds show up in the grid beside it.
    let (finder, online) = super::find_sounds::panel(st);
    let reveal = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideLeft)
        .transition_duration(260)
        .child(&finder)
        .build();
    // The entry in the panel wants to grow, and the revealer would take that over: closed, it
    // would still get half of the free width, and the grid only the other half.
    reveal.set_hexpand(false);
    find.connect_toggled({
        let st = st.clone();
        let reveal = reveal.clone();
        let online = online.clone();
        move |b| {
            reveal.set_reveal_child(b.is_active());
            if b.is_active() {
                online.grab_focus();
            } else {
                st.stop_preview();
            }
        }
    });
    // Escape empties the search, a second one closes the panel.
    online.connect_stop_search({
        let find = find.clone();
        move |e| {
            if e.text().is_empty() {
                find.set_active(false);
            } else {
                e.set_text("");
            }
        }
    });
    ask_online.connect_clicked({
        let (find, online, search) = (find.clone(), online.clone(), search.clone());
        move |_| {
            online.set_text(search.text().trim());
            if find.is_active() {
                online.grab_focus();
            } else {
                find.set_active(true);
            }
        }
    });
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.append(&bin);
    content.append(&reveal);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.add_top_bar(&discord_banner(st));
    view.set_content(Some(&content));
    view.add_bottom_bar(&picks_bar);
    view.set_reveal_bottom_bars(false);

    let pads: Rc<RefCell<HashMap<String, Pad>>> = Rc::default();
    // The first pad that matches the search, top left: Enter in the search plays it.
    let first: Rc<RefCell<Option<String>>> = Rc::default();
    // The folder the grid shows, None for all sounds. Folders sit on the grid like pads.
    let shown: Rc<RefCell<Option<String>>> = Rc::default();
    let tiles: board::Folders = Rc::default();
    // Sounds that just moved, and the folder that took some: they bounce when the grid is built.
    let landing = Rc::new(board::Landing::default());
    let item_of: board::ItemOf = Rc::new({
        let pads = pads.clone();
        move |w: &gtk::Widget| {
            pads.borrow().iter().find(|(_, p)| p.child.upcast_ref::<gtk::Widget>() == w).map(|(id, _)| id.clone())
        }
    });
    let faces: board::Faces = Rc::new({
        let pads = pads.clone();
        move |id: &str| pads.borrow().get(id).map(|p| (p.child.clone(), p.pad.clone().upcast()))
    });

    let sync_picks: Rc<dyn Fn()> = Rc::new({
        let (pads, picks, view, count) = (pads.clone(), picks.clone(), view.clone(), count.clone());
        let (select_all, grid, item_of) = (select_all.clone(), grid.clone(), item_of.clone());
        move || {
            let all = picks.all_of(&board::shown_items(&grid, &item_of));
            let ids = picks.ids.borrow();
            select_all.set_label(if all { t("Deselect all") } else { t("Select all") });
            for (id, p) in pads.borrow().iter() {
                let on = ids.contains(id);
                if on {
                    p.pad.add_css_class("vn-selected");
                } else {
                    p.pad.remove_css_class("vn-selected");
                }
                if on && !p.check.is_visible() {
                    board::bounce(&p.check, "vn-pop");
                }
                p.check.set_visible(on);
                p.badges.set_visible(!on);
            }
            view.set_reveal_bottom_bars(!ids.is_empty());
            count.set_label(&fill(t("{} selected"), &[&ids.len().to_string()]));
        }
    });

    // Shows the pads whose name has all words of the search.
    let refilter = {
        let (st, pads, stack, search, none, ask_online) =
            (st.clone(), pads.clone(), stack.clone(), search.clone(), none.clone(), ask_online.clone());
        let (grid, first, folder, tiles) = (grid.clone(), first.clone(), shown.clone(), tiles.clone());
        Rc::new(move || {
            let query = search.text().trim().to_string();
            let words: Vec<String> = super::fold(&query).split_whitespace().map(String::from).collect();
            let lib = st.library.borrow();
            let folder = folder.borrow().clone();
            let mut shown = 0;
            // Like a file manager: the top shows the folders and the sounds in none, a folder its
            // sounds. A search on the top looks in all folders.
            for (c, _) in tiles.borrow().iter() {
                let visible = folder.is_none() && words.is_empty();
                c.set_visible(visible);
                shown += visible as usize;
            }
            for s in &lib.sound {
                let Some(p) = pads.borrow().get(&s.id).map(|p| p.child.clone()) else { continue };
                let name = super::fold(&format!("{} {}", t_dyn(&s.name), s.name));
                let here = s.folder == folder || (folder.is_none() && !words.is_empty());
                let visible = here && words.iter().all(|w| name.contains(w.as_str()));
                p.set_visible(visible);
                shown += visible as usize;
            }
            let mut top = None;
            let mut child = grid.first_child();
            while let Some(c) = child {
                if top.is_none() && c.is_visible() && !words.is_empty() {
                    top = pads.borrow().iter().find(|(_, p)| p.child == c).map(|(id, _)| id.clone());
                }
                child = c.next_sibling();
            }
            for (id, p) in pads.borrow().iter() {
                if top.as_ref() == Some(id) {
                    p.pad.add_css_class("vn-first");
                } else {
                    p.pad.remove_css_class("vn-first");
                }
            }
            *first.borrow_mut() = top;
            // An empty folder says how sounds get in, not that the search found nothing.
            let empty_folder = folder.is_some() && query.is_empty();
            ask_online.set_visible(!empty_folder);
            if empty_folder {
                none.set_title(t("Nothing in this folder yet"));
                none.set_description(Some(t(
                    "Drag sounds onto the folder, or pick some with Ctrl+click and move them here.",
                )));
            } else {
                none.set_title(t("No sounds found"));
                none.set_description(Some(&fill(t("Nothing on your soundboard is called “{}”."), &[&query])));
            }
            ask_online.set_label(&fill(t("Search MyInstants for “{}”"), &[&query]));
            stack.set_visible_child_name(if lib.sound.is_empty() && lib.folders.is_empty() {
                "empty"
            } else if shown == 0 {
                "none"
            } else {
                "grid"
            });
        })
    };

    let sync_path: Rc<dyn Fn()> = Rc::new({
        let (shown, bar, here) = (shown.clone(), path.widget.clone(), path.here.clone());
        move || {
            let folder = shown.borrow().clone();
            bar.set_visible(folder.is_some());
            here.set_label(folder.as_deref().unwrap_or(""));
        }
    });
    // Goes into a folder, or back up with None, at once like a file manager: dozens of pads
    // fading in together made it stutter. The picks stay only for a spring while dragging:
    // they go along into the folder.
    let goto: Rc<dyn Fn(Option<String>, bool)> = Rc::new({
        let (shown, picks, sync_path, refilter, sync_picks) =
            (shown.clone(), picks.clone(), sync_path.clone(), refilter.clone(), sync_picks.clone());
        let scroll = scroll.downgrade();
        move |folder: Option<String>, keep_picks: bool| {
            *shown.borrow_mut() = folder;
            if !keep_picks {
                picks.clear();
            }
            sync_path();
            refilter();
            sync_picks();
            if let Some(scroll) = scroll.upgrade() {
                scroll.vadjustment().set_value(0.0);
            }
        }
    });
    let open_folder: Rc<dyn Fn(Option<String>)> = {
        let goto = goto.clone();
        Rc::new(move |folder| goto(folder, false))
    };
    let spring_to: Rc<dyn Fn(Option<String>)> = Rc::new(move |folder| goto(folder, true));

    let rebuild = {
        let (st, grid, pads, refilter) = (st.clone(), grid.clone(), pads.clone(), refilter.clone());
        let (picks, sync_picks, sync_path) = (picks.clone(), sync_picks.clone(), sync_path.clone());
        let (shown, tiles, open_folder, spring_to) =
            (shown.clone(), tiles.clone(), open_folder.clone(), spring_to.clone());
        let (shelf, faces, landing) = (shelf.clone(), faces.clone(), landing.clone());
        Rc::new(move |animate: bool| {
            // Keeps the focus on the same sound, or the one that took its place.
            let focused = grid.focus_child().and_then(|c| {
                let pads = pads.borrow();
                let id = pads.iter().find(|(_, p)| p.child == c).map(|(id, _)| id.clone())?;
                Some((id, c.downcast_ref::<gtk::FlowBoxChild>()?.index()))
            });
            clear(&grid);
            pads.borrow_mut().clear();
            tiles.borrow_mut().clear();
            let sounds = st.sounds_shown();
            let folders = st.library.borrow().folders.clone();
            // The folder went away, removed or renamed: back to the top.
            if shown.borrow().as_ref().is_some_and(|f| !folders.contains(f)) {
                shown.borrow_mut().take();
            }
            sync_path();
            for name in folders {
                let inside: Vec<&Sound> =
                    sounds.iter().filter(|s| s.folder.as_deref() == Some(name.as_str())).collect();
                let icons: Vec<String> = inside.iter().take(3).map(|s| s.icon.clone()).collect();
                let count = match inside.len() {
                    0 => t("Empty").to_string(),
                    1 => t("1 sound").to_string(),
                    n => fill(t("{} sounds"), &[&n.to_string()]),
                };
                let child = board::folder_tile(&name, &icons, &count, &FOLDER_LOOK);
                board::folder_actions(&shelf, &child, &name, &picks, &sync_picks, &open_folder, &spring_to, &landing);
                grid.append(&child);
                tiles.borrow_mut().push((child, name));
            }
            for (i, s) in sounds.iter().enumerate() {
                let p = pad(&st, s, &landing);
                board::drag_source(&p.child, p.pad.upcast_ref(), &s.id, &picks, &faces);
                grid.append(&p.child);
                if animate {
                    animate_in(&p.child, i);
                }
                pads.borrow_mut().insert(s.id.clone(), p);
            }
            picks.ids.borrow_mut().retain(|id| pads.borrow().contains_key(id));
            refilter();
            sync_picks();
            landing.play(|id| pads.borrow().get(id).map(|p| p.child.clone().upcast()), &tiles);
            if let Some((id, index)) = focused {
                let child = pads
                    .borrow()
                    .get(&id)
                    .map(|p| p.child.clone())
                    .or_else(|| grid.child_at_index(index.min(sounds.len() as i32 - 1)));
                if let Some(c) = child {
                    c.grab_focus();
                }
            }
        })
    };
    rebuild(false);
    search.connect_search_changed({
        let refilter = refilter.clone();
        move |_| refilter()
    });
    // Enter plays the first match and leaves the search as it is, for the next one. Down goes
    // to the matches, to pick another one with the arrows.
    // Nothing on the soundboard: Enter asks MyInstants, like the button on the empty page.
    search.connect_activate({
        let (st, first, ask_online) = (st.clone(), first.clone(), ask_online.clone());
        move |e| {
            let id = first.borrow().clone();
            match id {
                Some(id) => st.play_sound(&id),
                None if ask_online.is_mapped() && !e.text().trim().is_empty() => ask_online.emit_clicked(),
                None => {}
            }
        }
    });
    let down = gtk::EventControllerKey::new();
    down.connect_key_pressed({
        let (pads, first) = (pads.clone(), first.clone());
        move |_, key, _, _| {
            if !matches!(key, gdk::Key::Down | gdk::Key::KP_Down) {
                return glib::Propagation::Proceed;
            }
            let child = first.borrow().as_ref().and_then(|id| pads.borrow().get(id).map(|p| p.child.clone()));
            match child {
                Some(c) => {
                    c.grab_focus();
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            }
        }
    });
    search.add_controller(down);

    // Delete removes the focused sound, Alt with the arrows moves it. Only when the pad itself has
    // the focus: keys in its menu, like Delete in the name field, belong to the menu.
    // Ctrl+A picks all sounds on the grid, and Delete removes the picked ones.
    let pick_all: Rc<dyn Fn()> = Rc::new({
        let (grid, pads, picks, sync_picks) = (grid.clone(), pads.clone(), picks.clone(), sync_picks.clone());
        move || {
            let mut child = grid.first_child();
            while let Some(c) = child {
                if c.is_visible()
                    && let Some(id) = pads
                        .borrow()
                        .iter()
                        .find(|(_, p)| p.child.upcast_ref::<gtk::Widget>() == &c)
                        .map(|(id, _)| id.clone())
                {
                    picks.ids.borrow_mut().insert(id);
                }
                child = c.next_sibling();
            }
            sync_picks();
        }
    });
    let delete_picks: Rc<dyn Fn()> = Rc::new({
        let (st, picks, sync_picks) = (st.clone(), picks.clone(), sync_picks.clone());
        move || {
            let ids = picks.list();
            picks.clear();
            sync_picks();
            st.remove_sounds(&ids);
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let (st, grid, pads, picks) = (st.clone(), grid.clone(), pads.clone(), picks.clone());
        let (pick_all, delete_picks) = (pick_all.clone(), delete_picks.clone());
        move |_, key, _, mods| {
            let Some(child) = grid.focus_child().filter(|c| c.has_focus()) else { return glib::Propagation::Proceed };
            let Some(id) = pads.borrow().iter().find(|(_, p)| p.child == child).map(|(id, _)| id.clone()) else {
                return glib::Propagation::Proceed;
            };
            // Sorted by name, moving would only change an order you do not see.
            let alt = mods.contains(gdk::ModifierType::ALT_MASK) && !st.config.borrow().sounds_by_name;
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            match key {
                gdk::Key::a if ctrl => pick_all(),
                gdk::Key::Delete | gdk::Key::KP_Delete if !picks.is_empty() => delete_picks(),
                gdk::Key::Delete | gdk::Key::KP_Delete => st.remove_sound(&id),
                gdk::Key::Left | gdk::Key::KP_Left if alt => st.shift_sound(&id, false),
                gdk::Key::Right | gdk::Key::KP_Right if alt => st.shift_sound(&id, true),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });
    grid.add_controller(keys);

    // While sounds are picked, a click or Enter picks too, instead of playing.
    grid.connect_child_activated({
        let (st, pads, picks, sync_picks) = (st.clone(), pads.clone(), picks.clone(), sync_picks.clone());
        let (tiles, open_folder) = (tiles.clone(), open_folder.clone());
        move |_, child| {
            let folder = tiles.borrow().iter().find(|(c, _)| c == child).map(|(_, n)| n.clone());
            if let Some(folder) = folder {
                open_folder(Some(folder));
                return;
            }
            let id = pads.borrow().iter().find(|(_, p)| &p.child == child).map(|(id, _)| id.clone());
            let Some(id) = id else { return };
            if picks.is_empty() {
                st.play_sound(&id);
            } else {
                picks.toggle(&id);
                sync_picks();
            }
        }
    });
    board::pick_clicks(&grid, &item_of, &picks, &sync_picks);
    let can_order: Rc<dyn Fn() -> bool> = Rc::new({
        let st = st.clone();
        // Sorted by name, a place in the order means nothing, but a folder still does.
        move || !st.config.borrow().sounds_by_name
    });
    board::drop_on_grid(
        &shelf,
        &stack,
        &grid,
        &item_of,
        &shown,
        &picks,
        &sync_picks,
        can_order,
        Rc::new(|| true),
        &landing,
    );
    board::drop_folders_on_grid(&shelf, &stack, &grid, &tiles, &landing);
    board::up_takes_drops(&path.up, &shelf, &picks, &sync_picks, &spring_to);
    path.up.connect_clicked({
        let open_folder = open_folder.clone();
        move |_| open_folder(None)
    });
    board::mouse_back(&view, &shown, &open_folder);

    // A right click on free space: things for the whole board. Pads have their own menu.
    popup::context(&scroll, {
        let (st, scroll, add, find) = (st.clone(), scroll.downgrade(), add.clone(), find.clone());
        let (picks, sync_picks, pick_all, shelf) = (picks.clone(), sync_picks.clone(), pick_all.clone(), shelf.clone());
        let (grid, item_of) = (grid.clone(), item_of.clone());
        move |x, y| {
            let Some(scroll) = scroll.upgrade() else { return };
            let (add, find, pick_all) = (add.clone(), find.clone(), pick_all.clone());
            let mut items = vec![board::new_folder_item(&shelf, scroll.upcast_ref())];
            let all = picks.all_of(&board::shown_items(&grid, &item_of));
            if !all {
                items.push(popup::Item::new("edit-select-all-symbolic", t("Select all"), move || pick_all()));
            }
            if !picks.is_empty() {
                let (picks, sync_picks) = (picks.clone(), sync_picks.clone());
                items.push(popup::Item::new("edit-clear-symbolic", t("Deselect all"), move || {
                    picks.clear();
                    sync_picks();
                }));
            }
            items.push(popup::Item::new("list-add-symbolic", t("Add audio files"), move || add()));
            items.push(popup::Item::new("system-search-symbolic", t("Find sounds"), move || find.set_active(true)));
            items.push(sort_item(&st));
            if !st.playing_sounds().is_empty() {
                let st = st.clone();
                items.push(popup::Item::new("media-playback-stop-symbolic", t("Stop all"), move || st.stop_sounds()));
            }
            let st = st.clone();
            items.push(popup::Item::new("edit-undo-symbolic", t("Bring back the starter sounds"), move || {
                st.restore_starter_sounds()
            }));
            popup::show_list(&scroll, x, y, items);
        }
    });

    done.connect_clicked({
        let (picks, sync_picks) = (picks.clone(), sync_picks.clone());
        move |_| {
            picks.clear();
            sync_picks();
        }
    });
    delete.connect_clicked({
        let delete_picks = delete_picks.clone();
        move |_| delete_picks()
    });
    // The button in the bar puts all down again when all are picked.
    select_all.connect_clicked({
        let (pick_all, picks, sync_picks) = (pick_all.clone(), picks.clone(), sync_picks.clone());
        let (grid, item_of) = (grid.clone(), item_of.clone());
        move |_| {
            if picks.all_of(&board::shown_items(&grid, &item_of)) {
                picks.clear();
                sync_picks();
            } else {
                pick_all();
            }
        }
    });
    board::move_to_menu(&move_to, &shelf, &picks, &sync_picks, &landing);

    // Progress rings, once per frame while the page is on screen.
    view.add_tick_callback({
        let st = st.clone();
        let pads = pads.clone();
        let stop = stop.clone();
        move |w, _| {
            if !w.is_mapped() {
                return glib::ControlFlow::Continue;
            }
            let playing = st.engine.meters.playing();
            let mut now: HashMap<String, f32> = HashMap::new();
            for (rt, p) in playing {
                if let Some(id) = st.sound_id_of(rt) {
                    let e = now.entry(id).or_insert(0.0);
                    *e = e.max(p.max(0.001));
                }
            }
            // Stop all wakes up red while something plays.
            let on = !now.is_empty();
            if stop.is_sensitive() != on {
                stop.set_sensitive(on);
                if on {
                    stop.add_css_class("destructive-action");
                } else {
                    stop.remove_css_class("destructive-action");
                }
            }
            for (id, p) in pads.borrow().iter() {
                let v = now.get(id).copied().unwrap_or(0.0);
                if v > 0.0 {
                    p.pad.add_css_class("playing");
                } else {
                    p.pad.remove_css_class("playing");
                }
                if (p.progress.get() - v).abs() > 1e-4 {
                    p.progress.set(v);
                    p.ring.queue_draw();
                }
            }
            glib::ControlFlow::Continue
        }
    });

    let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    drop.connect_drop({
        let st = st.clone();
        let body = body.clone();
        move |_, value, _, _| {
            body.remove_css_class("vn-drop");
            let Ok(list) = value.get::<gdk::FileList>() else { return false };
            let paths: Vec<_> = list.files().iter().filter_map(|f| f.path()).collect();
            import(&st, &paths);
            true
        }
    });
    drop.connect_enter({
        let body = body.clone();
        move |_, _, _| {
            body.add_css_class("vn-drop");
            gdk::DragAction::COPY
        }
    });
    drop.connect_leave({
        let body = body.clone();
        move |_| body.remove_css_class("vn-drop")
    });
    view.add_controller(drop);

    // Escape stops all sounds, when nothing else wants it.
    // With sounds picked, it puts them down first.
    // Backspace or Alt+Left goes back up out of a folder.
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed({
        let (st, picks, sync_picks) = (st.clone(), picks.clone(), sync_picks.clone());
        let (shown, open_folder, search) = (shown.clone(), open_folder.clone(), search.clone());
        move |_, key, _, mods| {
            let alt_left = key == gdk::Key::Left && mods == gdk::ModifierType::ALT_MASK;
            let backspace = key == gdk::Key::BackSpace && mods.is_empty() && search.text().is_empty();
            if (alt_left || backspace) && shown.borrow().is_some() {
                open_folder(None);
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Escape && mods.is_empty() {
                if picks.is_empty() {
                    st.stop_sounds();
                } else {
                    picks.clear();
                    sync_picks();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    view.add_controller(escape);
    // Typing anywhere searches your sounds.
    search.set_key_capture_widget(Some(&view));

    // Many changes in a row, like a lot of new files, build the pads only once.
    let queued = Rc::new(Cell::new(false));
    st.subscribe({
        let (st, pads) = (st.clone(), pads.clone());
        move |c| match c {
            Change::Sounds if !queued.replace(true) => {
                let (rebuild, queued) = (rebuild.clone(), queued.clone());
                glib::idle_add_local_once(move || {
                    queued.set(false);
                    rebuild(false);
                });
            }
            Change::Settings => {
                for (id, p) in pads.borrow().iter() {
                    show_keys(&st, &p.keys, id);
                }
            }
            _ => {}
        }
    });
    let find_online: Rc<dyn Fn(&str)> = Rc::new({
        let (find, online) = (find.clone(), online.clone());
        move |text: &str| {
            find.set_active(true);
            online.set_text(text);
        }
    });
    // The first visit shows what nothing on screen tells: the right click, dragging, the deck.
    super::tour::on_first_visit(st, "soundboard", &view, {
        let (find, scroll, grid, deck) =
            (find.downgrade(), scroll.downgrade(), grid.downgrade(), deck_widget.downgrade());
        move || {
            let (Some(find), Some(scroll), Some(grid), Some(deck)) =
                (find.upgrade(), scroll.upgrade(), grid.upgrade(), deck.upgrade())
            else {
                return Vec::new();
            };
            let mut tips = vec![tour::Tip {
                at: find.upcast(),
                area: None,
                title: t("Find sounds"),
                text: t("Search MyInstants for any sound and add it with one click."),
            }];
            tips.push(tour::Tip {
                at: grid.clone().upcast(),
                area: Some(board::spot_finder(&grid, &scroll)),
                title: t("Right-click an empty spot"),
                text: t("There you make folders, add your own audio files, sort the sounds or pick all. Right-click a sound to edit it."),
            });
            if let Some(first) = grid.child_at_index(0) {
                tips.push(tour::Tip {
                    at: first.upcast(),
                    area: None,
                    title: t("Drag to sort"),
                    text: t("Drag a sound to another place, or onto a folder to put it in. Ctrl+click picks several."),
                });
            }
            tips.push(tour::Tip {
                at: deck.upcast(),
                area: None,
                title: t("Volume and play mode"),
                text: t("Sounds play into the Voicenux Microphone, so others hear them too. Here you set how loud they are, if you hear them yourself, and what a click does."),
            });
            tips
        }
    });
    SoundboardPage { widget: view, search, add, shrink: vec![find_content], find_online }
}

/// A note while Discord hears the Voicenux Microphone: Discord filters out sounds that are not
/// speech, so friends may not hear the soundboard. It goes away for good once read.
fn discord_banner(st: &St) -> adw::Banner {
    let banner =
        adw::Banner::builder().title(t("Discord may filter out your sounds")).button_label(t("How to fix")).build();
    banner.add_css_class("vn-hint");
    banner.connect_button_clicked({
        let st = st.clone();
        move |b| discord_help(&st, b)
    });
    let sync = {
        let (st, banner) = (Rc::downgrade(st), banner.downgrade());
        move || {
            if let (Some(st), Some(b)) = (st.upgrade(), banner.upgrade()) {
                b.set_revealed(st.discord_listens() && !st.config.borrow().discord_tip);
            }
        }
    };
    sync();
    st.subscribe(move |c| {
        if matches!(c, Change::Users | Change::Settings) {
            sync();
        }
    });
    banner
}

/// What to change in Discord so others hear the sounds.
pub fn discord_help(st: &St, parent: &impl IsA<gtk::Widget>) {
    let dialog = adw::AlertDialog::new(
        Some(t("Sounds in Discord")),
        Some(t(
            "Discord removes sounds that are not speech, so your friends may not hear the soundboard. In Discord, open Settings, then Voice & Video:\n\n• Set Noise Suppression to None.\n• Turn off Advanced Voice Activity, or use Push to Talk.",
        )),
    );
    dialog.add_response("ok", t("Got it"));
    dialog.connect_response(None, {
        let st = st.clone();
        move |_, _| {
            if !st.config.borrow().discord_tip {
                st.update(|c| c.discord_tip = true);
            }
        }
    });
    dialog.present(Some(parent));
}

/// Sort by name, or back to your own order when it is sorted.
fn sort_item(st: &St) -> popup::Item {
    let st = st.clone();
    if st.config.borrow().sounds_by_name {
        popup::Item::new("view-list-symbolic", t("Your own order"), move || st.set_sounds_by_name(false))
    } else {
        popup::Item::new("view-sort-ascending-symbolic", t("Sort by name"), move || st.set_sounds_by_name(true))
    }
}

fn import(st: &St, paths: &[std::path::PathBuf]) {
    st.import_sounds(paths.to_vec());
}

/// Objects with a text, and the name of their text property.
type Texts = Vec<(glib::Object, &'static str)>;

/// The deck under the sounds: how loud they play, whether you hear them yourself, what a click
/// does, and a way to stop them all. Gives the texts that go when the page is narrow, as
/// objects and their text property, the labels that go then, the volume slider and the stop
/// button.
fn deck(st: &St) -> (gtk::Box, Texts, Vec<gtk::Widget>, gtk::Box, gtk::Button) {
    let mut texts: Texts = Vec::new();
    // The volume is what everyone hears: others through the Voicenux Microphone, and you too
    // when Hear them myself is on. Both say so, so nobody wonders whose volume it is.
    let everyone = label(t("For everyone"), &["vn-deck-label"]);
    everyone.set_valign(gtk::Align::Center);
    let volume = volume_slider(st.config.borrow().sound_volume as f64, 1.0, {
        let st = st.clone();
        move |v| st.update(|c| c.sound_volume = v as f32)
    });
    let tip = t(
        "How loud the sounds play for everyone: for others in the Voicenux Microphone, and for you when Hear them myself is on.",
    );
    everyone.set_tooltip_text(Some(tip));
    volume.set_tooltip_text(Some(tip));
    volume.set_width_request(190);
    volume.set_hexpand(false);
    volume.set_valign(gtk::Align::Center);

    let hear_text = label(t("Hear them myself"), &[]);
    let hear_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    hear_content.set_halign(gtk::Align::Center);
    hear_content.append(&line_icon("headphones", 16));
    hear_content.append(&hear_text);
    let hear = gtk::ToggleButton::builder().child(&hear_content).valign(gtk::Align::Center).build();
    hear.add_css_class("flat");
    hear.add_css_class("vn-hear");
    hear.set_active(st.config.borrow().hear_sounds);
    hear.set_tooltip_text(Some(t("Hear sounds myself")));
    hear.connect_toggled({
        let st = st.clone();
        move |b| {
            if b.is_active() != st.config.borrow().hear_sounds {
                st.update(|c| c.hear_sounds = b.is_active());
            }
        }
    });

    // What a click on a sound does, all choices in sight.
    let modes = adw::ToggleGroup::new();
    modes.add_css_class("vn-segmented");
    modes.set_valign(gtk::Align::Center);
    // Each as wide as its text, so they fit beside the slider.
    modes.set_homogeneous(false);
    for m in PlayMode::ALL {
        let toggle = adw::Toggle::builder()
            .name(m.id())
            .icon_name(m.icon())
            .label(t_dyn(m.short()))
            .tooltip(t_dyn(m.label()))
            .build();
        texts.push((toggle.clone().upcast(), "label"));
        modes.add(toggle);
    }
    modes.set_active_name(Some(st.config.borrow().sound_mode.id()));
    modes.connect_active_name_notify({
        let st = st.clone();
        move |g| {
            if let Some(m) = g.active_name().and_then(|n| PlayMode::from_id(&n))
                && m != st.config.borrow().sound_mode
            {
                st.set_sound_mode(m);
            }
        }
    });
    st.subscribe({
        let (st, modes, hear) = (st.clone(), modes.downgrade(), hear.downgrade());
        move |c| {
            if !matches!(c, Change::Settings) {
                return;
            }
            // Copy first: setting a widget runs its handler, and that writes the config.
            let (mode, hear_sounds) = {
                let c = st.config.borrow();
                (c.sound_mode, c.hear_sounds)
            };
            if let Some(modes) = modes.upgrade()
                && modes.active_name().as_deref() != Some(mode.id())
            {
                modes.set_active_name(Some(mode.id()));
            }
            if let Some(hear) = hear.upgrade()
                && hear.is_active() != hear_sounds
            {
                hear.set_active(hear_sounds);
            }
        }
    });

    let stop_content =
        adw::ButtonContent::builder().icon_name("media-playback-stop-symbolic").label(t("Stop all")).build();
    texts.push((stop_content.clone().upcast(), "label"));
    let stop = gtk::Button::builder().child(&stop_content).valign(gtk::Align::Center).sensitive(false).build();
    stop.add_css_class("vn-stop-all");
    stop.set_tooltip_text(Some(t("Stop all sounds (Esc)")));
    stop.connect_clicked({
        let st = st.clone();
        move |_| st.stop_sounds()
    });

    let gap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    gap.set_hexpand(true);
    let deck = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    deck.add_css_class("vn-deck");
    deck.append(&everyone);
    deck.append(&volume);
    deck.append(&hear);
    deck.append(&gap);
    deck.append(&modes);
    deck.append(&stop);
    (deck, texts, vec![everyone.upcast(), hear_text.upcast()], volume, stop)
}

/// A volume slider with a speaker that shows the level, and the value in percent. Click the value
/// to type one. It holds at 50 %, 100 % and 150 % on the way, so those are easy to hit.
pub(super) fn volume_slider(value: f64, max: f64, on_change: impl Fn(f64) + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("vn-volume");
    let speaker = gtk::Image::new();
    speaker.add_css_class("vn-volume-icon");
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, max, 0.05);
    scale.set_hexpand(true);
    scale.set_value(value);
    super::notches(&scale, &[0.5, 1.0, 1.5]);
    super::wheel_slider(&scale, super::grid(0.05));
    let text = |v: f64| format!("{:.0} %", v * 100.0);
    let field = value_field(
        {
            let scale = scale.clone();
            move || format!("{:.0}", scale.value() * 100.0)
        },
        {
            let scale = scale.clone();
            move |f, typed_text| {
                if let Some(n) = super::typed_number(typed_text, &["%"]) {
                    scale.set_value((n / 100.0).clamp(0.0, max));
                }
                // Also puts the old value back after something that is not a number.
                f.set_text(&text(scale.value()));
            }
        },
    );
    field.add_css_class("vn-volume-value");
    field.set_width_chars(5);
    field.set_max_width_chars(5);
    let show = {
        let (speaker, field) = (speaker.clone(), field.clone());
        move |v: f64| {
            speaker.set_icon_name(Some(match v {
                v if v < 0.005 => "audio-volume-muted-symbolic",
                v if v < 0.4 => "audio-volume-low-symbolic",
                v if v < 0.8 => "audio-volume-medium-symbolic",
                _ => "audio-volume-high-symbolic",
            }));
            field.set_text(&text(v));
        }
    };
    show(value);
    scale.connect_value_changed(move |s| {
        let v = s.value();
        show(v);
        on_change(v);
    });
    row.append(&speaker);
    row.append(&scale);
    row.append(&field);
    row
}

fn pad(st: &St, s: &Sound, landing: &Rc<board::Landing>) -> Pad {
    let pad = gtk::Box::new(gtk::Orientation::Vertical, 8);
    pad.add_css_class("sound-pad");
    pad.set_size_request(136, 140);

    let progress = Rc::new(Cell::new(0.0f32));
    let ring = progress_ring(progress.clone());
    let pic = gtk::Overlay::new();
    pic.set_child(Some(&ring));
    let img = icon(&s.icon, 58);
    img.add_css_class("vn-pad-icon");
    img.set_halign(gtk::Align::Center);
    img.set_valign(gtk::Align::Center);
    pic.add_overlay(&img);
    pic.set_halign(gtk::Align::Center);
    pad.append(&pic);
    let name = label(&t_dyn(&s.name), &["vn-pad-name"]);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_max_width_chars(14);
    pad.append(&name);
    let keys = label("", &["vn-key", "vn-pad-keys"]);
    keys.set_halign(gtk::Align::Center);
    keys.set_ellipsize(gtk::pango::EllipsizeMode::End);
    keys.set_max_width_chars(16);
    show_keys(st, &keys, &s.id);
    pad.append(&keys);

    // A little mark when it is cut.
    let badges = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    badges.set_halign(gtk::Align::Start);
    badges.set_valign(gtk::Align::Start);
    badges.set_margin_top(8);
    badges.set_margin_start(8);
    if s.is_trimmed() {
        let img = gtk::Image::from_icon_name("edit-cut-symbolic");
        img.add_css_class("vn-pad-badge");
        img.set_tooltip_text(Some(t("Trimmed")));
        badges.append(&img);
    }

    let menu = gtk::Button::from_icon_name("view-more-symbolic");
    menu.add_css_class("flat");
    menu.add_css_class("vn-pad-menu");
    menu.set_halign(gtk::Align::End);
    menu.set_valign(gtk::Align::Start);
    menu.set_margin_top(6);
    menu.set_margin_end(6);
    menu.set_tooltip_text(Some(t("Edit sound")));

    // The star, next to the menu. Starred sounds come first.
    let star = gtk::Button::from_icon_name(if s.favorite { "starred-symbolic" } else { "non-starred-symbolic" });
    star.add_css_class("flat");
    star.add_css_class("vn-star");
    if s.favorite {
        // The pad bounces into its new place instead: the star does not pop each time the
        // soundboard builds again.
        star.add_css_class("on");
        star.add_css_class("vn-still");
    }
    star.set_halign(gtk::Align::End);
    star.set_valign(gtk::Align::Start);
    star.set_margin_top(5);
    star.set_margin_end(32);
    star.set_tooltip_text(Some(t("Favorite")));
    star.connect_clicked({
        let (st, id, landing) = (st.clone(), s.id.clone(), landing.clone());
        move |_| {
            landing.items.borrow_mut().insert(id.clone());
            st.toggle_sound_favorite(&id);
        }
    });

    let overlay = gtk::Overlay::new();
    overlay.add_css_class("vn-lift");
    overlay.set_child(Some(&pad));
    overlay.add_overlay(&menu);
    overlay.add_overlay(&star);
    overlay.add_overlay(&badges);
    let check = gtk::Image::from_icon_name("object-select-symbolic");
    check.add_css_class("vn-pad-check");
    check.set_halign(gtk::Align::Start);
    check.set_valign(gtk::Align::Start);
    check.set_margin_top(8);
    check.set_margin_start(8);
    check.set_visible(false);
    check.set_can_target(false);
    overlay.add_overlay(&check);
    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&overlay));
    super::tile_button(&menu, &child);
    super::tile_button(&star, &child);

    // The editor is made when it first opens: the grid builds again after every change, and most
    // pads never open it. It hangs on the grid cell, which never lifts or squishes.
    let made: Rc<std::cell::OnceCell<gtk::Popover>> = Rc::default();
    let open_editor = {
        let (st, s, child) = (st.clone(), s.clone(), child.downgrade());
        Rc::new(move |at: gdk::Rectangle| {
            let Some(child) = child.upgrade() else { return };
            let e = made.get_or_init(|| {
                let e = editor(&st, &s);
                e.set_parent(&child);
                popup::keep_clicks(&e);
                child.connect_destroy({
                    let e = e.clone();
                    move |_| e.unparent()
                });
                e
            });
            e.set_pointing_to(Some(&at));
            e.popup();
        })
    };
    menu.connect_clicked({
        let (open_editor, child) = (open_editor.clone(), child.downgrade());
        move |b| {
            let Some(r) = child.upgrade().and_then(|c| b.compute_bounds(&c)) else { return };
            open_editor(gdk::Rectangle::new(r.x() as i32, r.y() as i32, r.width() as i32, r.height() as i32));
        }
    });
    popup::context(&child, move |x, y| open_editor(gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    Pad { child, pad, progress, ring, keys, badges, check }
}

/// Shows the shortcut of a sound on its pad, or nothing.
fn show_keys(st: &St, keys: &gtk::Label, id: &str) {
    let shown = super::keys::shown(st, &format!("sound:{id}"));
    keys.set_label(shown.as_deref().unwrap_or(""));
    keys.set_visible(shown.is_some());
}

/// Popover to rename a sound, change its picture, volume and how it plays.
fn editor(st: &St, s: &Sound) -> gtk::Popover {
    let sound = Rc::new(RefCell::new(s.clone()));
    let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
    b.set_margin_top(12);
    b.set_margin_bottom(12);
    b.set_margin_start(12);
    b.set_margin_end(12);
    b.set_width_request(280);

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::new();
    entry.set_text(&t_dyn(&s.name));
    entry.set_hexpand(true);
    entry.set_valign(gtk::Align::Center);
    let save = {
        let st = st.clone();
        let sound = sound.clone();
        move || st.update_sound(sound.borrow().clone())
    };
    let save = Rc::new(save);
    top.append(&icon_picker(&s.icon, 32, {
        let sound = sound.clone();
        let save = save.clone();
        move |name| {
            sound.borrow_mut().icon = name.to_string();
            save();
        }
    }));
    top.append(&entry);
    entry.connect_activate({
        let sound = sound.clone();
        let save = save.clone();
        move |e| {
            sound.borrow_mut().name = e.text().to_string();
            save();
        }
    });
    b.append(&top);

    // Only this sound, on top of the volume for everyone under the sounds.
    let vol_label = label(t("Volume of this sound"), &["caption-heading"]);
    vol_label.set_xalign(0.0);
    b.append(&vol_label);
    let own = volume_slider(s.volume as f64, 2.0, {
        let st = st.clone();
        let sound = sound.clone();
        move |v| {
            sound.borrow_mut().volume = v as f32;
            st.set_sound_volume(&sound.borrow().id, v as f32);
        }
    });
    own.set_tooltip_text(Some(t("Only this sound. It comes on top of the volume for everyone under the sounds.")));
    b.append(&own);

    let trim = gtk::Button::new();
    trim.add_css_class("pill");
    trim.set_child(Some(&adw::ButtonContent::builder().icon_name("edit-cut-symbolic").label(t("Trim sound")).build()));
    b.append(&trim);

    b.append(&super::keys::button(st, &format!("sound:{}", s.id), &t_dyn(&s.name)));

    let remove = gtk::Button::with_label(t("Remove"));
    remove.add_css_class("destructive-action");
    remove.set_margin_top(4);
    remove.connect_clicked({
        let st = st.clone();
        let id = s.id.clone();
        move |_| st.remove_sound(&id)
    });
    b.append(&remove);

    let p = gtk::Popover::new();
    popup::menu(&p);
    p.set_child(Some(&b));
    trim.connect_clicked({
        let (st, p, id) = (st.clone(), p.downgrade(), s.id.clone());
        move |b| {
            let Some(p) = p.upgrade() else { return };
            p.popdown();
            // On the window: the grid is made again when the cut is saved.
            if let Some(window) = b.root() {
                super::trim::open(&st, &id, &window);
            }
        }
    });
    p.connect_closed({
        let sound = sound.clone();
        let entry = entry.clone();
        let save = save.clone();
        move |_| {
            let name = entry.text().to_string();
            let changed = sound.borrow().name != name;
            if changed && !name.trim().is_empty() {
                sound.borrow_mut().name = name;
                save();
            }
        }
    });
    p
}
