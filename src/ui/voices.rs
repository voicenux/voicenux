//! The voice page: a grid of voice cards and the selected voice on the right.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use super::visuals::Orb;
use super::{animate_in, board, cat_class, clear, icon, label, line_icon, popup, smooth_scroll, value_field};
use crate::dsp::blocks::{self, Kind};
use crate::i18n::{fill, t, t_dyn};
use crate::state::{Change, St};
use crate::voice::{CATEGORIES, Voice, category};

struct Card {
    child: gtk::FlowBoxChild,
    card: gtk::Box,
    star: gtk::Button,
    /// The shortcut of the voice, as a key cap in the corner.
    keys: gtk::Label,
    /// Marks in the top left corner. A check takes their place while the voice is picked.
    marks: Vec<gtk::Widget>,
    /// The marks in a row: AI, and changed by you.
    corner: gtk::Box,
    tweaked: gtk::Image,
    check: gtk::Image,
}

pub struct VoicesPage {
    pub widget: adw::ToolbarView,
    pub search: gtk::SearchEntry,
    /// Puts the focus on the card of the voice in use.
    pub show_current: Rc<dyn Fn()>,
}

/// Called with a voice id when someone wants to edit a voice in Voicelab.
pub type OpenLab = Rc<dyn Fn(&str)>;

/// The voices as a shelf with folders, for the grid that `board` shares with the soundboard.
struct VoiceShelf(St);

impl board::Shelf for VoiceShelf {
    fn folders(&self) -> Vec<String> {
        self.0.config.borrow().voice_folders.clone()
    }

    fn folder_of(&self, id: &str) -> Option<String> {
        self.0.voice_folder(id)
    }

    fn order(&self) -> Vec<String> {
        self.0.voices_shown()
    }

    fn add_folder(&self, name: &str) -> String {
        self.0.add_voice_folder(name)
    }

    fn rename_folder(&self, old: &str, new: &str) -> bool {
        self.0.rename_voice_folder(old, new)
    }

    fn remove_folder(&self, name: &str) {
        self.0.remove_voice_folder(name)
    }

    fn move_folder(&self, name: &str, target: &str, after: bool) {
        self.0.move_voice_folder(name, target, after)
    }

    fn move_to_folder(&self, ids: &[String], folder: Option<&str>) {
        self.0.voices_to_folder(ids, folder)
    }

    fn move_items(&self, ids: &[String], target: &str, after: bool) {
        self.0.move_voices(ids, target, after)
    }

    fn note(&self, text: String) {
        self.0.notify(Change::Notice(text))
    }
}

/// Folders look like voice cards.
const FOLDER_LOOK: board::TileLook = board::TileLook { card: "voice-card", size: (124, 146), title: "voice-name" };

pub fn build(st: &St, open_lab: OpenLab) -> VoicesPage {
    let shelf: board::Shared = Rc::new(VoiceShelf(st.clone()));
    let header = adw::HeaderBar::new();
    // The tools sit on the left, in line with the page. The rest of the bar is for dragging.
    header.set_show_title(false);
    let (search_box, search) = super::search_field(t("Search voices"));
    search_box.set_margin_start(12);
    header.pack_start(&search_box);
    // Out of all voices, or the arrow switches to your favorites.
    let random = super::random_button(
        [
            ("media-playlist-shuffle-symbolic", t("Random voice"), super::with_key(t("Random voice"), "R")),
            ("starred-symbolic", t("Random favorite"), super::with_key(t("Random favorite"), "Shift+R")),
        ],
        st.config.borrow().random_favorites,
        {
            let st = st.clone();
            move |favorites| {
                st.config.borrow_mut().random_favorites = favorites;
                st.save_later();
            }
        },
        {
            let st = st.clone();
            move |favorites| {
                if !st.random_voice(favorites) && favorites {
                    st.notify(Change::Notice(t("Star two voices or more first").into()));
                }
            }
        },
    );
    header.pack_start(&random);

    let filter = Rc::new(RefCell::new((String::from("all"), String::new())));

    // Category chips.
    let chips = adw::WrapBox::builder().child_spacing(8).line_spacing(8).build();
    chips.set_margin_start(20);
    chips.set_margin_end(20);
    chips.set_margin_top(6);
    chips.set_margin_bottom(12);

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
        .margin_top(14)
        .margin_bottom(24)
        .activate_on_single_click(true)
        .build();
    grid.add_css_class("vn-grid");
    let grid_scroll =
        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&grid).build();
    smooth_scroll(&grid_scroll);

    let empty = adw::StatusPage::builder()
        .paintable(&super::load_picture("magnifying-glass-tilted-left"))
        .title(t("No voices found"))
        .description(t("Try another word or category."))
        .build();
    let grid_stack = gtk::Stack::new();
    grid_stack.add_css_class("vn-pad-area");
    grid_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    grid_stack.add_named(&grid_scroll, Some("grid"));
    grid_stack.add_named(&empty, Some("empty"));

    // In a folder: the way back up over the grid. Drop a voice on it to take it out.
    let path = board::path_bar(t("All voices"), t("Back to all voices (Backspace)"));
    // And the bar for picked voices under the grid.
    let picks = Rc::new(board::Picks::default());
    let count = label("", &["heading"]);
    let done = gtk::Button::new();
    let delete = gtk::Button::with_label(t("Delete"));
    delete.add_css_class("destructive-action");
    delete.set_tooltip_text(Some(t("Deletes your own voices among them. The built-in ones stay.")));
    let move_to = gtk::MenuButton::new();
    move_to.set_label(t("Move to folder"));
    move_to.set_always_show_arrow(true);
    let star_all = gtk::Button::new();
    let select_all = gtk::Button::with_label(t("Select all"));
    select_all.set_tooltip_text(Some(&super::with_key(t("Select all"), "A")));
    let picks_bar = board::picks_bar(
        &done,
        &count,
        &[select_all.upcast_ref(), star_all.upcast_ref(), move_to.upcast_ref(), delete.upcast_ref()],
    );

    let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
    left.set_hexpand(true);
    left.append(&chips);
    left.append(&path.widget);
    left.append(&grid_stack);

    let detail = Detail::new(st, open_lab.clone());

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&left);
    body.append(&detail.widget);

    let widget = adw::ToolbarView::new();
    widget.add_top_bar(&header);
    widget.set_content(Some(&body));
    widget.add_bottom_bar(&picks_bar);
    widget.set_reveal_bottom_bars(false);

    let cards: Rc<RefCell<HashMap<String, Card>>> = Rc::default();
    // The folder the grid shows, None for the top. Folders sit at the start of the grid.
    let shown: Rc<RefCell<Option<String>>> = Rc::default();
    let tiles: board::Folders = Rc::default();
    // Voices that just moved, and the folder that took some: they bounce when the grid is sorted.
    let landing = Rc::new(board::Landing::default());
    // The first card that matches the search, top left: Enter in the search uses it.
    let first: Rc<RefCell<Option<String>>> = Rc::default();
    let item_of: board::ItemOf = Rc::new({
        let cards = cards.clone();
        move |w: &gtk::Widget| {
            cards.borrow().iter().find(|(_, c)| c.child.upcast_ref::<gtk::Widget>() == w).map(|(id, _)| id.clone())
        }
    });
    let faces: board::Faces = Rc::new({
        let cards = cards.clone();
        move |id: &str| cards.borrow().get(id).map(|c| (c.child.clone(), c.card.clone().upcast()))
    });

    // Folders first, then the voices in the order of the voice page.
    let rank: Rc<RefCell<HashMap<gtk::FlowBoxChild, usize>>> = Rc::default();
    grid.set_sort_func({
        let rank = rank.clone();
        move |a, b| {
            let rank = rank.borrow();
            rank.get(a).cmp(&rank.get(b)).into()
        }
    });
    let resort = {
        let (grid, cards, tiles, st) = (grid.clone(), cards.clone(), tiles.clone(), st.clone());
        Rc::new(move || {
            let mut rank = rank.borrow_mut();
            rank.clear();
            for (i, (c, _)) in tiles.borrow().iter().enumerate() {
                rank.insert(c.clone(), i);
            }
            let n = rank.len();
            for (i, id) in st.voices_shown().iter().enumerate() {
                if let Some(c) = cards.borrow().get(id) {
                    rank.insert(c.child.clone(), n + i);
                }
            }
            drop(rank);
            grid.invalidate_sort();
        })
    };

    let sync_picks: Rc<dyn Fn()> = Rc::new({
        let (st, cards, picks, view, count) = (st.clone(), cards.clone(), picks.clone(), widget.clone(), count.clone());
        let (select_all, star_all, delete, grid, item_of) =
            (select_all.clone(), star_all.clone(), delete.clone(), grid.clone(), item_of.clone());
        move || {
            let all = picks.all_of(&board::shown_items(&grid, &item_of));
            select_all.set_label(if all { t("Deselect all") } else { t("Select all") });
            let ids = picks.ids.borrow();
            for (id, c) in cards.borrow().iter() {
                let on = ids.contains(id);
                if on {
                    c.card.add_css_class("vn-selected");
                } else {
                    c.card.remove_css_class("vn-selected");
                }
                if on && !c.check.is_visible() {
                    board::bounce(&c.check, "vn-pop");
                }
                c.check.set_visible(on);
                for m in &c.marks {
                    m.set_opacity(if on { 0.0 } else { 1.0 });
                }
            }
            let favs = !ids.is_empty() && ids.iter().all(|id| st.is_favorite(id));
            star_all.set_label(if favs { t("Remove from favorites") } else { t("Add to favorites") });
            let own = ids.iter().any(|id| st.voice(id).is_some_and(|v| !v.builtin));
            delete.set_sensitive(own);
            view.set_reveal_bottom_bars(!ids.is_empty());
            count.set_label(&fill(t("{} selected"), &[&ids.len().to_string()]));
        }
    });

    // Like a file manager: the top shows the folders and the voices in none, a folder its voices.
    // A category or a search on the top looks at all voices.
    let refilter = {
        let (grid_stack, cards, filter, st, shown, tiles) =
            (grid_stack.clone(), cards.clone(), filter.clone(), st.clone(), shown.clone(), tiles.clone());
        let (empty, first) = (empty.clone(), first.clone());
        Rc::new(move |animate: bool| {
            let (cat, query) = filter.borrow().clone();
            let query = query.to_lowercase();
            let folder = shown.borrow().clone();
            let top = cat == "all" && query.is_empty() && folder.is_none();
            let mut visible_count = 0;
            for (c, _) in tiles.borrow().iter() {
                c.set_visible(top);
                if top {
                    if animate {
                        animate_in(c, visible_count);
                    }
                    visible_count += 1;
                }
            }
            let mut first_match = None;
            for id in st.voices_shown().iter() {
                let Some(c) = cards.borrow().get(id).map(|c| c.child.clone()) else { continue };
                let Some(v) = st.voices.borrow().iter().find(|v| &v.id == id).cloned() else { continue };
                let in_cat = match cat.as_str() {
                    "all" => true,
                    "favorites" => st.is_favorite(id),
                    c => v.category == c,
                };
                let here = if cat != "all" {
                    true
                } else if query.is_empty() {
                    st.voice_folder(id) == folder
                } else {
                    folder.is_none() || st.voice_folder(id) == folder
                };
                let name = t_dyn(&v.name).to_lowercase();
                let matches = query.is_empty()
                    || name.contains(&query)
                    || v.name.to_lowercase().contains(&query)
                    || t_dyn(&v.description).to_lowercase().contains(&query)
                    || t_dyn(category(&v.category).label).to_lowercase().contains(&query);
                let visible = in_cat && here && matches;
                c.set_visible(visible);
                if visible {
                    if first_match.is_none() && !query.is_empty() {
                        first_match = Some(id.clone());
                    }
                    if animate {
                        animate_in(&c, visible_count);
                    }
                    visible_count += 1;
                }
            }
            for (id, c) in cards.borrow().iter() {
                if first_match.as_ref() == Some(id) {
                    c.card.add_css_class("vn-first");
                } else {
                    c.card.remove_css_class("vn-first");
                }
            }
            *first.borrow_mut() = first_match;
            // An empty folder says how voices get in, not that the search found nothing.
            if folder.is_some() && query.is_empty() {
                empty.set_title(t("Nothing in this folder yet"));
                empty.set_description(Some(t(
                    "Drag voices onto the folder, or pick some with Ctrl+click and move them here.",
                )));
            } else {
                empty.set_title(t("No voices found"));
                empty.set_description(Some(t("Try another word or category.")));
            }
            grid_stack.set_visible_child_name(if visible_count == 0 { "empty" } else { "grid" });
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
    // Goes into a folder, or back up with None, at once like a file manager. The picks stay only
    // for a spring while dragging: they go along into the folder.
    let goto: Rc<dyn Fn(Option<String>, bool)> = Rc::new({
        let (shown, picks, sync_path, refilter, sync_picks) =
            (shown.clone(), picks.clone(), sync_path.clone(), refilter.clone(), sync_picks.clone());
        let scroll = grid_scroll.downgrade();
        move |folder: Option<String>, keep_picks: bool| {
            *shown.borrow_mut() = folder;
            if !keep_picks {
                picks.clear();
            }
            sync_path();
            refilter(false);
            sync_picks();
            if let Some(scroll) = scroll.upgrade() {
                scroll.vadjustment().set_value(0.0);
            }
        }
    });
    let open_folder: board::Open = {
        let goto = goto.clone();
        Rc::new(move |folder| goto(folder, false))
    };
    let spring_to: board::Open = Rc::new(move |folder| goto(folder, true));

    // The folder tiles, made new when folders or what is in them change.
    let rebuild_tiles = {
        let (st, grid, tiles, shelf, picks, sync_picks) =
            (st.clone(), grid.clone(), tiles.clone(), shelf.clone(), picks.clone(), sync_picks.clone());
        let (open_folder, spring_to, landing, shown, sync_path) =
            (open_folder.clone(), spring_to.clone(), landing.clone(), shown.clone(), sync_path.clone());
        Rc::new(move || {
            for (c, _) in tiles.borrow_mut().drain(..) {
                grid.remove(&c);
            }
            let folders = st.config.borrow().voice_folders.clone();
            // The folder went away, removed or renamed: back to the top.
            if shown.borrow().as_ref().is_some_and(|f| !folders.contains(f)) {
                shown.borrow_mut().take();
            }
            sync_path();
            let order = st.voices_shown();
            for name in folders {
                let inside: Vec<&String> =
                    order.iter().filter(|id| st.voice_folder(id).as_deref() == Some(name.as_str())).collect();
                let icons: Vec<String> = inside.iter().take(3).filter_map(|id| st.voice(id).map(|v| v.icon)).collect();
                let count = match inside.len() {
                    0 => t("Empty").to_string(),
                    1 => t("1 voice").to_string(),
                    n => fill(t("{} voices"), &[&n.to_string()]),
                };
                let child = board::folder_tile(&name, &icons, &count, &FOLDER_LOOK);
                board::folder_actions(&shelf, &child, &name, &picks, &sync_picks, &open_folder, &spring_to, &landing);
                grid.append(&child);
                tiles.borrow_mut().push((child, name));
            }
        })
    };

    let rebuild_chips = {
        let chips = chips.clone();
        let filter = filter.clone();
        let refilter = refilter.clone();
        let st = st.clone();
        let (shown, sync_path, picks, sync_picks) =
            (shown.clone(), sync_path.clone(), picks.clone(), sync_picks.clone());
        Rc::new(move || {
            clear(&chips);
            let has_custom = st.voices.borrow().iter().any(|v| !v.builtin);
            let mut list: Vec<(&str, String)> = vec![("all", t("All").into()), ("favorites", t("Favorites").into())];
            if has_custom {
                list.push(("custom", t("My voices").into()));
            }
            for c in CATEGORIES.iter().filter(|c| c.id != "custom") {
                list.push((c.id, t_dyn(c.label)));
            }
            let current = filter.borrow().0.clone();
            let mut first: Option<gtk::ToggleButton> = None;
            // A click on the category you are in goes back to All. A click that picks a category
            // toggles it first, so this tells the two apart.
            let picked = Rc::new(Cell::new(false));
            for (id, name) in list {
                let b = gtk::ToggleButton::new();
                let inner = gtk::Box::new(gtk::Orientation::Horizontal, 7);
                let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                dot.add_css_class("vn-chip-dot");
                dot.set_valign(gtk::Align::Center);
                if id == "favorites" {
                    let s = gtk::Image::from_icon_name("starred-symbolic");
                    s.set_pixel_size(13);
                    inner.append(&s);
                } else {
                    inner.append(&dot);
                }
                inner.append(&gtk::Label::new(Some(&name)));
                b.set_child(Some(&inner));
                b.add_css_class("vn-chip");
                b.add_css_class(&cat_class(id));
                b.set_active(id == current);
                match &first {
                    Some(f) => b.set_group(Some(f)),
                    None => first = Some(b.clone()),
                }
                let filter = filter.clone();
                let refilter = refilter.clone();
                let id = id.to_string();
                b.connect_toggled({
                    let picked = picked.clone();
                    let (shown, sync_path, picks, sync_picks) =
                        (shown.clone(), sync_path.clone(), picks.clone(), sync_picks.clone());
                    move |b| {
                        if b.is_active() {
                            picked.set(true);
                            filter.borrow_mut().0 = id.clone();
                            // A category leaves the folder: it looks at all voices.
                            shown.borrow_mut().take();
                            sync_path();
                            picks.clear();
                            refilter(true);
                            sync_picks();
                        }
                    }
                });
                b.connect_clicked({
                    let (picked, first) = (picked.clone(), first.clone());
                    move |b| {
                        if !picked.replace(false)
                            && b.is_active()
                            && let Some(all) = first.as_ref().filter(|all| *all != b)
                        {
                            all.set_active(true);
                            picked.set(false);
                        }
                    }
                });
                chips.append(&b);
                let _ = &chips;
            }
            if !st.voices.borrow().iter().any(|v| !v.builtin) && current == "custom" {
                filter.borrow_mut().0 = "all".into();
            }
        })
    };

    let update_active = {
        let cards = cards.clone();
        let st = st.clone();
        Rc::new(move || {
            let current = st.config.borrow().voice.clone();
            let on = st.config.borrow().voice_on;
            for (id, c) in cards.borrow().iter() {
                let active = *id == current;
                if active {
                    c.card.add_css_class("active");
                } else {
                    c.card.remove_css_class("active");
                }
                if active && !on {
                    c.card.add_css_class("off");
                } else {
                    c.card.remove_css_class("off");
                }
                let fav = st.is_favorite(id);
                c.star.set_icon_name(if fav { "starred-symbolic" } else { "non-starred-symbolic" });
                if fav {
                    c.star.add_css_class("on");
                } else {
                    c.star.remove_css_class("on");
                }
            }
        })
    };

    // After the folders or the order changed: tiles, order and what shows, and the bounce of
    // what just moved.
    let reshelve = {
        let (rebuild_tiles, resort, refilter, sync_picks, landing, cards, tiles) = (
            rebuild_tiles.clone(),
            resort.clone(),
            refilter.clone(),
            sync_picks.clone(),
            landing.clone(),
            cards.clone(),
            tiles.clone(),
        );
        let (picks, st) = (picks.clone(), st.clone());
        Rc::new(move || {
            rebuild_tiles();
            resort();
            refilter(false);
            picks.ids.borrow_mut().retain(|id| st.voice(id).is_some());
            sync_picks();
            landing.play(|id| cards.borrow().get(id).map(|c| c.child.clone().upcast()), &tiles);
        })
    };

    let rebuild_cards = {
        let grid = grid.clone();
        let cards = cards.clone();
        let st = st.clone();
        let open_lab = open_lab.clone();
        let (update_active, reshelve, picks, faces) =
            (update_active.clone(), reshelve.clone(), picks.clone(), faces.clone());
        let refilter = refilter.clone();
        Rc::new(move || {
            for (_, c) in cards.borrow_mut().drain() {
                grid.remove(&c.child);
            }
            let voices = st.voices.borrow().clone();
            for v in voices.iter() {
                let c = card(&st, v, &open_lab);
                board::drag_source(&c.child, c.card.upcast_ref(), &v.id, &picks, &faces);
                grid.append(&c.child);
                cards.borrow_mut().insert(v.id.clone(), c);
            }
            update_active();
            reshelve();
            refilter(true);
        })
    };

    // While voices are picked, a click or Enter picks too, instead of using the voice.
    grid.connect_child_activated({
        let st = st.clone();
        let cards = cards.clone();
        let (tiles, open_folder, picks, sync_picks) =
            (tiles.clone(), open_folder.clone(), picks.clone(), sync_picks.clone());
        move |_, child| {
            let folder = tiles.borrow().iter().find(|(c, _)| c == child).map(|(_, n)| n.clone());
            if let Some(folder) = folder {
                open_folder(Some(folder));
                return;
            }
            let id = cards.borrow().iter().find(|(_, c)| &c.child == child).map(|(id, _)| id.clone());
            let Some(id) = id else { return };
            if !picks.is_empty() {
                picks.toggle(&id);
                sync_picks();
                return;
            }
            use_voice(&st, &id);
        }
    });
    board::pick_clicks(&grid, &item_of, &picks, &sync_picks);
    // Only on All and in your order does a place mean something, and only there do voices come
    // into a folder: the other chips are filters.
    let can_order: Rc<dyn Fn() -> bool> = Rc::new({
        let (st, filter) = (st.clone(), filter.clone());
        move || filter.borrow().0 == "all" && !st.config.borrow().voices_by_name
    });
    let can_take: Rc<dyn Fn() -> bool> = Rc::new({
        let filter = filter.clone();
        move || filter.borrow().0 == "all"
    });
    board::drop_on_grid(
        &shelf,
        &grid_stack,
        &grid,
        &item_of,
        &shown,
        &picks,
        &sync_picks,
        can_order,
        can_take,
        &landing,
    );
    board::drop_folders_on_grid(&shelf, &grid_stack, &grid, &tiles, &landing);
    board::up_takes_drops(&path.up, &shelf, &picks, &sync_picks, &spring_to);
    path.up.connect_clicked({
        let open_folder = open_folder.clone();
        move |_| open_folder(None)
    });
    board::mouse_back(&widget, &shown, &open_folder);
    board::move_to_menu(&move_to, &shelf, &picks, &sync_picks, &landing);

    let pick_all: Rc<dyn Fn()> = Rc::new({
        let (grid, item_of, picks, sync_picks) = (grid.clone(), item_of.clone(), picks.clone(), sync_picks.clone());
        move || {
            picks.ids.borrow_mut().extend(board::shown_items(&grid, &item_of));
            sync_picks();
        }
    });
    let delete_picks: Rc<dyn Fn()> = Rc::new({
        let (st, shelf, picks, sync_picks, widget) =
            (st.clone(), shelf.clone(), picks.clone(), sync_picks.clone(), widget.downgrade());
        move || {
            let own: Vec<String> =
                picks.in_order(&shelf).into_iter().filter(|id| st.voice(id).is_some_and(|v| !v.builtin)).collect();
            let Some(parent) = widget.upgrade() else { return };
            ask_delete_voices(&st, &own, &parent, {
                let (picks, sync_picks) = (picks.clone(), sync_picks.clone());
                move || {
                    picks.clear();
                    sync_picks();
                }
            });
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
    star_all.connect_clicked({
        let (st, picks) = (st.clone(), picks.clone());
        move |_| {
            let ids = picks.list();
            let all = ids.iter().all(|id| st.is_favorite(id));
            st.set_favorites(&ids, !all);
        }
    });
    // The button in the bar puts all down again when all are picked.
    select_all.connect_clicked({
        let (pick_all, picks, sync_picks, grid, item_of) =
            (pick_all.clone(), picks.clone(), sync_picks.clone(), grid.clone(), item_of.clone());
        move |_| {
            if picks.all_of(&board::shown_items(&grid, &item_of)) {
                picks.clear();
                sync_picks();
            } else {
                pick_all();
            }
        }
    });

    // Ctrl+A picks all voices on the grid, Delete removes the picked ones of your own.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let (grid, picks, pick_all, delete_picks) =
            (grid.clone(), picks.clone(), pick_all.clone(), delete_picks.clone());
        move |_, key, _, mods| {
            if grid.focus_child().filter(|c| c.has_focus()).is_none() {
                return glib::Propagation::Proceed;
            }
            match key {
                gdk::Key::a if mods.contains(gdk::ModifierType::CONTROL_MASK) => pick_all(),
                gdk::Key::Delete | gdk::Key::KP_Delete if !picks.is_empty() => delete_picks(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });
    grid.add_controller(keys);
    // Escape puts the picks down. Backspace or Alt+Left goes back up out of a folder.
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed({
        let (picks, sync_picks, shown, open_folder, search) =
            (picks.clone(), sync_picks.clone(), shown.clone(), open_folder.clone(), search.clone());
        move |_, key, _, mods| {
            let alt_left = key == gdk::Key::Left && mods == gdk::ModifierType::ALT_MASK;
            let backspace = key == gdk::Key::BackSpace && mods.is_empty() && search.text().is_empty();
            if (alt_left || backspace) && shown.borrow().is_some() {
                open_folder(None);
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Escape && mods.is_empty() && !picks.is_empty() {
                picks.clear();
                sync_picks();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    widget.add_controller(escape);

    // A right click on free space: things for all voices. Cards have their own menu.
    popup::context(&grid_scroll, {
        let (st, scroll, shelf, open_lab) = (st.clone(), grid_scroll.downgrade(), shelf.clone(), open_lab.clone());
        let (picks, sync_picks, pick_all, grid, item_of) =
            (picks.clone(), sync_picks.clone(), pick_all.clone(), grid.clone(), item_of.clone());
        move |x, y| {
            let Some(scroll) = scroll.upgrade() else { return };
            let mut items = vec![board::new_folder_item(&shelf, scroll.upcast_ref())];
            if !picks.all_of(&board::shown_items(&grid, &item_of)) {
                let pick_all = pick_all.clone();
                items.push(popup::Item::new("edit-select-all-symbolic", t("Select all"), move || pick_all()));
            }
            if !picks.is_empty() {
                let (picks, sync_picks) = (picks.clone(), sync_picks.clone());
                items.push(popup::Item::new("edit-clear-symbolic", t("Deselect all"), move || {
                    picks.clear();
                    sync_picks();
                }));
            }
            let (st1, st2, st3, st4, open_lab) = (st.clone(), st.clone(), st.clone(), st.clone(), open_lab.clone());
            items.push(popup::Item::new("list-add-symbolic", t("New voice"), move || {
                let v = st1.new_custom();
                st1.set_voice(&v.id);
                open_lab(&v.id);
            }));
            items.push(popup::Item::new("media-playlist-shuffle-symbolic", t("Random voice"), move || {
                st2.random_voice(false);
            }));
            if st4.config.borrow().favorites.len() >= 2 {
                items.push(popup::Item::new("starred-symbolic", t("Random favorite"), move || {
                    st4.random_voice(true);
                }));
            }
            items.push(if st3.config.borrow().voices_by_name {
                popup::Item::new("view-list-symbolic", t("Your own order"), move || st3.set_voices_by_name(false))
            } else {
                popup::Item::new("view-sort-ascending-symbolic", t("Sort by name"), move || {
                    st3.set_voices_by_name(true)
                })
            });
            popup::show_list(&scroll, x, y, items);
        }
    });

    search.connect_search_changed({
        let filter = filter.clone();
        let refilter = refilter.clone();
        move |s| {
            filter.borrow_mut().1 = s.text().to_string();
            refilter(false);
        }
    });
    // Enter uses the first match and leaves the search as it is. Down goes to the matches.
    search.connect_activate({
        let (st, first) = (st.clone(), first.clone());
        move |_| {
            let id = first.borrow().clone();
            if let Some(id) = id {
                use_voice(&st, &id);
            }
        }
    });
    let down = gtk::EventControllerKey::new();
    down.connect_key_pressed({
        let (cards, first) = (cards.clone(), first.clone());
        move |_, key, _, _| {
            if !matches!(key, gdk::Key::Down | gdk::Key::KP_Down) {
                return glib::Propagation::Proceed;
            }
            let child = first.borrow().as_ref().and_then(|id| cards.borrow().get(id).map(|c| c.child.clone()));
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

    rebuild_chips();
    rebuild_cards();
    detail.show(&st.current(), false);

    st.subscribe({
        let rebuild_cards = rebuild_cards.clone();
        let rebuild_chips = rebuild_chips.clone();
        let update_active = update_active.clone();
        let refilter = refilter.clone();
        let filter = filter.clone();
        let cards = cards.clone();
        let st = st.clone();
        let detail = detail.clone();
        let (resort, reshelve, sync_picks) = (resort.clone(), reshelve.clone(), sync_picks.clone());
        // Many changes in a row, like a drop that moves voices into a folder and to a place,
        // sort the grid once.
        let queued = Rc::new(Cell::new(false));
        move |change| match change {
            Change::Voices => {
                rebuild_chips();
                rebuild_cards();
                detail.show(&st.current(), false);
            }
            Change::VoiceShelf if !queued.replace(true) => {
                let (reshelve, queued) = (reshelve.clone(), queued.clone());
                glib::idle_add_local_once(move || {
                    queued.set(false);
                    reshelve();
                });
            }
            Change::VoiceEdited(id) => {
                if let Some(v) = st.voice(id) {
                    if let Some(c) = cards.borrow().get(id) {
                        refresh_card(c, &v);
                        show_tweaked(c, st.is_tweaked(id));
                    }
                    if *id == st.config.borrow().voice {
                        detail.edited(&v);
                    }
                }
            }
            Change::Voice => {
                update_active();
                detail.show(&st.current(), true);
                let id = st.config.borrow().voice.clone();
                if let Some(c) = cards.borrow().get(&id)
                    && c.child.is_visible()
                {
                    c.child.grab_focus();
                }
            }
            Change::Toggles => {
                update_active();
                detail.toggles();
            }
            Change::Favorites => {
                update_active();
                detail.toggles();
                resort();
                sync_picks();
                if filter.borrow().0 == "favorites" {
                    refilter(false);
                }
            }
            Change::Settings => {
                for (id, c) in cards.borrow().iter() {
                    show_keys(&st, &c.keys, id);
                }
            }
            _ => {}
        }
    });

    // The card of the voice in use. In a folder, the folder opens for it.
    let show_current: Rc<dyn Fn()> = Rc::new({
        let (st, cards, open_folder, filter, shown) =
            (st.clone(), cards.clone(), open_folder.clone(), filter.clone(), shown.clone());
        move || {
            let id = st.config.borrow().voice.clone();
            let folder = st.voice_folder(&id);
            if filter.borrow().0 == "all" && *shown.borrow() != folder {
                open_folder(folder);
            }
            if let Some(c) = cards.borrow().get(&id)
                && c.child.is_visible()
            {
                c.child.grab_focus();
            }
        }
    });
    // The first visit shows what nothing on screen tells: the right click, and the settings.
    super::tour::on_first_visit(st, "voices", &widget, {
        let (grid, scroll, params) = (grid.downgrade(), grid_scroll.downgrade(), detail.params.downgrade());
        move || {
            let (Some(grid), Some(scroll), Some(params)) = (grid.upgrade(), scroll.upgrade(), params.upgrade()) else {
                return Vec::new();
            };
            let mut tips = Vec::new();
            if let Some(first) = grid.child_at_index(0) {
                tips.push(super::tour::Tip {
                    at: first.upcast(),
                    area: None,
                    title: t("Pick a voice"),
                    text: t("Click a voice to use it."),
                });
            }
            // The switches in the bottom bar, right after: that is where trying it goes on. One
            // tip for all three, so it takes one click, not three.
            if let Some(at) = super::tour::named(&grid, "vn-switches") {
                tips.push(super::tour::Tip {
                    at,
                    area: None,
                    title: t("The switches"),
                    text: t("Voice changer turns your new voice on and off. Hear myself plays it in your headphones. Background adds the sound of a voice, like rain or a campfire."),
                });
            }
            tips.push(super::tour::Tip {
                at: grid.clone().upcast(),
                area: Some(board::spot_finder(&grid, &scroll)),
                title: t("Right-click an empty spot"),
                text: t(
                    "There you make folders and sort the voices. Drag voices to move them, Ctrl+click picks several.",
                ),
            });
            // The first setting, not the whole panel: the bubble sits next to it.
            tips.push(super::tour::Tip {
                at: params.first_child().unwrap_or_else(|| params.upcast()),
                area: None,
                title: t("Make it yours"),
                text: t("Change the main settings here, or click Customize for all of them. What you change gets a blue frame."),
            });
            tips
        }
    });
    VoicesPage { widget, search, show_current }
}

/// A click on a voice: it goes on. A click on the voice that is on turns it off, your normal
/// voice. Unless it is an AI voice that is not downloaded: then the click offers the download.
fn use_voice(st: &St, id: &str) {
    let on = st.config.borrow().voice == id && st.config.borrow().voice_on;
    let missing =
        st.voice(id).and_then(|v| crate::ai::model_of(&v.block_values())).is_some_and(|m| !crate::ai::ready(m));
    if on && !missing { st.set_voice_on(false) } else { st.set_voice(id) }
}

/// Asks before your own voices go for good, then deletes them. `done` runs after.
fn ask_delete_voices(st: &St, ids: &[String], parent: &impl IsA<gtk::Widget>, done: impl Fn() + 'static) {
    let heading = match ids {
        [] => return,
        [id] => fill(t("Delete {}?"), &[&st.voice(id).map(|v| t_dyn(&v.name)).unwrap_or_default()]),
        _ => fill(t("Delete {} voices?"), &[&ids.len().to_string()]),
    };
    let body = if ids.len() == 1 { t("The voice is gone for good.") } else { t("They are gone for good.") };
    let dialog = adw::AlertDialog::new(Some(&heading), Some(body));
    dialog.add_response("cancel", t("Cancel"));
    dialog.add_response("delete", t("Delete"));
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    let (st, ids) = (st.clone(), ids.to_vec());
    dialog.connect_response(None, move |_, r| {
        if r == "delete" {
            for id in &ids {
                st.delete_custom(id);
            }
            done();
        }
    });
    dialog.present(Some(parent));
}

fn refresh_card(c: &Card, v: &Voice) {
    // Custom voices can change name and picture in Voicelab.
    let mut child = c.card.first_child();
    while let Some(w) = child {
        if let Some(img) = w.downcast_ref::<gtk::Image>() {
            img.set_paintable(Some(&super::texture(&v.icon)));
        }
        if let Some(l) = w.downcast_ref::<gtk::Label>()
            && l.has_css_class("voice-name")
        {
            l.set_label(&t_dyn(&v.name));
        }
        child = w.next_sibling();
    }
}

fn card(st: &St, v: &Voice, open_lab: &OpenLab) -> Card {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("voice-card");
    card.add_css_class(&cat_class(&v.category));
    card.set_size_request(124, 146);

    let img = icon(&v.icon, 72);
    img.add_css_class("voice-icon");
    img.set_margin_top(8);
    card.append(&img);
    let name = label(&t_dyn(&v.name), &["voice-name"]);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_max_width_chars(14);
    card.append(&name);

    let star = gtk::Button::from_icon_name("non-starred-symbolic");
    star.add_css_class("flat");
    star.add_css_class("vn-star");
    star.set_halign(gtk::Align::End);
    star.set_valign(gtk::Align::Start);
    star.set_margin_top(6);
    star.set_margin_end(6);
    star.set_tooltip_text(Some(t("Favorite")));
    star.connect_clicked({
        let st = st.clone();
        let id = v.id.clone();
        move |_| st.toggle_favorite(&id)
    });

    let keys = label("", &["vn-key", "vn-card-keys"]);
    keys.set_halign(gtk::Align::Start);
    keys.set_valign(gtk::Align::Start);
    keys.set_margin_top(10);
    keys.set_margin_start(10);
    keys.set_can_target(false);
    show_keys(st, &keys, &v.id);

    // The overlay lifts on hover, so the star moves along with the card.
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("vn-lift");
    overlay.set_child(Some(&card));
    overlay.add_overlay(&star);
    overlay.add_overlay(&keys);
    // The marks sit in a row in the corner, the shortcut goes under them.
    let corner = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    corner.set_halign(gtk::Align::Start);
    corner.set_valign(gtk::Align::Start);
    corner.set_margin_top(10);
    corner.set_margin_start(10);
    overlay.add_overlay(&corner);
    let marks: Vec<gtk::Widget> = vec![keys.clone().upcast(), corner.clone().upcast()];

    let mut tip = t_dyn(&v.description);
    // AI voices: marked, gray without a graphics card, with a download mark until they are there.
    if let Some(model) = crate::ai::model_of(&v.block_values()) {
        let mark = label(t("AI"), &["vn-ai-mark"]);
        mark.set_valign(gtk::Align::Center);
        mark.set_can_target(false);
        corner.append(&mark);
        if !crate::ai::usable() {
            card.add_css_class("vn-ai-off");
            tip = format!(
                "{tip}\n{}",
                t("AI voices need an NVIDIA or AMD graphics card, or Run on the processor in Settings.")
            );
        } else if !crate::ai::ready(model) {
            let get = gtk::Image::from_icon_name("folder-download-symbolic");
            get.add_css_class("vn-ai-get");
            get.set_halign(gtk::Align::End);
            get.set_valign(gtk::Align::Start);
            get.set_margin_top(40);
            get.set_margin_end(10);
            get.set_can_target(false);
            overlay.add_overlay(&get);
            tip = format!("{tip}\n{}", t("Not downloaded yet. Pick the voice to get it."));
        }
    }

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
    child.set_tooltip_text(Some(&tip));
    super::tile_button(&star, &child);
    popup::context(&child, {
        let (st, open_lab, id, weak) = (st.clone(), open_lab.clone(), v.id.clone(), child.downgrade());
        move |x, y| {
            if let Some(child) = weak.upgrade() {
                card_menu(&st, &open_lab, &child, &id, x, y);
            }
        }
    });
    // A voice you changed says so, until you reset it.
    let tweaked = line_icon("sliders-horizontal", 12);
    tweaked.add_css_class("vn-tweak-mark");
    tweaked.set_valign(gtk::Align::Center);
    tweaked.set_tooltip_text(Some(t("You changed this voice")));
    corner.append(&tweaked);

    let c = Card { child, card, star, keys, marks, corner, tweaked, check };
    show_tweaked(&c, st.is_tweaked(&v.id));
    c
}

/// Shows the mark of a voice you changed, and puts the shortcut under the marks.
fn show_tweaked(c: &Card, on: bool) {
    c.tweaked.set_visible(on);
    let marked = std::iter::successors(c.corner.first_child(), |w| w.next_sibling()).any(|w| w.is_visible());
    c.keys.set_margin_top(if marked { 34 } else { 10 });
}

/// Shows the shortcut of a voice on its card, or nothing.
fn show_keys(st: &St, keys: &gtk::Label, id: &str) {
    let shown = super::keys::shown(st, &format!("voice:{id}"));
    keys.set_label(shown.as_deref().unwrap_or(""));
    keys.set_visible(shown.is_some());
}

/// The right-click menu of a voice card.
fn card_menu(st: &St, open_lab: &OpenLab, child: &gtk::FlowBoxChild, id: &str, x: f64, y: f64) {
    let Some(v) = st.voice(id) else { return };
    let on = st.config.borrow().voice == id && st.config.borrow().voice_on;
    let fav = st.is_favorite(id);
    let key = format!("voice:{id}");
    let has_key = super::keys::shown(st, &key).is_some();
    let (st1, st2, st3, open_lab) = (st.clone(), st.clone(), st.clone(), open_lab.clone());
    let (id1, id2, id3) = (id.to_string(), id.to_string(), id.to_string());
    let weak = child.downgrade();
    let items = vec![
        if on {
            popup::Item::new("system-shutdown-symbolic", t("Turn off"), move || st1.set_voice_on(false))
        } else {
            popup::Item::new("object-select-symbolic", t("Use voice"), move || st1.set_voice(&id1))
        },
        popup::Item::new(
            if fav { "starred-symbolic" } else { "non-starred-symbolic" },
            if fav { t("Remove from favorites") } else { t("Add to favorites") },
            move || st2.toggle_favorite(&id2),
        ),
        popup::Item::new("applications-science-symbolic", t("Customize"), move || open_lab(&id3)),
        popup::Item::new(
            "input-keyboard-symbolic",
            if has_key { t("Change shortcut") } else { t("Add shortcut") },
            move || {
                if let Some(child) = weak.upgrade() {
                    super::keys::record(&st3, &child, &key, &t_dyn(&v.name));
                }
            },
        ),
    ];
    popup::show_list(child, x, y, items);
}

/// The selected voice: orb, name, and its main settings.
#[derive(Clone)]
struct Detail {
    widget: gtk::ScrolledWindow,
    inner: gtk::Box,
    orb: Rc<Orb>,
    cat: gtk::Label,
    name: gtk::Label,
    about: gtk::Label,
    fav: gtk::Button,
    fav_state: gtk::Stack,
    params: gtk::Box,
    key_slot: gtk::Box,
    /// Says that you changed the voice, with the way back.
    tweaked: gtk::Revealer,
    note: gtk::Label,
    st: St,
    shown: Rc<RefCell<String>>,
    busy: Rc<Cell<bool>>,
}

impl Detail {
    fn new(st: &St, open_lab: OpenLab) -> Self {
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);
        inner.set_margin_top(18);
        inner.set_margin_bottom(24);
        inner.set_margin_start(22);
        inner.set_margin_end(22);

        let orb = Rc::new(Orb::new(st, 224));
        inner.append(&orb.widget);

        let cat = label("", &["vn-detail-cat"]);
        cat.set_margin_top(4);
        let name = label("", &["vn-detail-name"]);
        name.set_wrap(true);
        name.set_justify(gtk::Justification::Center);
        let about = label("", &["dim-label"]);
        about.set_wrap(true);
        about.set_justify(gtk::Justification::Center);
        about.set_max_width_chars(30);
        inner.append(&cat);
        inner.append(&name);
        inner.append(&about);

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::Center);
        buttons.set_margin_top(10);
        // Both labels in a stack: the button keeps the width of the longer one, nothing jumps.
        let fav_state = gtk::Stack::new();
        fav_state.set_transition_type(gtk::StackTransitionType::Crossfade);
        for (name, icon, text) in
            [("off", "non-starred-symbolic", t("Add to favorites")), ("on", "starred-symbolic", t("Favorite"))]
        {
            fav_state.add_named(&adw::ButtonContent::builder().icon_name(icon).label(text).build(), Some(name));
        }
        let fav = gtk::Button::builder().child(&fav_state).build();
        fav.add_css_class("pill");
        fav.add_css_class("image-text-button");
        fav.add_css_class("vn-fav");
        let lab = gtk::Button::new();
        lab.add_css_class("pill");
        lab.set_child(Some(
            &adw::ButtonContent::builder().icon_name("applications-science-symbolic").label(t("Customize")).build(),
        ));
        buttons.append(&fav);
        buttons.append(&lab);
        inner.append(&buttons);
        let key_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        key_slot.set_halign(gtk::Align::Center);
        inner.append(&key_slot);

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        head.set_margin_top(18);
        let title = label(t("Voice settings"), &["heading"]);
        title.set_halign(gtk::Align::Start);
        title.set_hexpand(true);
        head.append(&title);
        inner.append(&head);

        // Once you change a built-in voice, a note says so, with the way back.
        let reset_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        reset_content.append(&line_icon("rotate-ccw", 14));
        reset_content.append(&gtk::Label::new(Some(t("Reset"))));
        let reset = gtk::Button::builder().child(&reset_content).valign(gtk::Align::Center).build();
        reset.add_css_class("vn-gradient-button");
        reset.add_css_class("vn-reset");
        reset.set_tooltip_text(Some(t("Back to how the voice comes")));
        let changed = label(t("Changed by you"), &[]);
        changed.set_hexpand(true);
        changed.set_xalign(0.0);
        changed.set_wrap(true);
        let banner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        banner.add_css_class("vn-tweaked");
        banner.set_margin_top(4);
        banner.append(&line_icon("sliders-horizontal", 16));
        banner.append(&changed);
        banner.append(&reset);
        let tweaked = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(220)
            .child(&banner)
            .build();
        inner.append(&tweaked);

        let params = gtk::Box::new(gtk::Orientation::Vertical, 8);
        params.set_margin_top(4);
        inner.append(&params);

        let note = label("", &["dim-label", "caption"]);
        note.set_wrap(true);
        note.set_margin_top(10);
        note.set_xalign(0.0);
        inner.append(&note);

        let widget = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(320)
            .child(&inner)
            .build();
        widget.add_css_class("vn-detail");
        smooth_scroll(&widget);

        let d = Self {
            widget,
            inner,
            orb,
            cat,
            name,
            about,
            fav: fav.clone(),
            fav_state,
            params,
            key_slot,
            tweaked,
            note,
            st: st.clone(),
            shown: Rc::default(),
            busy: Rc::default(),
        };
        fav.connect_clicked({
            let st = st.clone();
            let shown = d.shown.clone();
            move |_| {
                let id = shown.borrow().clone();
                st.toggle_favorite(&id);
            }
        });
        // The AI tells how it is doing while it starts and loads.
        glib::timeout_add_local(std::time::Duration::from_millis(500), {
            let d = d.clone();
            move || {
                if d.widget.is_mapped() {
                    d.update_note();
                }
                glib::ControlFlow::Continue
            }
        });
        lab.connect_clicked({
            let shown = d.shown.clone();
            move |_| open_lab(&shown.borrow())
        });
        reset.connect_clicked({
            let st = st.clone();
            let shown = d.shown.clone();
            let d = d.clone();
            move |_| {
                let id = shown.borrow().clone();
                st.reset_voice(&id);
                if let Some(v) = st.voice(&id) {
                    d.show(&v, false);
                }
            }
        });
        d
    }

    fn show(&self, v: &Voice, animate: bool) {
        *self.shown.borrow_mut() = v.id.clone();
        let c = category(&v.category);
        for cat in CATEGORIES {
            self.inner.remove_css_class(&cat_class(cat.id));
        }
        self.inner.add_css_class(&cat_class(&v.category));
        self.orb.set_voice(&v.icon, c.color, animate);
        self.cat.set_label(&t_dyn(c.label).to_uppercase());
        self.name.set_label(&t_dyn(&v.name));
        let about =
            if v.description.is_empty() { t("Made by you in Voicelab.").to_string() } else { t_dyn(&v.description) };
        self.about.set_label(&about);
        self.params_for(v);
        clear(&self.key_slot);
        self.key_slot.append(&super::keys::button(&self.st, &format!("voice:{}", v.id), &t_dyn(&v.name)));
        self.toggles();
        if animate {
            for w in [self.name.upcast_ref::<gtk::Widget>(), self.about.upcast_ref(), self.params.upcast_ref()] {
                w.remove_css_class("vn-detail-pop");
                let w = w.clone();
                glib::idle_add_local_once(move || w.add_css_class("vn-detail-pop"));
            }
        }
    }

    fn edited(&self, v: &Voice) {
        if self.busy.get() || *self.shown.borrow() != v.id {
            self.tweaked.set_reveal_child(self.st.is_tweaked(&v.id));
            return;
        }
        self.show(v, false);
    }

    fn toggles(&self) {
        let id = self.shown.borrow().clone();
        let fav = self.st.is_favorite(&id);
        self.fav_state.set_visible_child_name(if fav { "on" } else { "off" });
        if fav {
            self.fav.add_css_class("on");
        } else {
            self.fav.remove_css_class("on");
        }
        self.update_note();
        self.tweaked.set_reveal_child(self.st.is_tweaked(&id));
    }

    /// The note under the settings: what the AI does, or a hint about the background sound.
    fn update_note(&self) {
        let id = self.shown.borrow().clone();
        let v = self.st.voice(&id);
        let model = v.as_ref().and_then(|v| crate::ai::model_of(&v.block_values()));
        let note = if let Some(model) = model {
            super::ai::status_text(model)
        } else if v.is_some_and(|v| v.has_background() && !self.st.config.borrow().background) {
            t("This voice has a background sound. Turn on Background below to hear it.").into()
        } else {
            String::new()
        };
        if self.note.label() != note {
            self.note.set_label(&note);
        }
        self.note.set_visible(!note.is_empty());
    }

    fn params_for(&self, v: &Voice) {
        clear(&self.params);
        // What you changed of a built-in voice lights up, with a way back for each setting.
        let original = self.st.original(&v.id);
        let mut any = false;
        for (bi, b) in v.blocks.iter().enumerate() {
            let Some(spec) = blocks::find(&b.kind) else { continue };
            if !b.enabled {
                continue;
            }
            let values = b.values();
            let first =
                original.as_ref().and_then(|o| o.blocks.get(bi)).filter(|o| o.kind == b.kind).map(|o| o.values());
            for (pi, p) in spec.params.iter().enumerate() {
                if !p.main {
                    continue;
                }
                any = true;
                let was = first.as_ref().and_then(|f| f.get(pi).copied());
                let undo =
                    gtk::Button::builder().child(&line_icon("rotate-ccw", 14)).valign(gtk::Align::Center).build();
                undo.add_css_class("flat");
                undo.add_css_class("circular");
                undo.add_css_class("vn-param-undo");
                let row_of: Rc<std::cell::OnceCell<gtk::Box>> = Rc::default();
                let mark = {
                    let (undo, row_of) = (undo.clone(), row_of.clone());
                    move |value: f32| {
                        let changed = was.is_some_and(|w| (w - value).abs() > 1e-4);
                        undo.set_visible(changed);
                        if let Some(row) = row_of.get() {
                            if changed {
                                row.add_css_class("vn-changed");
                            } else {
                                row.remove_css_class("vn-changed");
                            }
                        }
                    }
                };
                let row = param_row(p, values[pi], {
                    let st = self.st.clone();
                    let id = v.id.clone();
                    let key = p.key;
                    let busy = self.busy.clone();
                    let tweaked = self.tweaked.clone();
                    let mark = mark.clone();
                    move |value| {
                        busy.set(true);
                        st.set_param(&id, bi, key, value);
                        busy.set(false);
                        tweaked.set_reveal_child(st.is_tweaked(&id));
                        mark(value);
                    }
                });
                let _ = row_of.set(row.clone());
                mark(values[pi]);
                if let Some(w) = was {
                    let shown = t_dyn(&blocks::format_value(p, w));
                    undo.set_tooltip_text(Some(&fill(t("Back to {}"), &[&shown])));
                    undo.connect_clicked({
                        let (st, id, key) = (self.st.clone(), v.id.clone(), p.key);
                        move |_| {
                            // After the click: going back builds these rows again.
                            let (st, id) = (st.clone(), id.clone());
                            glib::idle_add_local_once(move || st.set_param(&id, bi, key, w));
                        }
                    });
                }
                // Between the name and the value.
                if let Some(top) = row.first_child().and_downcast::<gtk::Box>() {
                    top.insert_child_after(&undo, top.first_child().as_ref());
                }
                self.params.append(&row);
            }
        }
        if !any {
            self.params.append(&label(t("This voice has nothing to adjust."), &["dim-label"]));
        }
    }
}

/// A labelled slider (or switch, or dropdown) for one parameter.
pub fn param_row(p: &'static blocks::ParamSpec, value: f32, on_change: impl Fn(f32) + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.add_css_class("vn-param");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = label(t_dyn(p.label).as_str(), &["vn-param-label"]);
    name.set_hexpand(true);
    name.set_xalign(0.0);
    top.append(&name);
    row.append(&top);

    match p.kind {
        Kind::Toggle => {
            let sw = gtk::Switch::new();
            sw.set_active(value >= 0.5);
            sw.set_valign(gtk::Align::Center);
            sw.connect_active_notify(move |s| on_change(if s.is_active() { 1.0 } else { 0.0 }));
            top.append(&sw);
            row.set_margin_bottom(0);
        }
        Kind::Choice(list) => {
            let names: Vec<String> = list.iter().map(|c| t_dyn(c)).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let dd = gtk::DropDown::from_strings(&names);
            dd.set_selected(value as u32);
            super::popup::menus_in(&dd);
            dd.connect_selected_notify(move |d| on_change(d.selected() as f32));
            top.append(&dd);
        }
        Kind::Float | Kind::Int => {
            // Log sliders run on log(value), the rest on the value itself.
            let to_scale = move |v: f32| if p.log { v.max(1e-3).ln() as f64 } else { v as f64 };
            let from_scale = move |s: &gtk::Scale| {
                let raw = s.value() as f32;
                p.clamp(if p.log { raw.exp() } else { raw })
            };
            let (lo, hi) = (to_scale(p.min), to_scale(p.max));
            let step = if matches!(p.kind, Kind::Int) { 1.0 } else { (hi - lo) / 400.0 };
            let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, lo, hi, step);
            scale.set_value(to_scale(value));
            scale.set_draw_value(false);
            scale.set_hexpand(true);
            // Holds at zero, half way for percents, and where the effect starts out.
            let mut points = vec![to_scale(p.default)];
            if !p.log && p.min < 0.0 && p.max > 0.0 {
                points.push(0.0);
            }
            if matches!(p.unit, blocks::Unit::Percent) {
                points.push(0.5);
            }
            super::notches(&scale, &points);
            // A notch of the wheel: one step, or a fiftieth of the way.
            super::wheel_slider(&scale, if matches!(p.kind, Kind::Int) { 1.0 } else { (hi - lo) / 50.0 });
            let shown = value_field(
                {
                    let scale = scale.clone();
                    move || blocks::format_value(p, from_scale(&scale))
                },
                {
                    let scale = scale.clone();
                    move |f, text| {
                        if let Some(v) = blocks::parse_value(p, text) {
                            scale.set_value(to_scale(v));
                        }
                        // Also puts the old value back after something that is not a value.
                        f.set_text(&blocks::format_value(p, from_scale(&scale)));
                    }
                },
            );
            shown.add_css_class("vn-param-value");
            shown.set_text(&blocks::format_value(p, value));
            top.append(&shown);
            // While you drag, the voice follows, at most every 40 ms. Settings can make it wait
            // until you let go: a voice that slides while you talk can sound like a DJ scratching.
            // Keys, the wheel and typed values change it after a short pause, so a held key does
            // not slide it either.
            let sent = Rc::new(Cell::new(value));
            let apply: Rc<dyn Fn(f32)> = Rc::new(move |v: f32| {
                if (v - sent.replace(v)).abs() > f32::EPSILON {
                    on_change(v);
                }
            });
            let held = Rc::new(Cell::new(false));
            let press = gtk::EventControllerLegacy::new();
            press.set_propagation_phase(gtk::PropagationPhase::Capture);
            press.connect_event({
                let (held, apply) = (held.clone(), apply.clone());
                move |c, e| {
                    use gdk::EventType::*;
                    match e.event_type() {
                        ButtonPress | TouchBegin => held.set(true),
                        ButtonRelease | TouchEnd | TouchCancel if held.replace(false) => {
                            // After the slider took the release and its last move.
                            let (scale, apply) = (c.widget().and_downcast::<gtk::Scale>(), apply.clone());
                            glib::idle_add_local_once(move || {
                                if let Some(s) = scale {
                                    apply(from_scale(&s));
                                }
                            });
                        }
                        _ => {}
                    }
                    glib::Propagation::Proceed
                }
            });
            scale.add_controller(press);
            let pause: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
            let follow: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
            scale.connect_value_changed(move |s| {
                let v = from_scale(s);
                shown.set_text(&blocks::format_value(p, v));
                if held.get() {
                    if super::live_sliders() && follow.borrow().is_none() {
                        let (scale, apply, again) = (s.downgrade(), apply.clone(), follow.clone());
                        let wait = std::time::Duration::from_millis(40);
                        follow.replace(Some(glib::timeout_add_local_once(wait, move || {
                            again.take();
                            if let Some(s) = scale.upgrade() {
                                apply(from_scale(&s));
                            }
                        })));
                    }
                    return;
                }
                if let Some(id) = pause.take() {
                    id.remove();
                }
                let (scale, apply, again) = (s.downgrade(), apply.clone(), pause.clone());
                pause.replace(Some(glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                    again.take();
                    if let Some(s) = scale.upgrade() {
                        apply(from_scale(&s));
                    }
                })));
            });
            row.append(&scale);
        }
    }
    row
}
