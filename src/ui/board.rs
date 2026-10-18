//! What the soundboard and the voice page share: a grid with folders on it, like a file manager.
//! Tiles are picked with Ctrl+click and Shift+click, dragged to a new place or into a folder, and
//! a folder opens when you hold a tile over it for a moment. Folders stay at the start and move
//! among themselves. Moved tiles bounce in at their new place, a folder that takes some gulps.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};

use super::{icon, label, popup};
use crate::i18n::{fill, t};

/// What a grid with folders works on: the sounds of the soundboard, or the voices.
pub trait Shelf {
    fn folders(&self) -> Vec<String>;
    fn folder_of(&self, id: &str) -> Option<String>;
    /// All ids, in the order they are stored: a group that moves keeps this order.
    fn order(&self) -> Vec<String>;
    fn add_folder(&self, name: &str) -> String;
    /// False when the new name is empty or taken.
    fn rename_folder(&self, old: &str, new: &str) -> bool;
    fn remove_folder(&self, name: &str);
    fn move_folder(&self, name: &str, target: &str, after: bool);
    fn move_to_folder(&self, ids: &[String], folder: Option<&str>);
    fn move_items(&self, ids: &[String], target: &str, after: bool);
    /// A short note at the bottom of the window.
    fn note(&self, text: String);
}

pub type Shared = Rc<dyn Shelf>;
/// The id of the item a grid child shows. None for folders and anything else.
pub type ItemOf = Rc<dyn Fn(&gtk::Widget) -> Option<String>>;
/// The grid child of an item, and its face: the card that is pictured while dragging.
pub type Faces = Rc<dyn Fn(&str) -> Option<(gtk::FlowBoxChild, gtk::Widget)>>;
/// Goes into a folder, or back up with None.
pub type Open = Rc<dyn Fn(Option<String>)>;
pub type Folders = Rc<RefCell<Vec<(gtk::FlowBoxChild, String)>>>;

/// The tiles picked with Ctrl+click or Shift+click, for Delete and Move. While some are, a click
/// picks instead of doing what it does.
#[derive(Default)]
pub struct Picks {
    pub ids: RefCell<HashSet<String>>,
    /// Where Shift+click starts its row of picks.
    pub anchor: RefCell<Option<String>>,
}

impl Picks {
    pub fn is_empty(&self) -> bool {
        self.ids.borrow().is_empty()
    }

    pub fn list(&self) -> Vec<String> {
        self.ids.borrow().iter().cloned().collect()
    }

    pub fn toggle(&self, id: &str) {
        let mut ids = self.ids.borrow_mut();
        if !ids.remove(id) {
            ids.insert(id.to_string());
        }
        *self.anchor.borrow_mut() = Some(id.to_string());
    }

    pub fn clear(&self) {
        self.ids.borrow_mut().clear();
        self.anchor.borrow_mut().take();
    }

    /// Every one of these, the tiles on the grid right now, is picked.
    pub fn all_of(&self, shown: &[String]) -> bool {
        let ids = self.ids.borrow();
        !shown.is_empty() && shown.iter().all(|id| ids.contains(id))
    }

    /// The picked ones in the order of the shelf.
    pub fn in_order(&self, shelf: &Shared) -> Vec<String> {
        let ids = self.ids.borrow();
        shelf.order().into_iter().filter(|id| ids.contains(id)).collect()
    }
}

/// The ids of the items the grid shows right now, in its order.
pub fn shown_items(grid: &gtk::FlowBox, item_of: &ItemOf) -> Vec<String> {
    let mut ids = Vec::new();
    let mut child = grid.first_child();
    while let Some(c) = child {
        if c.is_visible()
            && let Some(id) = item_of(&c)
        {
            ids.push(id);
        }
        child = c.next_sibling();
    }
    ids
}

/// The area for a tip to right-click a free spot, looked up again every frame: [`free_spot`],
/// which keeps the one it showed while that stays in sight.
pub fn spot_finder(grid: &gtk::FlowBox, scroll: &gtk::ScrolledWindow) -> super::tour::Area {
    let (grid, scroll, shown) = (grid.downgrade(), scroll.downgrade(), Cell::new(None));
    Rc::new(move || {
        let spot = free_spot(&grid.upgrade()?, &scroll.upgrade()?, shown.get());
        shown.set(spot);
        spot
    })
}

/// A free spot among the tiles that is in sight in `scroll`, for the tip to right-click one, in
/// the coordinates of `grid`: after the last tile when its row has room, a gap between two tiles
/// of a row, or between two rows. `keep` is the spot shown so far: while it stays in sight, it
/// stays, so the mark does not hop when the list scrolls.
pub fn free_spot(
    grid: &gtk::FlowBox,
    scroll: &gtk::ScrolledWindow,
    keep: Option<gtk::graphene::Rect>,
) -> Option<gtk::graphene::Rect> {
    let mut cells: Vec<gtk::graphene::Rect> = Vec::new();
    let mut child = grid.first_child();
    while let Some(c) = child {
        if c.is_visible()
            && let Some(r) = c.compute_bounds(grid)
        {
            cells.push(r);
        }
        child = c.next_sibling();
    }
    // In the order on screen, rows from the top: the widgets are in the order they came in, not
    // in the sorted one.
    cells.sort_by(|a, b| a.y().total_cmp(&b.y()).then(a.x().total_cmp(&b.x())));
    let in_sight = |r: &gtk::graphene::Rect| {
        let top = grid.compute_point(scroll, &gtk::graphene::Point::new(r.x(), r.y()));
        top.is_some_and(|p| p.y() >= 0.0 && p.y() + r.height() <= scroll.height() as f32)
    };
    let mut spots: Vec<gtk::graphene::Rect> = Vec::new();
    let gap = grid.column_spacing() as f32;
    if let Some(last) = cells.last() {
        let free = gtk::graphene::Rect::new(last.x() + last.width() + gap, last.y(), last.width(), last.height());
        if free.x() + free.width() <= grid.width() as f32 {
            spots.push(free);
        }
    }
    for w in cells.windows(2).filter(|w| (w[0].y() - w[1].y()).abs() < 1.0) {
        let left = w[0].x() + w[0].width();
        spots.push(gtk::graphene::Rect::new(left, w[0].y(), w[1].x() - left, w[0].height()));
    }
    // Between two rows, under the first tile of the upper one: for a list one tile wide.
    for w in cells.windows(2).filter(|w| w[1].y() > w[0].y() + w[0].height()) {
        let row_start = cells.iter().find(|c| (c.y() - w[0].y()).abs() < 1.0).unwrap_or(&w[0]);
        let bottom = w[0].y() + w[0].height();
        spots.push(gtk::graphene::Rect::new(row_start.x(), bottom, row_start.width(), w[1].y() - bottom));
    }
    let spots: Vec<_> = spots.into_iter().filter(|r| r.width() > 4.0 && r.height() > 4.0 && in_sight(r)).collect();
    let same = |a: &gtk::graphene::Rect, b: &gtk::graphene::Rect| {
        (a.x() - b.x()).abs() < 1.0 && (a.y() - b.y()).abs() < 1.0 && (a.width() - b.width()).abs() < 1.0
    };
    if let Some(k) = keep
        && spots.iter().any(|s| same(s, &k))
    {
        return Some(k);
    }
    spots.into_iter().next()
}

/// What a drop moves: the picked tiles when the dragged one is one of them, in the order of the
/// shelf, else just the dragged one.
pub fn group_of(shelf: &Shared, picks: &Picks, id: &str) -> Vec<String> {
    if picks.ids.borrow().contains(id) { picks.in_order(shelf) } else { vec![id.to_string()] }
}

/// Tiles that just moved, to bounce in at their new place once the grid shows them there, and the
/// folder that took some, to gulp. The pages play them after they built or sorted the grid.
#[derive(Default)]
pub struct Landing {
    pub items: RefCell<HashSet<String>>,
    pub folder: RefCell<Option<String>>,
    pub moved_folder: RefCell<Option<String>>,
}

impl Landing {
    pub fn play(&self, child_of: impl Fn(&str) -> Option<gtk::Widget>, folders: &Folders) {
        for id in self.items.borrow_mut().drain() {
            if let Some(c) = child_of(&id) {
                bounce(&c, "vn-landed");
            }
        }
        let gulp = self.folder.borrow_mut().take();
        let moved = self.moved_folder.borrow_mut().take();
        for (c, name) in folders.borrow().iter() {
            if gulp.as_ref() == Some(name) {
                bounce(c, "vn-gulp");
            } else if moved.as_ref() == Some(name) {
                bounce(c, "vn-landed");
            }
        }
    }
}

/// Plays a one-off animation on a widget: the class starts it, and goes again when it is done,
/// so the next time starts it again too.
pub fn bounce(w: &impl IsA<gtk::Widget>, class: &'static str) {
    let w = w.as_ref();
    w.remove_css_class(class);
    w.add_css_class(class);
    let weak = w.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(700), move || {
        if let Some(w) = weak.upgrade() {
            w.remove_css_class(class);
        }
    });
}

/// Lets a tile be dragged: onto the grid to move it, or onto a folder to put it there. A picked
/// tile takes the other picked ones along, and they show as a little stack under the pointer.
pub fn drag_source(child: &gtk::FlowBoxChild, face: &gtk::Widget, id: &str, picks: &Rc<Picks>, faces: &Faces) {
    let drag = gtk::DragSource::new();
    drag.set_actions(gdk::DragAction::MOVE);
    // Where the tile was grabbed, so the picture under the pointer hangs from the same spot.
    let grabbed = Rc::new(Cell::new((0.0, 0.0)));
    let dimmed: Rc<RefCell<Vec<gtk::FlowBoxChild>>> = Rc::default();
    drag.connect_prepare({
        let (grabbed, id) = (grabbed.clone(), id.to_string());
        move |_, x, y| {
            grabbed.set((x, y));
            Some(gdk::ContentProvider::for_value(&id.to_value()))
        }
    });
    drag.connect_drag_begin({
        let (face, picks, faces, id, dimmed) =
            (face.downgrade(), picks.clone(), faces.clone(), id.to_string(), dimmed.clone());
        move |src, _| {
            let Some(face) = face.upgrade() else { return };
            let ids: Vec<String> = if picks.ids.borrow().contains(&id) { picks.list() } else { vec![id.clone()] };
            let others: Vec<gtk::Widget> = ids
                .iter()
                .filter(|i| **i != id)
                .filter_map(|i| faces(i).map(|(_, f)| f))
                .filter(|f| f.is_mapped())
                .take(2)
                .collect();
            let (x, y) = grabbed.get();
            src.set_icon(Some(&drag_picture(&face, &others, ids.len())), x as i32, y as i32);
            for i in &ids {
                if let Some((c, _)) = faces(i) {
                    c.add_css_class("vn-dragging");
                    dimmed.borrow_mut().push(c);
                }
            }
        }
    });
    drag.connect_drag_end(move |_, _, _| {
        for c in dimmed.borrow_mut().drain(..) {
            c.remove_css_class("vn-dragging");
        }
    });
    child.add_controller(drag);
}

/// What hangs under the pointer while dragging: a still picture of the tile, since it may leave
/// the grid on the way, into another folder. With more tiles, the others stack up behind it, and a
/// badge says how many.
fn drag_picture(face: &gtk::Widget, others: &[gtk::Widget], count: usize) -> gdk::Paintable {
    let still = |w: &gtk::Widget| gtk::WidgetPaintable::new(Some(w)).current_image();
    if count <= 1 {
        return still(face);
    }
    const STEP: f32 = 9.0;
    let (w, h) = (face.width() as f32, face.height() as f32);
    // Tiles in other folders are not on screen to take a picture of: the face stands in for them.
    let behind: Vec<gdk::Paintable> = match others {
        [] => vec![still(face)],
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
    still(face).snapshot(&snap, w as f64, h as f64);
    let d = 26.0;
    let badge = gtk::graphene::Rect::new(w - d - 6.0, 6.0, d, d);
    snap.push_rounded_clip(&gtk::gsk::RoundedRect::from_rect(badge, d / 2.0));
    snap.append_color(&gdk::RGBA::new(0.23, 0.51, 0.96, 1.0), &badge);
    snap.pop();
    let layout = face.create_pango_layout(Some(&count.to_string()));
    layout.set_font_description(Some(&gtk::pango::FontDescription::from_string("Bold 11")));
    let (lw, lh) = layout.pixel_size();
    snap.save();
    snap.translate(&gtk::graphene::Point::new(badge.x() + (d - lw as f32) / 2.0, badge.y() + (d - lh as f32) / 2.0));
    snap.append_layout(&layout, &gdk::RGBA::WHITE);
    snap.restore();
    let size = gtk::graphene::Size::new(w + STEP * behind.len() as f32, h + STEP * behind.len() as f32);
    snap.to_paintable(Some(&size)).unwrap_or_else(|| still(face))
}

/// Opens something when a tile is held over a drop target for a moment, like spring-loaded
/// folders in a file manager: drag a tile out, back up and into another folder in one go. Enter
/// starts the wait, leave or drop ends it.
#[derive(Clone, Default)]
pub struct Spring(Rc<RefCell<Option<glib::SourceId>>>);

impl Spring {
    pub fn arm(&self, open: impl FnOnce() + 'static) {
        self.cancel();
        let slot = self.0.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(650), move || {
            // Gone before it runs, so a late cancel does not remove a source that is done.
            slot.borrow_mut().take();
            open();
        });
        *self.0.borrow_mut() = Some(id);
    }

    pub fn cancel(&self) {
        if let Some(id) = self.0.borrow_mut().take() {
            id.remove();
        }
    }
}

/// The gap a drop at `x`, `y` of `area` lands in, among the tiles of the grid that `keep` takes:
/// like the cursor in a text, in the row under the pointer or the nearest row, before the first
/// tile whose middle is right of it, else after the last tile of the row. It gives the tiles on
/// both sides of the gap in that row, so the empty end of a row means after its last tile.
fn gap_at(
    area: &gtk::Widget,
    grid: &gtk::FlowBox,
    x: f64,
    y: f64,
    keep: &dyn Fn(&gtk::Widget) -> bool,
) -> Option<(Option<gtk::Widget>, Option<gtk::Widget>)> {
    if !grid.is_mapped() {
        return None;
    }
    let p = area.compute_point(grid, &gtk::graphene::Point::new(x as f32, y as f32))?;
    let (x, y) = (p.x(), p.y());
    let mut cells: Vec<(gtk::Widget, gtk::graphene::Rect)> = Vec::new();
    let mut child = grid.first_child();
    while let Some(c) = child {
        if c.is_visible()
            && keep(&c)
            && let Some(r) = c.compute_bounds(grid)
        {
            cells.push((c.clone(), r));
        }
        child = c.next_sibling();
    }
    let off = |r: &gtk::graphene::Rect| (r.y() - y).max(y - r.y() - r.height()).max(0.0);
    let row = cells.iter().map(|(_, r)| *r).min_by(|a, b| off(a).total_cmp(&off(b)))?.y();
    let mut line: Vec<_> = cells.into_iter().filter(|(_, r)| (r.y() - row).abs() < 1.0).collect();
    line.sort_by(|a, b| a.1.x().total_cmp(&b.1.x()));
    let right = line.iter().position(|(_, r)| x < r.x() + r.width() / 2.0).unwrap_or(line.len());
    let left = right.checked_sub(1).map(|i| line[i].0.clone());
    let right = line.get(right).map(|(c, _)| c.clone());
    Some((left, right))
}

/// Lights up both tiles at a gap, at the edge that faces it, and remembers them to put them out.
/// The same gap again changes nothing, so the light does not start over with every move.
#[derive(Clone, Default)]
struct GapMark(Rc<RefCell<Vec<(gtk::Widget, &'static str)>>>);

impl GapMark {
    fn show(&self, gap: (Option<gtk::Widget>, Option<gtk::Widget>)) {
        let lit: Vec<(gtk::Widget, &'static str)> = [(gap.0, "vn-drop-after"), (gap.1, "vn-drop-before")]
            .into_iter()
            .filter_map(|(c, k)| Some((c?, k)))
            .collect();
        if *self.0.borrow() == lit {
            return;
        }
        self.clear();
        for (c, class) in &lit {
            c.add_css_class(class);
        }
        *self.0.borrow_mut() = lit;
    }

    fn clear(&self) {
        for (c, class) in self.0.borrow_mut().drain(..) {
            c.remove_css_class(class);
        }
    }
}

/// Before the tile right of a gap, or after the one left of it at the end of a row.
fn place(gap: (Option<gtk::Widget>, Option<gtk::Widget>)) -> Option<(gtk::Widget, bool)> {
    match gap {
        (_, Some(r)) => Some((r, false)),
        (Some(l), None) => Some((l, true)),
        (None, None) => None,
    }
}

/// Drops anywhere on the grid area move tiles: into the gap nearest the pointer, with both tiles
/// at the gap lit up. A tile from another folder comes into the one shown, together with the
/// other picked ones, also when the folder is empty. `can_order` says if a place in the order
/// means anything right now: not when the grid is sorted by name, say. `can_take` says if tiles
/// from other folders may come in: not in a view that is only a filter, like the favorites.
#[allow(clippy::too_many_arguments)]
pub fn drop_on_grid(
    shelf: &Shared,
    area: &impl IsA<gtk::Widget>,
    grid: &gtk::FlowBox,
    item_of: &ItemOf,
    shown: &Rc<RefCell<Option<String>>>,
    picks: &Rc<Picks>,
    sync_picks: &Rc<dyn Fn()>,
    can_order: Rc<dyn Fn() -> bool>,
    can_take: Rc<dyn Fn() -> bool>,
    landing: &Rc<Landing>,
) {
    let mark = GapMark::default();
    let nearest = {
        let (area, grid, item_of) = (area.as_ref().downgrade(), grid.downgrade(), item_of.clone());
        move |x: f64, y: f64| {
            // Folders are no place in the order: a drop on one goes into it.
            let is_item = |c: &gtk::Widget| item_of(c).is_some();
            gap_at(&area.upgrade()?, &grid.upgrade()?, x, y, &is_item)
        }
    };
    let elsewhere = {
        let (shelf, shown) = (shelf.clone(), shown.clone());
        move |id: &str| can_take() && shelf.folder_of(id) != *shown.borrow()
    };
    // Where a drop of `from` would land, by the gap. None when it would change nothing, like the
    // gaps right next to the dragged tile: those do not light up or open.
    let spot_for = {
        let (shelf, picks, grid, item_of) = (shelf.clone(), picks.clone(), grid.downgrade(), item_of.clone());
        move |from: Option<&str>, gap: (Option<gtk::Widget>, Option<gtk::Widget>)| {
            let (c, after) = place(gap)?;
            let target = item_of(&c)?;
            let (Some(from), Some(grid)) = (from, grid.upgrade()) else { return Some((target, after)) };
            let shown = shown_items(&grid, &item_of);
            // Tiles from another folder come in at any gap.
            if !shown.iter().any(|s| s == from) {
                return Some((target, after));
            }
            let mut moved = shown.clone();
            crate::order::move_group(&mut moved, |s| s.as_str(), &group_of(&shelf, &picks, from), &target, after);
            (moved != shown).then_some((target, after))
        }
    };
    let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
    // The dragged id, already while it moves over: a tile from elsewhere may come in anywhere.
    drop.set_preload(true);
    drop.connect_motion({
        let (mark, nearest, elsewhere, can_order, spot_for) =
            (mark.clone(), nearest.clone(), elsewhere.clone(), can_order.clone(), spot_for.clone());
        move |target, x, y| {
            let id = target.value().and_then(|v| v.get::<String>().ok());
            if can_order()
                && let Some(gap) = nearest(x, y)
                && spot_for(id.as_deref(), gap.clone()).is_some()
            {
                mark.show(gap);
                return gdk::DragAction::MOVE;
            }
            mark.clear();
            if id.is_some_and(|id| elsewhere(&id)) { gdk::DragAction::MOVE } else { gdk::DragAction::empty() }
        }
    });
    drop.connect_leave({
        let mark = mark.clone();
        move |_| mark.clear()
    });
    drop.connect_drop({
        let (shelf, shown, picks, sync_picks, landing) =
            (shelf.clone(), shown.clone(), picks.clone(), sync_picks.clone(), landing.clone());
        move |_, value, x, y| {
            mark.clear();
            let Ok(from) = value.get::<String>() else { return false };
            let spot = can_order().then(|| nearest(x, y)).flatten().and_then(|gap| spot_for(Some(&from), gap));
            let group = group_of(&shelf, &picks, &from);
            let into = elsewhere(&from);
            if !into && spot.is_none() {
                return false;
            }
            // Into another folder, the picks are done. Moved within one, they stay picked.
            if into {
                picks.clear();
                sync_picks();
            }
            landing.items.borrow_mut().extend(group.iter().cloned());
            // After the drop is done: moving builds the grid again, with the dragged tiles in it.
            let (shelf, folder) = (shelf.clone(), shown.borrow().clone());
            glib::idle_add_local_once(move || {
                if into {
                    shelf.move_to_folder(&group, folder.as_deref());
                }
                if let Some((target, after)) = spot {
                    shelf.move_items(&group, &target, after);
                }
            });
            true
        }
    });
    area.as_ref().add_controller(drop);
}

/// A folder that is dragged, by name. A type of its own, so places for tiles do not take it.
#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "VoicenuxFolderDrag")]
struct FolderDrag(String);

/// Folders stay at the start of the grid, but move among themselves: a dragged folder lands in
/// the gap between two others, and both of them light up like for tiles.
pub fn drop_folders_on_grid(
    shelf: &Shared,
    area: &impl IsA<gtk::Widget>,
    grid: &gtk::FlowBox,
    folders: &Folders,
    landing: &Rc<Landing>,
) {
    let mark = GapMark::default();
    let nearest = {
        let (area, grid, folders) = (area.as_ref().downgrade(), grid.downgrade(), folders.clone());
        move |x: f64, y: f64| {
            let is_folder = |c: &gtk::Widget| folders.borrow().iter().any(|(t, _)| t.upcast_ref::<gtk::Widget>() == c);
            gap_at(&area.upgrade()?, &grid.upgrade()?, x, y, &is_folder)
        }
    };
    // Where a drop of the folder `name` would land. None when it would change nothing, like the
    // gaps right next to it: those do not light up.
    let spot_for = {
        let (shelf, folders) = (shelf.clone(), folders.clone());
        move |name: Option<&str>, gap: (Option<gtk::Widget>, Option<gtk::Widget>)| {
            let (c, after) = place(gap)?;
            let target =
                folders.borrow().iter().find(|(t, _)| t.upcast_ref::<gtk::Widget>() == &c).map(|(_, n)| n.clone())?;
            let Some(name) = name else { return Some((target, after)) };
            let list = shelf.folders();
            let mut moved = list.clone();
            crate::order::move_one(&mut moved, name, &target, after);
            (moved != list).then_some((target, after))
        }
    };
    let drop = gtk::DropTarget::new(FolderDrag::static_type(), gdk::DragAction::MOVE);
    // The dragged folder, already while it moves over.
    drop.set_preload(true);
    drop.connect_motion({
        let (mark, nearest, spot_for) = (mark.clone(), nearest.clone(), spot_for.clone());
        move |target, x, y| {
            let name = target.value().and_then(|v| v.get::<FolderDrag>().ok()).map(|FolderDrag(n)| n);
            match nearest(x, y) {
                Some(gap) if spot_for(name.as_deref(), gap.clone()).is_some() => {
                    mark.show(gap);
                    gdk::DragAction::MOVE
                }
                _ => {
                    mark.clear();
                    gdk::DragAction::empty()
                }
            }
        }
    });
    drop.connect_leave({
        let mark = mark.clone();
        move |_| mark.clear()
    });
    drop.connect_drop({
        let (shelf, landing) = (shelf.clone(), landing.clone());
        move |_, value, x, y| {
            mark.clear();
            let Ok(FolderDrag(name)) = value.get::<FolderDrag>() else { return false };
            let Some((target, after)) = nearest(x, y).and_then(|gap| spot_for(Some(&name), gap)) else { return false };
            *landing.moved_folder.borrow_mut() = Some(name.clone());
            // After the drop is done: moving builds the grid again, this folder too.
            let shelf = shelf.clone();
            glib::idle_add_local_once(move || shelf.move_folder(&name, &target, after));
            true
        }
    });
    area.as_ref().add_controller(drop);
}

/// How a folder tile fits in with the tiles around it: its card class, size and title class.
pub struct TileLook {
    pub card: &'static str,
    pub size: (i32, i32),
    pub title: &'static str,
}

/// A folder on the grid, as big as the other tiles: a glass folder with up to three pictures of
/// what is in it peeking out and showing through its front, and under it the name and `count`.
pub fn folder_tile(name: &str, icons: &[String], count: &str, look: &TileLook) -> gtk::FlowBoxChild {
    let tile = gtk::Box::new(gtk::Orientation::Vertical, 8);
    tile.add_css_class(look.card);
    tile.add_css_class("vn-folder");
    tile.set_size_request(look.size.0, look.size.1);
    // Drawn back to front, as high as the picture of a tile: the back with its tab, the
    // pictures, then the glass front.
    let art = gtk::Fixed::new();
    art.set_size_request(100, 84);
    art.set_halign(gtk::Align::Center);
    art.set_can_target(false);
    // The flap of the front tips out of it, and the pictures jump above it.
    art.set_overflow(gtk::Overflow::Visible);
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
        img.add_css_class("vn-peek");
        img.add_css_class(class);
        art.put(&img, *x, *y);
    }
    let front = folder_glass(100, 46, false);
    front.add_css_class("vn-folder-front");
    art.put(&front, 0.0, 38.0);
    tile.append(&art);
    let title = label(name, &[look.title]);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(14);
    tile.append(&title);
    tile.append(&label(count, &["dim-label", "caption"]));

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

/// What a folder on the grid does besides opening: it drags to another place among the folders,
/// takes tiles dropped on it, opens when one is held over it, and has a right-click menu to open,
/// rename or remove it.
#[allow(clippy::too_many_arguments)]
pub fn folder_actions(
    shelf: &Shared,
    child: &gtk::FlowBoxChild,
    name: &str,
    picks: &Rc<Picks>,
    sync_picks: &Rc<dyn Fn()>,
    open_folder: &Open,
    spring_to: &Open,
    landing: &Rc<Landing>,
) {
    let tile = child.child().and_then(|lift| lift.first_child());
    let drag = gtk::DragSource::new();
    drag.set_actions(gdk::DragAction::MOVE);
    let grabbed = Rc::new(Cell::new((0.0, 0.0)));
    drag.connect_prepare({
        let (grabbed, name) = (grabbed.clone(), name.to_string());
        move |_, x, y| {
            grabbed.set((x, y));
            Some(gdk::ContentProvider::for_value(&FolderDrag(name.clone()).to_value()))
        }
    });
    drag.connect_drag_begin({
        let (child, tile) = (child.downgrade(), tile.clone());
        move |src, _| {
            let (Some(child), Some(tile)) = (child.upgrade(), tile.as_ref()) else { return };
            let (x, y) = grabbed.get();
            src.set_icon(Some(&gtk::WidgetPaintable::new(Some(tile)).current_image()), x as i32, y as i32);
            child.add_css_class("vn-dragging");
        }
    });
    drag.connect_drag_end({
        let child = child.downgrade();
        move |_, _, _| {
            if let Some(c) = child.upgrade() {
                c.remove_css_class("vn-dragging");
            }
        }
    });
    child.add_controller(drag);

    // A tile held over it for a moment opens the folder, to be dropped at a place in it. While
    // it is held there the folder lifts and opens, and its pictures rise.
    let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
    let spring = Spring::default();
    let weak = child.downgrade();
    drop.connect_enter({
        let (weak, spring, spring_to, name) = (weak.clone(), spring.clone(), spring_to.clone(), name.to_string());
        move |_, _, _| {
            if let Some(c) = weak.upgrade() {
                c.add_css_class("vn-drop-into");
            }
            let (weak, spring_to, name) = (weak.clone(), spring_to.clone(), name.clone());
            spring.arm(move || {
                if let Some(c) = weak.upgrade() {
                    c.remove_css_class("vn-drop-into");
                }
                spring_to(Some(name));
            });
            gdk::DragAction::MOVE
        }
    });
    drop.connect_leave({
        let (weak, spring) = (weak.clone(), spring.clone());
        move |_| {
            spring.cancel();
            if let Some(c) = weak.upgrade() {
                c.remove_css_class("vn-drop-into");
            }
        }
    });
    drop.connect_drop({
        let (shelf, picks, sync_picks, name, landing) =
            (shelf.clone(), picks.clone(), sync_picks.clone(), name.to_string(), landing.clone());
        move |_, value, _, _| {
            spring.cancel();
            if let Some(c) = weak.upgrade() {
                c.remove_css_class("vn-drop-into");
            }
            let Ok(id) = value.get::<String>() else { return false };
            let ids = group_of(&shelf, &picks, &id);
            picks.clear();
            sync_picks();
            *landing.folder.borrow_mut() = Some(name.clone());
            // After the drop is done: moving builds the grid again, this folder too.
            let (shelf, name) = (shelf.clone(), name.clone());
            glib::idle_add_local_once(move || shelf.move_to_folder(&ids, Some(&name)));
            true
        }
    });
    child.add_controller(drop);

    let (shelf, name, open_folder, weak) = (shelf.clone(), name.to_string(), open_folder.clone(), child.downgrade());
    popup::context(child, move |x, y| {
        let Some(child) = weak.upgrade() else { return };
        let (s1, s2, n0, n1, n2, open_folder, parent) = (
            shelf.clone(),
            shelf.clone(),
            name.clone(),
            name.clone(),
            name.clone(),
            open_folder.clone(),
            child.downgrade(),
        );
        popup::show_list(
            &child,
            x,
            y,
            vec![
                popup::Item::new("folder-open-symbolic", t("Open"), move || open_folder(Some(n0.clone()))),
                popup::Item::new("document-edit-symbolic", t("Rename"), move || {
                    let Some(parent) = parent.upgrade() else { return };
                    let (shelf, old) = (s1.clone(), n1.clone());
                    ask_folder_name(&parent, t("Rename folder"), &n1, t("Rename"), move |new| {
                        if new != old && shelf.folders().contains(&new) {
                            shelf.note(fill(t("There is a folder called “{}” already"), &[&new]));
                            return;
                        }
                        shelf.rename_folder(&old, &new);
                    });
                }),
                popup::Item::new("user-trash-symbolic", t("Delete folder"), move || s2.remove_folder(&n2)).danger(),
            ],
        );
    });
}

/// Asks for the name of a folder. `done` gets it trimmed, and only when it is not empty.
pub fn ask_folder_name(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    current: &str,
    ok: &str,
    done: impl Fn(String) + 'static,
) {
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

/// Asks for a name over `parent`, then makes the folder.
pub fn new_folder_item(shelf: &Shared, parent: &gtk::Widget) -> popup::Item {
    let (shelf, parent) = (shelf.clone(), parent.downgrade());
    popup::Item::new("folder-new-symbolic", t("New folder"), move || {
        let Some(parent) = parent.upgrade() else { return };
        let shelf = shelf.clone();
        ask_folder_name(&parent, t("New folder"), "", t("Create"), move |name| {
            shelf.add_folder(&name);
        });
    })
}

/// The menu of the Move to folder button for picked tiles: each folder, none, or a new one.
pub fn move_to_menu(
    button: &gtk::MenuButton,
    shelf: &Shared,
    picks: &Rc<Picks>,
    sync_picks: &Rc<dyn Fn()>,
    landing: &Rc<Landing>,
) {
    let (shelf, picks, sync_picks, landing) = (shelf.clone(), picks.clone(), sync_picks.clone(), landing.clone());
    button.set_create_popup_func(move |b| {
        let folders = shelf.folders();
        let put = |folder: Option<String>| {
            let (shelf, picks, sync_picks, landing) =
                (shelf.clone(), picks.clone(), sync_picks.clone(), landing.clone());
            move || {
                let ids = picks.in_order(&shelf);
                picks.clear();
                sync_picks();
                *landing.folder.borrow_mut() = folder.clone();
                shelf.move_to_folder(&ids, folder.as_deref());
            }
        };
        let mut items: Vec<popup::Item> =
            folders.iter().map(|f| popup::Item::new("folder-symbolic", f.clone(), put(Some(f.clone())))).collect();
        if !folders.is_empty() {
            items.push(popup::Item::new("edit-clear-symbolic", t("No folder"), put(None)));
        }
        let (shelf, picks, sync_picks, landing, parent) =
            (shelf.clone(), picks.clone(), sync_picks.clone(), landing.clone(), b.downgrade());
        items.push(popup::Item::new("list-add-symbolic", t("New folder"), move || {
            let Some(parent) = parent.upgrade() else { return };
            let (shelf, picks, sync_picks, landing) =
                (shelf.clone(), picks.clone(), sync_picks.clone(), landing.clone());
            ask_folder_name(&parent, t("New folder"), "", t("Create"), move |name| {
                let name = shelf.add_folder(&name);
                let ids = picks.in_order(&shelf);
                picks.clear();
                sync_picks();
                *landing.folder.borrow_mut() = Some(name.clone());
                shelf.move_to_folder(&ids, Some(&name));
            });
        }));
        b.set_popover(Some(&popup::list(items)));
    });
}

/// The bar for picked tiles, at the bottom of the page: a cross that puts them down, how many,
/// then the buttons. The page shows it as a bottom bar of its toolbar view, across the whole page,
/// and reveals it while tiles are picked. In a narrow window the buttons wrap onto another line,
/// so it never makes the window wider.
pub fn picks_bar(done: &gtk::Button, count: &gtk::Label, buttons: &[&gtk::Widget]) -> gtk::Box {
    done.set_icon_name("window-close-symbolic");
    done.add_css_class("flat");
    done.add_css_class("circular");
    done.set_valign(gtk::Align::Center);
    done.set_tooltip_text(Some(t("Stop selecting (Esc)")));
    let wrap = adw::WrapBox::builder().child_spacing(8).line_spacing(6).align(1.0).hexpand(true).build();
    for b in buttons {
        wrap.append(*b);
    }
    count.set_valign(gtk::Align::Center);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    bar.add_css_class("vn-picks-bar");
    bar.append(done);
    bar.append(count);
    bar.append(&wrap);
    bar
}

/// Over the grid while a folder is open: the way back up, and the name of the folder.
pub struct PathBar {
    pub widget: gtk::Box,
    pub up: gtk::Button,
    pub here: gtk::Label,
}

/// `all` names the top, like "All sounds", and `tip` says how to get there.
pub fn path_bar(all: &str, tip: &str) -> PathBar {
    let up = gtk::Button::new();
    up.set_child(Some(&adw::ButtonContent::builder().icon_name("go-previous-symbolic").label(all).build()));
    up.add_css_class("flat");
    up.add_css_class("vn-up");
    up.set_tooltip_text(Some(tip));
    let here = label("", &["vn-folder-title"]);
    here.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    widget.append(&up);
    widget.append(&label("›", &["dim-label"]));
    widget.append(&gtk::Image::from_icon_name("folder-symbolic"));
    widget.append(&here);
    widget.set_margin_start(16);
    widget.set_margin_end(20);
    widget.set_margin_top(10);
    widget.set_visible(false);
    PathBar { widget, up, here }
}

/// A tile dropped on the way back up leaves its folder, and one held over it goes up along, to be
/// dropped elsewhere.
pub fn up_takes_drops(
    up: &gtk::Button,
    shelf: &Shared,
    picks: &Rc<Picks>,
    sync_picks: &Rc<dyn Fn()>,
    spring_to: &Open,
) {
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
        let (shelf, picks, sync_picks, up) = (shelf.clone(), picks.clone(), sync_picks.clone(), up.downgrade());
        move |_, value, _, _| {
            spring.cancel();
            if let Some(up) = up.upgrade() {
                up.remove_css_class("vn-drop-into");
            }
            let Ok(id) = value.get::<String>() else { return false };
            let ids = group_of(&shelf, &picks, &id);
            picks.clear();
            sync_picks();
            let shelf = shelf.clone();
            glib::idle_add_local_once(move || shelf.move_to_folder(&ids, None));
            true
        }
    });
    up.add_controller(out);
}

/// Ctrl+click picks a tile, Shift+click all tiles from the last pick to this one, like in a file
/// manager.
pub fn pick_clicks(grid: &gtk::FlowBox, item_of: &ItemOf, picks: &Rc<Picks>, sync_picks: &Rc<dyn Fn()>) {
    let click = gtk::GestureClick::new();
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    click.connect_pressed({
        let (grid, item_of, picks, sync_picks) = (grid.downgrade(), item_of.clone(), picks.clone(), sync_picks.clone());
        move |g, _, x, y| {
            let mods = g.current_event_state();
            let (ctrl, shift) =
                (mods.contains(gdk::ModifierType::CONTROL_MASK), mods.contains(gdk::ModifierType::SHIFT_MASK));
            let Some(grid) = grid.upgrade() else { return };
            let Some(child) = (ctrl || shift).then(|| grid.child_at_pos(x as i32, y as i32)).flatten() else { return };
            let Some(id) = item_of(child.upcast_ref()) else { return };
            g.set_state(gtk::EventSequenceState::Claimed);
            let anchor = picks.anchor.borrow().clone();
            match anchor.filter(|_| shift) {
                Some(anchor) => {
                    let (mut on, mut row) = (false, Vec::new());
                    for i in shown_items(&grid, &item_of) {
                        let edge = i == anchor || i == id;
                        if edge || on {
                            row.push(i.clone());
                        }
                        if edge && anchor != id {
                            on = !on;
                        }
                    }
                    picks.ids.borrow_mut().extend(row);
                }
                None => picks.toggle(&id),
            }
            sync_picks();
        }
    });
    grid.add_controller(click);
}

/// A page with folders, and how it goes up out of one. False when it shows none.
type Up = (glib::WeakRef<gtk::Widget>, Rc<dyn Fn() -> bool>);

thread_local! {
    /// How each page with folders goes up out of one, for the back button of the mouse.
    static UP: RefCell<Vec<Up>> = RefCell::default();
}

/// The back button of the mouse goes up out of a folder first, like in a file manager, and only
/// then back to the page before. The window asks [`up_on`] before it goes back a page.
pub fn mouse_back(page: &impl IsA<gtk::Widget>, shown: &Rc<RefCell<Option<String>>>, open_folder: &Open) {
    let (shown, open_folder) = (shown.clone(), open_folder.clone());
    let up: Rc<dyn Fn() -> bool> = Rc::new(move || {
        if shown.borrow().is_none() {
            return false;
        }
        open_folder(None);
        true
    });
    UP.with(|u| u.borrow_mut().push((page.as_ref().downgrade(), up)));
}

/// Goes up out of the folder that `page` shows, or a page in it. False when it shows none.
pub fn up_on(page: &gtk::Widget) -> bool {
    let up = UP.with(|u| {
        u.borrow()
            .iter()
            .find(|(w, _)| w.upgrade().is_some_and(|w| &w == page || w.is_ancestor(page)))
            .map(|(_, up)| up.clone())
    });
    up.is_some_and(|up| up())
}
