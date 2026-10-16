//! The soundboard page: big pads that play sounds into your microphone.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use super::visuals::progress_ring;
use super::{animate_in, popup, clear, icon, icon_picker, label, smooth_scroll, value_field};
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

/// The sounds picked with Ctrl+click or Shift+click, for Delete and Move. While some are, a click
/// picks instead of playing.
#[derive(Default)]
struct Picks {
    ids: RefCell<HashSet<String>>,
    /// Where Shift+click starts its row of picks.
    anchor: RefCell<Option<String>>,
}

impl Picks {
    fn is_empty(&self) -> bool {
        self.ids.borrow().is_empty()
    }

    fn list(&self) -> Vec<String> {
        self.ids.borrow().iter().cloned().collect()
    }

    fn toggle(&self, id: &str) {
        let mut ids = self.ids.borrow_mut();
        if !ids.remove(id) {
            ids.insert(id.to_string());
        }
        *self.anchor.borrow_mut() = Some(id.to_string());
    }

    fn clear(&self) {
        self.ids.borrow_mut().clear();
        self.anchor.borrow_mut().take();
    }

    /// Every sound on the grid right now is picked.
    fn all_of(&self, pads: &HashMap<String, Pad>) -> bool {
        let ids = self.ids.borrow();
        let mut shown = pads.iter().filter(|(_, p)| p.child.is_visible()).peekable();
        shown.peek().is_some() && shown.all(|(id, _)| ids.contains(id))
    }
}

pub struct SoundboardPage {
    pub widget: adw::ToolbarView,
    pub search: gtk::SearchEntry,
    /// Opens the file chooser to add sounds.
    pub add: Rc<dyn Fn()>,
    /// Header buttons that drop their text in a narrow window.
    pub shrink: Vec<adw::ButtonContent>,
    /// The text of the play mode button, which goes first when the header gets full.
    pub mode_label: gtk::Label,
    /// Opens the MyInstants search with some words.
    pub find_online: Rc<dyn Fn(&str)>,
}

pub fn build(st: &St) -> SoundboardPage {
    let header = adw::HeaderBar::new();
    // The tools sit on the left, in line with the page, and the rest of the bar is for dragging.
    header.set_show_title(false);
    let (search_box, search) = super::search_field(t("Search sounds"));
    search_box.set_margin_start(12);
    header.pack_start(&search_box);
    let content = |icon: &str, text: &str| adw::ButtonContent::builder().icon_name(icon).label(text).build();
    let (find_content, stop_content) =
        (content("system-search-symbolic", t("Find sounds")), content("media-playback-stop-symbolic", t("Stop all")));
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
    let stop = gtk::Button::new();
    stop.set_child(Some(&stop_content));
    stop.set_tooltip_text(Some(t("Stop all sounds (Esc)")));
    header.pack_end(&stop);
    header.pack_end(&more_button(st, add.clone()));
    header.pack_end(&volume_button(st));
    let (mode, mode_label) = mode_button(st);
    header.pack_end(&mode);
    stop.connect_clicked({
        let st = st.clone();
        move |_| st.stop_sounds()
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

    let hint = label(
        t("Sounds play into the Voicenux Microphone. Drop audio files anywhere on this page to add them."),
        &["dim-label", "caption"],
    );
    hint.set_wrap(true);
    hint.set_margin_bottom(10);
    hint.set_margin_start(20);
    hint.set_margin_end(20);

    // In a folder: the way back up over the grid. Drop a sound on it to take it out.
    let up = gtk::Button::new();
    up.set_child(Some(&adw::ButtonContent::builder().icon_name("go-previous-symbolic").label(t("All sounds")).build()));
    up.add_css_class("flat");
    up.add_css_class("vn-up");
    up.set_tooltip_text(Some(t("Back to all sounds (Backspace)")));
    let here = label("", &["vn-folder-title"]);
    here.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let path = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    path.append(&up);
    path.append(&label("›", &["dim-label"]));
    path.append(&gtk::Image::from_icon_name("folder-symbolic"));
    path.append(&here);
    path.set_margin_start(16);
    path.set_margin_end(20);
    path.set_margin_top(10);
    path.set_visible(false);
    // And the bar for picked sounds under the grid.
    let picks = Rc::new(Picks::default());
    let count = label("", &["heading"]);
    let bar = gtk::ActionBar::new();
    bar.add_css_class("vn-picks-bar");
    bar.pack_start(&count);
    let done = gtk::Button::with_label(t("Done"));
    done.set_tooltip_text(Some(t("Stop selecting (Esc)")));
    let delete = gtk::Button::with_label(t("Delete"));
    delete.add_css_class("destructive-action");
    let move_to = gtk::MenuButton::new();
    move_to.set_label(t("Move to folder"));
    move_to.set_always_show_arrow(true);
    let select_all = gtk::Button::with_label(t("Select all"));
    select_all.set_tooltip_text(Some(&super::with_key(t("Select all"), "A")));
    bar.pack_end(&done);
    bar.pack_end(&delete);
    bar.pack_end(&move_to);
    bar.pack_end(&select_all);
    let picks_reveal = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .transition_duration(200)
        .child(&bar)
        .build();

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.set_hexpand(true);
    body.append(&path);
    body.append(&stack);
    body.append(&picks_reveal);
    body.append(&hint);

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
    content.append(&body);
    content.append(&reveal);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&content));

    let pads: Rc<RefCell<HashMap<String, Pad>>> = Rc::default();
    // The first pad that matches the search, top left: Enter in the search plays it.
    let first: Rc<RefCell<Option<String>>> = Rc::default();
    // The folder the grid shows, None for all sounds. Folders sit on the grid like pads.
    let shown: Rc<RefCell<Option<String>>> = Rc::default();
    let tiles: Rc<RefCell<Vec<(gtk::FlowBoxChild, String)>>> = Rc::default();

    let sync_picks: Rc<dyn Fn()> = Rc::new({
        let (pads, picks, picks_reveal, count) = (pads.clone(), picks.clone(), picks_reveal.clone(), count.clone());
        let select_all = select_all.clone();
        move || {
            let ids = picks.ids.borrow();
            select_all.set_label(if picks.all_of(&pads.borrow()) { t("Deselect all") } else { t("Select all") });
            for (id, p) in pads.borrow().iter() {
                let on = ids.contains(id);
                if on {
                    p.pad.add_css_class("vn-selected");
                } else {
                    p.pad.remove_css_class("vn-selected");
                }
                p.check.set_visible(on);
                p.badges.set_visible(!on);
            }
            picks_reveal.set_reveal_child(!ids.is_empty());
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
                none.set_description(Some(t("Drag sounds onto the folder, or pick some with Ctrl+click and move them here.")));
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
        let (shown, path, here) = (shown.clone(), path.clone(), here.clone());
        move || {
            let folder = shown.borrow().clone();
            path.set_visible(folder.is_some());
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
        let (shown, tiles, open_folder, spring_to) = (shown.clone(), tiles.clone(), open_folder.clone(), spring_to.clone());
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
                let inside: Vec<&Sound> = sounds.iter().filter(|s| s.folder.as_deref() == Some(name.as_str())).collect();
                let icons: Vec<String> = inside.iter().take(3).map(|s| s.icon.clone()).collect();
                let child = folder_tile(&name, &icons, inside.len());
                folder_actions(&st, &child, &name, &picks, &sync_picks, &open_folder, &spring_to);
                grid.append(&child);
                tiles.borrow_mut().push((child, name));
            }
            for (i, s) in sounds.iter().enumerate() {
                let p = pad(&st, s);
                drag_source(&p.child, &p.pad, &s.id, &picks, &pads);
                grid.append(&p.child);
                if animate {
                    animate_in(&p.child, i);
                }
                pads.borrow_mut().insert(s.id.clone(), p);
            }
            picks.ids.borrow_mut().retain(|id| pads.borrow().contains_key(id));
            refilter();
            sync_picks();
            if let Some((id, index)) = focused {
                let child = pads.borrow().get(&id).map(|p| p.child.clone()).or_else(|| grid.child_at_index(index.min(sounds.len() as i32 - 1)));
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
                    && let Some(id) = pads.borrow().iter().find(|(_, p)| p.child.upcast_ref::<gtk::Widget>() == &c).map(|(id, _)| id.clone())
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
    // Ctrl+click picks a sound, Shift+click all sounds from the last pick to this one, like in a
    // file manager.
    let pick_click = gtk::GestureClick::new();
    pick_click.set_propagation_phase(gtk::PropagationPhase::Capture);
    pick_click.connect_pressed({
        let (grid, pads, picks, sync_picks) = (grid.downgrade(), pads.clone(), picks.clone(), sync_picks.clone());
        move |g, _, x, y| {
            let mods = g.current_event_state();
            let (ctrl, shift) =
                (mods.contains(gdk::ModifierType::CONTROL_MASK), mods.contains(gdk::ModifierType::SHIFT_MASK));
            let Some(grid) = grid.upgrade() else { return };
            let Some(child) = (ctrl || shift).then(|| grid.child_at_pos(x as i32, y as i32)).flatten() else { return };
            let id_of = |c: &gtk::Widget| pads.borrow().iter().find(|(_, p)| p.child.upcast_ref::<gtk::Widget>() == c).map(|(id, _)| id.clone());
            let Some(id) = id_of(child.upcast_ref()) else { return };
            g.set_state(gtk::EventSequenceState::Claimed);
            let anchor = picks.anchor.borrow().clone();
            match anchor.filter(|_| shift) {
                Some(anchor) => {
                    let (mut on, mut row) = (false, Vec::new());
                    let mut c = grid.first_child();
                    while let Some(w) = c {
                        if w.is_visible()
                            && let Some(i) = id_of(&w)
                        {
                            let edge = i == anchor || i == id;
                            if edge || on {
                                row.push(i);
                            }
                            if edge && anchor != id {
                                on = !on;
                            }
                        }
                        c = w.next_sibling();
                    }
                    picks.ids.borrow_mut().extend(row);
                }
                None => picks.toggle(&id),
            }
            sync_picks();
        }
    });
    grid.add_controller(pick_click);
    drop_on_grid(st, &stack, &grid, &pads, &shown, &picks, &sync_picks);

    up.connect_clicked({
        let open_folder = open_folder.clone();
        move |_| open_folder(None)
    });
    // Held over it for a moment, a sound goes back up with you, to be dropped elsewhere.
    let out = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
    let spring = Spring::default();
    out.connect_enter({
        let (spring, spring_to, up) = (spring.clone(), spring_to.clone(), up.downgrade());
        move |_, _, _| {
            if let Some(up) = up.upgrade() {
                up.add_css_class("vn-drop-into");
            }
            let (spring_to, up) = (spring_to.clone(), up.clone());
            spring.arm(move || {
                if let Some(up) = up.upgrade() {
                    up.remove_css_class("vn-drop-into");
                }
                spring_to(None);
            });
            gdk::DragAction::MOVE
        }
    });
    out.connect_leave({
        let (spring, up) = (spring.clone(), up.downgrade());
        move |_| {
            spring.cancel();
            if let Some(up) = up.upgrade() {
                up.remove_css_class("vn-drop-into");
            }
        }
    });
    out.connect_drop({
        let (st, picks, sync_picks, up) = (st.clone(), picks.clone(), sync_picks.clone(), up.downgrade());
        move |_, value, _, _| {
            spring.cancel();
            if let Some(up) = up.upgrade() {
                up.remove_css_class("vn-drop-into");
            }
            let Ok(id) = value.get::<String>() else { return false };
            let ids = take_dragged(&picks, &sync_picks, id);
            let st = st.clone();
            glib::idle_add_local_once(move || st.move_to_folder(&ids, None));
            true
        }
    });
    up.add_controller(out);
    // The back button of the mouse goes up too, like in a file manager.
    let back = gtk::GestureClick::new();
    back.set_button(8);
    back.connect_pressed({
        let (shown, open_folder) = (shown.clone(), open_folder.clone());
        move |g, _, _, _| {
            if shown.borrow().is_some() {
                g.set_state(gtk::EventSequenceState::Claimed);
                open_folder(None);
            }
        }
    });
    view.add_controller(back);

    // A right click on free space: things for the whole board. Pads have their own menu.
    popup::context(&scroll, {
        let (st, scroll, add, find) = (st.clone(), scroll.downgrade(), add.clone(), find.clone());
        let (picks, sync_picks, pick_all, pads) = (picks.clone(), sync_picks.clone(), pick_all.clone(), pads.clone());
        move |x, y| {
            let Some(scroll) = scroll.upgrade() else { return };
            let (add, find, pick_all) = (add.clone(), find.clone(), pick_all.clone());
            let mut items = vec![new_folder_item(&st, scroll.upcast_ref())];
            let all = picks.all_of(&pads.borrow());
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
        let (pick_all, picks, pads, sync_picks) = (pick_all.clone(), picks.clone(), pads.clone(), sync_picks.clone());
        move |_| {
            if picks.all_of(&pads.borrow()) {
                picks.clear();
                sync_picks();
            } else {
                pick_all();
            }
        }
    });
    move_to.set_create_popup_func({
        let (st, picks, sync_picks) = (st.clone(), picks.clone(), sync_picks.clone());
        move |b| {
            let folders = st.library.borrow().folders.clone();
            let put = |folder: Option<String>| {
                let (st, picks, sync_picks) = (st.clone(), picks.clone(), sync_picks.clone());
                move || {
                    let ids = picks.list();
                    picks.clear();
                    sync_picks();
                    st.move_to_folder(&ids, folder.as_deref());
                }
            };
            let mut items: Vec<popup::Item> =
                folders.iter().map(|f| popup::Item::new("folder-symbolic", f.clone(), put(Some(f.clone())))).collect();
            if !folders.is_empty() {
                items.push(popup::Item::new("edit-clear-symbolic", t("No folder"), put(None)));
            }
            let (st, picks, sync_picks, parent) = (st.clone(), picks.clone(), sync_picks.clone(), b.downgrade());
            items.push(popup::Item::new("list-add-symbolic", t("New folder"), move || {
                let Some(parent) = parent.upgrade() else { return };
                let (st, picks, sync_picks) = (st.clone(), picks.clone(), sync_picks.clone());
                ask_folder_name(&parent, t("New folder"), "", t("Create"), move |name| {
                    let name = st.add_folder(&name);
                    let ids = picks.list();
                    picks.clear();
                    sync_picks();
                    st.move_to_folder(&ids, Some(&name));
                });
            }));
            b.set_popover(Some(&popup::list(items)));
        }
    });

    // Progress rings, once per frame while the page is on screen.
    view.add_tick_callback({
        let st = st.clone();
        let pads = pads.clone();
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
    SoundboardPage { widget: view, search, add, shrink: vec![find_content, stop_content], mode_label, find_online }
}

/// What a click on a sound does, for all sounds at once. Sounds with a mode of their own keep it.
fn mode_button(st: &St) -> (gtk::MenuButton, gtk::Label) {
    // An icon and a label of its own, not a ButtonContent: a narrow window hides the label, and
    // a new mode only changes its text.
    let (icon, text) = (gtk::Image::new(), gtk::Label::new(None));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&icon);
    content.append(&text);
    let b = gtk::MenuButton::new();
    b.set_child(Some(&content));
    b.set_always_show_arrow(true);
    b.set_tooltip_text(Some(t("What a click on a sound does")));
    let items = PlayMode::BOARD
        .iter()
        .map(|&m| {
            let st = st.clone();
            popup::Item::new(m.icon(), t_dyn(m.label()), move || st.set_sound_mode(m))
        })
        .collect();
    b.set_popover(Some(&popup::list(items)));
    let sync = {
        let (st, icon, text) = (st.clone(), icon.clone(), text.clone());
        move || {
            let m = st.config.borrow().sound_mode;
            icon.set_icon_name(Some(m.icon()));
            text.set_label(&t_dyn(m.label()));
        }
    };
    sync();
    st.subscribe(move |c| {
        if matches!(c, Change::Settings) {
            sync();
        }
    });
    (b, text)
}

/// More: add own files, sort the sounds, or bring back the starter sounds.
fn more_button(st: &St, add: Rc<dyn Fn()>) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    b.set_icon_name("view-more-symbolic");
    b.set_tooltip_text(Some(t("More")));
    // Made new each time it opens: the sort item says what it switches to.
    let st = st.clone();
    b.set_create_popup_func(move |b| {
        let (st2, add) = (st.clone(), add.clone());
        b.set_popover(Some(&popup::list(vec![
            popup::Item::new("list-add-symbolic", t("Add audio files"), move || add()),
            new_folder_item(&st, b.upcast_ref()),
            sort_item(&st),
            popup::Item::new("edit-undo-symbolic", t("Bring back the starter sounds"), move || st2.restore_starter_sounds()),
        ])));
    });
    b
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

/// Asks for a name over `parent`, then makes the folder.
fn new_folder_item(st: &St, parent: &gtk::Widget) -> popup::Item {
    let (st, parent) = (st.clone(), parent.downgrade());
    popup::Item::new("folder-new-symbolic", t("New folder"), move || {
        let Some(parent) = parent.upgrade() else { return };
        let st = st.clone();
        ask_folder_name(&parent, t("New folder"), "", t("Create"), move |name| {
            st.add_folder(&name);
        });
    })
}

fn import(st: &St, paths: &[std::path::PathBuf]) {
    st.import_sounds(paths.to_vec());
}

/// The sounds a drop moves: all picked ones when the dragged one is picked, else just it. The
/// picks are done then.
fn take_dragged(picks: &Picks, sync_picks: &Rc<dyn Fn()>, id: String) -> Vec<String> {
    if !picks.ids.borrow().contains(&id) {
        return vec![id];
    }
    let ids = picks.list();
    picks.clear();
    sync_picks();
    ids
}

/// A folder on the grid, as big as a pad: a glass folder with up to three pictures of its sounds
/// peeking out and showing through its front, and under it the name and how many sounds it has.
fn folder_tile(name: &str, icons: &[String], count: usize) -> gtk::FlowBoxChild {
    let tile = gtk::Box::new(gtk::Orientation::Vertical, 8);
    tile.add_css_class("sound-pad");
    tile.add_css_class("vn-folder");
    tile.set_size_request(136, 140);
    // Drawn back to front, as high as the ring of a pad: the back with its tab, the pictures,
    // then the glass front.
    let art = gtk::Fixed::new();
    art.set_size_request(100, 84);
    art.set_halign(gtk::Align::Center);
    art.set_can_target(false);
    art.put(&folder_glass(100, 84, true), 0.0, 0.0);
    // One picture sits deep in the folder, more of them fan out.
    let spots: &[(f64, f64, &str)] = match icons.len() {
        0 => &[],
        1 => &[(32.0, 24.0, "vn-peek-mid")],
        2 => &[(18.0, 18.0, "vn-peek-left"), (46.0, 18.0, "vn-peek-right")],
        _ => &[(6.0, 20.0, "vn-peek-left"), (32.0, 12.0, "vn-peek-mid"), (58.0, 20.0, "vn-peek-right")],
    };
    for (pic, (x, y, class)) in icons.iter().zip(spots) {
        let img = icon(pic, 36);
        img.add_css_class(class);
        art.put(&img, *x, *y);
    }
    let front = folder_glass(100, 46, false);
    front.add_css_class("vn-folder-front");
    art.put(&front, 0.0, 38.0);
    tile.append(&art);
    let title = label(name, &["vn-pad-name"]);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(14);
    tile.append(&title);
    let n = match count {
        0 => t("Empty").to_string(),
        1 => t("1 sound").to_string(),
        n => fill(t("{} sounds"), &[&n.to_string()]),
    };
    tile.append(&label(&n, &["dim-label", "caption"]));

    let lift = gtk::Overlay::new();
    lift.add_css_class("vn-lift");
    lift.set_child(Some(&tile));
    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&lift));
    child.set_tooltip_text(Some(name));
    child
}

/// A glass part of a folder: the back with its tab as one shape, or the front. White and see
/// through in dark mode, a light blue in light mode, so it shows on the white cards there.
fn folder_glass(w: i32, h: i32, back: bool) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(w);
    area.set_content_height(h);
    area.set_can_target(false);
    area.set_draw_func(move |_, cr, w, h| {
        use std::f64::consts::PI;
        let dark = adw::StyleManager::default().is_dark();
        let (w, h) = (w as f64, h as f64);
        let (l, r, b) = (0.5, w - 0.5, h - 0.5);
        cr.new_path();
        if back {
            // The tab rises at the top left and slopes down into the top edge.
            let (tab, top) = (8.5, 16.5);
            cr.arc(l + 6.0, tab + 6.0, 6.0, PI, 1.5 * PI);
            cr.line_to(34.0, tab);
            cr.curve_to(40.0, tab, 40.0, top, 47.0, top);
            cr.arc(r - 10.0, top + 10.0, 10.0, 1.5 * PI, 2.0 * PI);
        } else {
            cr.arc(l + 10.0, 10.5, 10.0, PI, 1.5 * PI);
            cr.arc(r - 10.0, 10.5, 10.0, 1.5 * PI, 2.0 * PI);
        }
        cr.arc(r - 11.0, b - 11.0, 11.0, 0.0, 0.5 * PI);
        cr.arc(l + 11.0, b - 11.0, 11.0, 0.5 * PI, PI);
        cr.close_path();
        let (fill, edge) = match (dark, back) {
            (true, true) => ((1.0, 1.0, 1.0, 0.12), (1.0, 1.0, 1.0, 0.24)),
            (true, false) => ((1.0, 1.0, 1.0, 0.08), (1.0, 1.0, 1.0, 0.34)),
            (false, true) => ((0.23, 0.51, 0.96, 0.16), (0.23, 0.51, 0.96, 0.3)),
            (false, false) => ((1.0, 1.0, 1.0, 0.45), (0.23, 0.51, 0.96, 0.32)),
        };
        cr.set_source_rgba(fill.0, fill.1, fill.2, fill.3);
        let _ = cr.fill_preserve();
        if !back {
            // Light along the top of the glass.
            let shine = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, h * 0.6);
            shine.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, if dark { 0.14 } else { 0.35 });
            shine.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
            let _ = cr.set_source(&shine);
            let _ = cr.fill_preserve();
        }
        cr.set_line_width(1.0);
        cr.set_source_rgba(edge.0, edge.1, edge.2, edge.3);
        let _ = cr.stroke();
    });
    // The theme can change while it is on screen.
    let weak = area.downgrade();
    adw::StyleManager::default().connect_dark_notify(move |_| {
        if let Some(a) = weak.upgrade() {
            a.queue_draw();
        }
    });
    area
}

/// What a folder on the grid does besides opening: take sounds dropped on it, and a right-click
/// menu to open, rename or remove it.
fn folder_actions(
    st: &St,
    child: &gtk::FlowBoxChild,
    name: &str,
    picks: &Rc<Picks>,
    sync_picks: &Rc<dyn Fn()>,
    open_folder: &Rc<dyn Fn(Option<String>)>,
    spring_to: &Rc<dyn Fn(Option<String>)>,
) {
    let tile = child.child().and_then(|lift| lift.first_child());
    let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
    // Held over it for a moment, a sound opens the folder, to be dropped at a place in it.
    let spring = Spring::default();
    drop.connect_enter({
        let (tile, spring, spring_to, name) = (tile.clone(), spring.clone(), spring_to.clone(), name.to_string());
        move |_, _, _| {
            if let Some(t) = &tile {
                t.add_css_class("vn-drop-into");
            }
            let (tile, spring_to, name) = (tile.clone(), spring_to.clone(), name.clone());
            spring.arm(move || {
                if let Some(t) = &tile {
                    t.remove_css_class("vn-drop-into");
                }
                spring_to(Some(name));
            });
            gdk::DragAction::MOVE
        }
    });
    drop.connect_leave({
        let (tile, spring) = (tile.clone(), spring.clone());
        move |_| {
            spring.cancel();
            if let Some(t) = &tile {
                t.remove_css_class("vn-drop-into");
            }
        }
    });
    drop.connect_drop({
        let (st, picks, sync_picks, name) = (st.clone(), picks.clone(), sync_picks.clone(), name.to_string());
        move |_, value, _, _| {
            spring.cancel();
            if let Some(t) = &tile {
                t.remove_css_class("vn-drop-into");
            }
            let Ok(id) = value.get::<String>() else { return false };
            let ids = take_dragged(&picks, &sync_picks, id);
            // After the drop is done: moving builds the grid again, this folder too.
            let (st, name) = (st.clone(), name.clone());
            glib::idle_add_local_once(move || st.move_to_folder(&ids, Some(&name)));
            true
        }
    });
    child.add_controller(drop);

    let (st, name, open_folder, weak) = (st.clone(), name.to_string(), open_folder.clone(), child.downgrade());
    popup::context(child, move |x, y| {
        let Some(child) = weak.upgrade() else { return };
        let (st1, st2, n0, n1, n2, open_folder, parent) =
            (st.clone(), st.clone(), name.clone(), name.clone(), name.clone(), open_folder.clone(), child.downgrade());
        popup::show_list(&child, x, y, vec![
            popup::Item::new("folder-open-symbolic", t("Open"), move || open_folder(Some(n0.clone()))),
            popup::Item::new("document-edit-symbolic", t("Rename"), move || {
                let Some(parent) = parent.upgrade() else { return };
                let (st, old) = (st1.clone(), n1.clone());
                ask_folder_name(&parent, t("Rename folder"), &n1, t("Rename"), move |new| {
                    if new != old && st.library.borrow().folders.contains(&new) {
                        st.notify(Change::Notice(fill(t("There is a folder called “{}” already"), &[&new])));
                        return;
                    }
                    st.rename_folder(&old, &new);
                });
            }),
            popup::Item::new("user-trash-symbolic", t("Delete folder"), move || st2.remove_folder(&n2)).danger(),
        ]);
    });
}

/// Asks for the name of a folder. `done` gets it trimmed, and only when it is not empty.
fn ask_folder_name(parent: &impl IsA<gtk::Widget>, heading: &str, current: &str, ok: &str, done: impl Fn(String) + 'static) {
    let entry = gtk::Entry::builder().text(current).activates_default(true).build();
    entry.set_placeholder_text(Some(t("Folder name")));
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .extra_child(&entry)
        .default_response("ok")
        .close_response("cancel")
        .build();
    dialog.add_responses(&[("cancel", t("Cancel")), ("ok", ok)]);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_response_enabled("ok", !current.trim().is_empty());
    entry.connect_changed({
        let dialog = dialog.downgrade();
        move |e| {
            if let Some(d) = dialog.upgrade() {
                d.set_response_enabled("ok", !e.text().trim().is_empty());
            }
        }
    });
    dialog.connect_response(Some("ok"), {
        let entry = entry.clone();
        move |_, _| {
            let name = entry.text().trim().to_string();
            if !name.is_empty() {
                done(name);
            }
        }
    });
    dialog.present(Some(parent));
    entry.grab_focus();
}

fn volume_button(st: &St) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    b.set_icon_name("audio-volume-high-symbolic");
    b.set_tooltip_text(Some(t("Soundboard volume")));
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 10);
    bx.set_margin_top(12);
    bx.set_margin_bottom(12);
    bx.set_margin_start(12);
    bx.set_margin_end(12);
    bx.set_width_request(260);
    let title = label(t("Soundboard volume"), &["heading"]);
    title.set_xalign(0.0);
    let scale = volume_slider(st.config.borrow().sound_volume as f64, 1.0, {
        let st = st.clone();
        move |v| st.update(|c| c.sound_volume = v as f32)
    });
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let l = label(t("Hear sounds myself"), &[]);
    l.set_hexpand(true);
    l.set_xalign(0.0);
    let sw = gtk::Switch::new();
    sw.set_active(st.config.borrow().hear_sounds);
    sw.connect_active_notify({
        let st = st.clone();
        move |s| st.update(|c| c.hear_sounds = s.is_active())
    });
    row.append(&l);
    row.append(&sw);
    bx.append(&title);
    bx.append(&scale);
    bx.append(&row);
    let p = gtk::Popover::new();
    popup::menu(&p);
    p.set_child(Some(&bx));
    b.set_popover(Some(&p));
    b
}

/// A volume slider with a speaker that shows the level, and the value in percent. Click the value
/// to type one. It holds at 50 %, 100 % and 150 % on the way, so those are easy to hit.
fn volume_slider(value: f64, max: f64, on_change: impl Fn(f64) + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("vn-volume");
    let speaker = gtk::Image::new();
    speaker.add_css_class("vn-volume-icon");
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, max, 0.05);
    scale.set_hexpand(true);
    scale.set_value(value);
    super::notches(&scale, &[0.5, 1.0, 1.5]);
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

fn pad(st: &St, s: &Sound) -> Pad {
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

    // Little marks for how it plays and whether it is cut.
    let badges = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    badges.set_halign(gtk::Align::Start);
    badges.set_valign(gtk::Align::Start);
    badges.set_margin_top(8);
    badges.set_margin_start(8);
    // Only a mode of its own shows: the soundboard button says how the others play.
    let own = s.mode.filter(|m| *m != st.config.borrow().sound_mode);
    let marks = [
        (own.map(PlayMode::icon), own.map(|m| t_dyn(m.label())).unwrap_or_default()),
        (s.is_trimmed().then_some("edit-cut-symbolic"), t("Trimmed").to_string()),
    ];
    for (name, tip) in marks {
        if let Some(name) = name {
            let img = gtk::Image::from_icon_name(name);
            img.add_css_class("vn-pad-badge");
            img.set_tooltip_text(Some(&tip));
            badges.append(&img);
        }
    }

    let menu = gtk::Button::from_icon_name("view-more-symbolic");
    menu.add_css_class("flat");
    menu.add_css_class("vn-pad-menu");
    menu.set_halign(gtk::Align::End);
    menu.set_valign(gtk::Align::Start);
    menu.set_margin_top(6);
    menu.set_margin_end(6);
    menu.set_tooltip_text(Some(t("Edit sound")));

    let overlay = gtk::Overlay::new();
    overlay.add_css_class("vn-lift");
    overlay.set_child(Some(&pad));
    overlay.add_overlay(&menu);
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

/// Lets a pad be dragged: onto the grid to move it, or onto a folder to put it there. A picked
/// pad takes the other picked ones along, and they show as a little stack under the pointer.
fn drag_source(child: &gtk::FlowBoxChild, pad: &gtk::Box, id: &str, picks: &Rc<Picks>, pads: &Rc<RefCell<HashMap<String, Pad>>>) {
    let drag = gtk::DragSource::new();
    drag.set_actions(gdk::DragAction::MOVE);
    // Where the pad was grabbed, so the picture under the pointer hangs from the same spot.
    let grabbed = Rc::new(Cell::new((0.0, 0.0)));
    drag.connect_prepare({
        let (grabbed, id) = (grabbed.clone(), id.to_string());
        move |_, x, y| {
            grabbed.set((x, y));
            Some(gdk::ContentProvider::for_value(&id.to_value()))
        }
    });
    drag.connect_drag_begin({
        let (pad, picks, pads, id) = (pad.downgrade(), picks.clone(), pads.clone(), id.to_string());
        move |src, _| {
            let Some(pad) = pad.upgrade() else { return };
            let together = picks.ids.borrow().contains(&id);
            let ids: Vec<String> = if together { picks.list() } else { vec![id.clone()] };
            let pads = pads.borrow();
            let others: Vec<gtk::Box> = ids
                .iter()
                .filter(|i| **i != id)
                .filter_map(|i| pads.get(i).map(|p| p.pad.clone()))
                .filter(|p| p.is_mapped())
                .take(2)
                .collect();
            let (x, y) = grabbed.get();
            src.set_icon(Some(&drag_picture(&pad, &others, ids.len())), x as i32, y as i32);
            for i in &ids {
                if let Some(p) = pads.get(i) {
                    p.child.add_css_class("vn-dragging");
                }
            }
        }
    });
    drag.connect_drag_end({
        let pads = pads.clone();
        move |_, _, _| {
            for p in pads.borrow().values() {
                p.child.remove_css_class("vn-dragging");
            }
        }
    });
    child.add_controller(drag);
}

/// What hangs under the pointer while dragging: a still picture of the pad, since it may leave
/// the grid on the way, into another folder. With more sounds, the others stack up behind it,
/// and a badge says how many.
fn drag_picture(pad: &gtk::Box, others: &[gtk::Box], count: usize) -> gdk::Paintable {
    let still = |w: &gtk::Box| gtk::WidgetPaintable::new(Some(w)).current_image();
    if count <= 1 {
        return still(pad);
    }
    const STEP: f32 = 9.0;
    let (w, h) = (pad.width() as f32, pad.height() as f32);
    // Sounds in other folders are not on screen to take a picture of: the pad stands in for them.
    let behind: Vec<gdk::Paintable> = match others {
        [] => vec![still(pad)],
        _ => others.iter().map(still).collect(),
    };
    let snap = gtk::Snapshot::new();
    for (i, p) in behind.iter().enumerate().rev() {
        let off = STEP * (i + 1) as f32;
        snap.save();
        snap.translate(&gtk::graphene::Point::new(off, off));
        snap.push_opacity(0.85 - 0.2 * i as f64);
        p.snapshot(&snap, w as f64, h as f64);
        snap.pop();
        snap.restore();
    }
    still(pad).snapshot(&snap, w as f64, h as f64);
    let d = 26.0;
    let badge = gtk::graphene::Rect::new(w - d - 6.0, 6.0, d, d);
    snap.push_rounded_clip(&gtk::gsk::RoundedRect::from_rect(badge, d / 2.0));
    snap.append_color(&gdk::RGBA::new(0.23, 0.51, 0.96, 1.0), &badge);
    snap.pop();
    let layout = pad.create_pango_layout(Some(&count.to_string()));
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string("Bold 11")));
    let (lw, lh) = layout.pixel_size();
    snap.save();
    snap.translate(&gtk::graphene::Point::new(badge.x() + (d - lw as f32) / 2.0, badge.y() + (d - lh as f32) / 2.0));
    snap.append_layout(&layout, &gdk::RGBA::WHITE);
    snap.restore();
    let size = gtk::graphene::Size::new(w + STEP * behind.len() as f32, h + STEP * behind.len() as f32);
    snap.to_paintable(Some(&size)).unwrap_or_else(|| still(pad))
}

/// Opens something when a sound is held over a drop target for a moment, like spring-loaded
/// folders in a file manager: drag a sound out, back up and into another folder in one go. Enter
/// starts the wait, leave or drop ends it.
#[derive(Clone, Default)]
struct Spring(Rc<RefCell<Option<glib::SourceId>>>);

impl Spring {
    fn arm(&self, open: impl FnOnce() + 'static) {
        self.cancel();
        let slot = self.0.clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(650), move || {
            // Gone before it runs, so a late cancel does not remove a source that is done.
            slot.borrow_mut().take();
            open();
        });
        *self.0.borrow_mut() = Some(id);
    }

    fn cancel(&self) {
        if let Some(id) = self.0.borrow_mut().take() {
            id.remove();
        }
    }
}

/// Drops anywhere on the grid area move a pad: next to the nearest one, on the side the pointer
/// is, also in the gaps. A glowing bar shows where. A sound from another folder comes into the
/// one shown, together with the other picked ones, also when the folder is empty.
fn drop_on_grid(
    st: &St,
    area: &gtk::Stack,
    grid: &gtk::FlowBox,
    pads: &Rc<RefCell<HashMap<String, Pad>>>,
    shown: &Rc<RefCell<Option<String>>>,
    picks: &Rc<Picks>,
    sync_picks: &Rc<dyn Fn()>,
) {
    let mark: Rc<RefCell<Option<gtk::Widget>>> = Rc::default();
    let unmark = {
        let mark = mark.clone();
        move || {
            if let Some(c) = mark.borrow_mut().take() {
                c.remove_css_class("vn-drop-before");
                c.remove_css_class("vn-drop-after");
            }
        }
    };
    // Where a drop lands, like the cursor in a text: in the row under the pointer, or the nearest
    // row, before the first pad whose middle is right of it, else after the last pad of the row.
    // So the empty end of the last row puts the sounds at the end.
    let nearest = {
        let (area, grid, pads) = (area.downgrade(), grid.downgrade(), pads.clone());
        move |x: f64, y: f64| -> Option<(gtk::Widget, bool)> {
            let (area, grid) = (area.upgrade()?, grid.upgrade()?);
            if !grid.is_mapped() {
                return None;
            }
            let p = area.compute_point(&grid, &gtk::graphene::Point::new(x as f32, y as f32))?;
            let (x, y) = (p.x(), p.y());
            let mut cells: Vec<(gtk::Widget, gtk::graphene::Rect)> = Vec::new();
            let mut child = grid.first_child();
            while let Some(c) = child {
                // Folders are no place in the order: a drop on one goes into it.
                let is_pad = pads.borrow().values().any(|p| p.child.upcast_ref::<gtk::Widget>() == &c);
                if c.is_visible()
                    && is_pad
                    && let Some(r) = c.compute_bounds(&grid)
                {
                    cells.push((c.clone(), r));
                }
                child = c.next_sibling();
            }
            let off = |r: &gtk::graphene::Rect| (r.y() - y).max(y - r.y() - r.height()).max(0.0);
            let row = cells.iter().map(|(_, r)| *r).min_by(|a, b| off(a).total_cmp(&off(b)))?.y();
            let mut line: Vec<_> = cells.into_iter().filter(|(_, r)| (r.y() - row).abs() < 1.0).collect();
            line.sort_by(|a, b| a.1.x().total_cmp(&b.1.x()));
            match line.iter().find(|(_, r)| x < r.x() + r.width() / 2.0) {
                Some((c, _)) => Some((c.clone(), false)),
                None => line.last().map(|(c, _)| (c.clone(), true)),
            }
        }
    };
    let elsewhere = {
        let (st, shown) = (st.clone(), shown.clone());
        move |id: &str| st.library.borrow().find(id).is_some_and(|s| s.folder != *shown.borrow())
    };
    let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
    // The dragged id, already while it moves over: a sound from elsewhere may come in anywhere.
    drop.set_preload(true);
    drop.connect_motion({
        let (st, mark, unmark, nearest, elsewhere) = (st.clone(), mark.clone(), unmark.clone(), nearest.clone(), elsewhere.clone());
        move |target, x, y| {
            unmark();
            // Sorted by name, a place in the order means nothing, but a folder still does.
            if !st.config.borrow().sounds_by_name
                && let Some((c, after)) = nearest(x, y)
            {
                c.add_css_class(if after { "vn-drop-after" } else { "vn-drop-before" });
                *mark.borrow_mut() = Some(c);
                return gdk::DragAction::MOVE;
            }
            let id = target.value().and_then(|v| v.get::<String>().ok());
            if id.is_some_and(|id| elsewhere(&id)) { gdk::DragAction::MOVE } else { gdk::DragAction::empty() }
        }
    });
    drop.connect_leave({
        let unmark = unmark.clone();
        move |_| unmark()
    });
    drop.connect_drop({
        let (st, pads, shown, picks, sync_picks) = (st.clone(), pads.clone(), shown.clone(), picks.clone(), sync_picks.clone());
        move |_, value, x, y| {
            unmark();
            let Ok(from) = value.get::<String>() else { return false };
            let place = (!st.config.borrow().sounds_by_name).then(|| nearest(x, y)).flatten().and_then(|(c, after)| {
                let pads = pads.borrow();
                let target = pads.iter().find(|(_, p)| p.child.upcast_ref::<gtk::Widget>() == &c)?.0.clone();
                Some((target, after))
            });
            // A picked sound takes the other picked ones along, in the order of the soundboard.
            let group: Vec<String> = if picks.ids.borrow().contains(&from) {
                let ids = picks.ids.borrow();
                st.library.borrow().sound.iter().filter(|s| ids.contains(&s.id)).map(|s| s.id.clone()).collect()
            } else {
                vec![from.clone()]
            };
            let into = elsewhere(&from);
            if !into && place.is_none() {
                return false;
            }
            // Into another folder, the picks are done. Moved within one, they stay picked.
            if into {
                picks.clear();
                sync_picks();
            }
            // After the drop is done: moving builds the grid again, with the dragged pads in it.
            let (st, folder) = (st.clone(), shown.borrow().clone());
            glib::idle_add_local_once(move || {
                if into {
                    st.move_to_folder(&group, folder.as_deref());
                }
                if let Some((target, after)) = place {
                    st.move_sounds(&group, &target, after);
                }
            });
            true
        }
    });
    area.add_controller(drop);
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

    let vol_label = label(t("Volume"), &["caption-heading"]);
    vol_label.set_xalign(0.0);
    b.append(&vol_label);
    b.append(&volume_slider(s.volume as f64, 2.0, {
        let st = st.clone();
        let sound = sound.clone();
        move |v| {
            sound.borrow_mut().volume = v as f32;
            st.set_sound_volume(&sound.borrow().id, v as f32);
        }
    }));

    let mode_label = label(t("When pressed"), &["caption-heading"]);
    mode_label.set_xalign(0.0);
    b.append(&mode_label);
    // First "like the soundboard", then the modes.
    let mut names: Vec<String> = vec![t("Like the soundboard").into()];
    names.extend(PlayMode::ALL.iter().map(|m| t_dyn(m.label())));
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mode = gtk::DropDown::from_strings(&names);
    let own = s.mode.and_then(|m| PlayMode::ALL.iter().position(|a| *a == m));
    mode.set_selected(own.map_or(0, |i| i as u32 + 1));
    popup::menus_in(&mode);
    mode.connect_selected_notify({
        let sound = sound.clone();
        let save = save.clone();
        move |d| {
            sound.borrow_mut().mode = (d.selected() as usize).checked_sub(1).map(|i| PlayMode::ALL[i]);
            save();
        }
    });
    b.append(&mode);

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
