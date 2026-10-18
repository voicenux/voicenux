//! Menus and dialogs that act like on other desktops. GTK eats the click that closes a menu, so
//! a click on another button only closes the menu. Here the click also does what it hit, like in
//! KDE or a browser: a right click on another pad opens its menu right away. A click on the dark
//! area around a dialog closes the dialog, or makes it pulse when closing would lose something.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib};

/// A menu that closed on a press next to it. GDK swallowed that press, but its release still
/// comes to the window and does what the press would have done.
struct Pending {
    surface: gdk::Surface,
    /// Where the press was, on the surface.
    x: f64,
    y: f64,
    /// What the menu hangs on: a click there only closes the menu, like a second click.
    opener: Option<glib::WeakRef<gtk::Widget>>,
    when: Instant,
}

type Context = Rc<dyn Fn(f64, f64)>;

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    /// Widgets with a right-click menu, so a right click that closes another menu opens theirs.
    static CONTEXT: RefCell<Vec<(glib::WeakRef<gtk::Widget>, Context)>> = const { RefCell::new(Vec::new()) };
}

/// How far a press may move before its release: further, and it was a drag, not a click.
const SLOP: f64 = 8.0;

/// Sets up a popover: it opens with a little bounce, has no arrow, and a click next to it goes on
/// to what it hits.
pub fn menu(p: &gtk::Popover) {
    if p.has_css_class("vn-menu") {
        return;
    }
    p.add_css_class("vn-menu");
    p.set_has_arrow(false);
    // A new animation name starts the animation again, the same name would not.
    let flip = Cell::new(false);
    p.connect_map(move |p| {
        let a = !flip.replace(!flip.get());
        p.remove_css_class(if a { "vn-bounce-b" } else { "vn-bounce-a" });
        p.add_css_class(if a { "vn-bounce-a" } else { "vn-bounce-b" });
        // GTK passes hover and presses in a menu on to what it hangs on. A tile under its menu
        // would squish and spring back on every click in the menu, so it knows the menu is open.
        if let Some(parent) = p.parent() {
            parent.add_css_class("vn-menu-open");
        }
    });
    p.connect_unmap(|p| {
        if let Some(parent) = p.parent() {
            parent.remove_css_class("vn-menu-open");
            parent.remove_css_class("vn-menu-hover");
        }
    });
    // The pointer in the menu counts as on the tile too, so the tile lit up whenever the pointer
    // came back into its menu. It knows when the pointer is in the menu, and stays dark then.
    let inside = gtk::EventControllerMotion::new();
    inside.connect_contains_pointer_notify(|m| {
        let Some(parent) = m.widget().and_then(|p| p.parent()) else { return };
        if m.contains_pointer() {
            parent.add_css_class("vn-menu-hover");
        } else {
            parent.remove_css_class("vn-menu-hover");
        }
    });
    p.add_controller(inside);
    p.connect_closed(|p| {
        let Some(pointer) = p.display().default_seat().and_then(|s| s.pointer()) else { return };
        let (Some(surface), x, y) = pointer.surface_at_position() else { return };
        // Closed from inside, like a click on one of its buttons.
        if p.surface().as_ref() == Some(&surface) {
            return;
        }
        let opener = p.parent().map(|w| w.downgrade());
        PENDING
            .with(|c| *c.borrow_mut() = Some(Pending { surface: surface.clone(), x, y, opener, when: Instant::now() }));
        // GTK closes only the menu on top. A click outside of a menu in a menu, like the list of a
        // drop-down in the menu of a sound, closes the outer one too, unless it landed on it.
        let buttons =
            gdk::ModifierType::BUTTON1_MASK | gdk::ModifierType::BUTTON2_MASK | gdk::ModifierType::BUTTON3_MASK;
        if !pointer.modifier_state().intersects(buttons) {
            return;
        }
        let mut w = p.parent();
        while let Some(widget) = w {
            if let Some(outer) = widget.downcast_ref::<gtk::Popover>() {
                if outer.is_visible() && outer.surface().as_ref() != Some(&surface) {
                    outer.popdown();
                }
                break;
            }
            w = widget.parent();
        }
    });
}

/// Keeps clicks in a popover to itself. They travel on to the widget it hangs on, and a pad or
/// card there would squish or play.
pub fn keep_clicks(popover: &gtk::Popover) {
    let c = gtk::EventControllerLegacy::new();
    c.connect_event(|_, e| {
        use gdk::EventType::*;
        if matches!(e.event_type(), ButtonPress | ButtonRelease | TouchBegin | TouchUpdate | TouchEnd) {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    popover.add_controller(c);
}

/// One line of a [`show_list`] menu.
pub struct Item {
    pub icon: &'static str,
    pub label: String,
    pub run: Box<dyn Fn()>,
    /// Red, for things that remove.
    pub danger: bool,
}

impl Item {
    pub fn new(icon: &'static str, label: impl Into<String>, run: impl Fn() + 'static) -> Self {
        Self { icon, label: label.into(), run: Box::new(run), danger: false }
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

/// A menu: a list of buttons with icons.
pub fn list(items: Vec<Item>) -> gtk::Popover {
    let p = gtk::Popover::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    list.add_css_class("vn-menu-list");
    for item in items {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.append(&gtk::Image::from_icon_name(item.icon));
        let l = gtk::Label::new(Some(&item.label));
        l.set_xalign(0.0);
        row.append(&l);
        let b = gtk::Button::builder().child(&row).css_classes(["flat", "vn-menu-item"]).build();
        if item.danger {
            b.add_css_class("vn-danger");
        }
        let weak = p.downgrade();
        b.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.popdown();
            }
            (item.run)();
        });
        list.append(&b);
    }
    p.set_child(Some(&list));
    menu(&p);
    p
}

/// A right-click menu. It opens with its corner at the point of `parent`, like menus on other
/// desktops, and is gone after it closes.
pub fn show_list(parent: &impl IsA<gtk::Widget>, x: f64, y: f64, items: Vec<Item>) {
    let p = list(items);
    keep_clicks(&p);
    p.set_halign(gtk::Align::Start);
    p.set_position(gtk::PositionType::Bottom);
    p.set_parent(parent);
    p.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    // An item can remove what the menu hangs on, like Duplicate in Voicelab, which builds the list
    // again. GTK frees that widget with the menu still in it, so let go of it right then.
    let weak = p.downgrade();
    let on_destroy = parent.as_ref().connect_destroy(move |_| {
        if let Some(p) = weak.upgrade() {
            p.unparent();
        }
    });
    let on_destroy = Cell::new(Some(on_destroy));
    // Later: the closed handler of `menu` still asks what it hangs on.
    p.connect_closed(move |p| {
        let p = p.clone();
        let on_destroy = on_destroy.take();
        glib::idle_add_local_once(move || {
            if let Some(parent) = p.parent() {
                if let Some(id) = on_destroy {
                    parent.disconnect(id);
                }
                p.unparent();
            }
        });
    });
    p.popup();
}

/// Sets up every popover inside `widget` that GTK made, like the lists of drop-downs.
pub fn menus_in(widget: &impl IsA<gtk::Widget>) {
    let mut child = widget.as_ref().first_child();
    while let Some(c) = child {
        if let Some(p) = c.downcast_ref::<gtk::Popover>() {
            menu(p);
        }
        menus_in(&c);
        child = c.next_sibling();
    }
}

/// Gives a widget a right-click menu: `open` gets the point in the widget. The Menu key and
/// Shift+F10 open it too, in the middle of the widget.
pub fn context(widget: &impl IsA<gtk::Widget>, open: impl Fn(f64, f64) + 'static) {
    let w = widget.as_ref();
    let open: Context = Rc::new(open);
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    click.connect_pressed({
        let open = open.clone();
        move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            open(x, y);
        }
    });
    w.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let (open, w) = (open.clone(), w.downgrade());
        move |_, key, _, mods| {
            let shift_f10 = key == gdk::Key::F10 && mods.contains(gdk::ModifierType::SHIFT_MASK);
            match w.upgrade() {
                Some(w) if key == gdk::Key::Menu || shift_f10 => {
                    open(w.width() as f64 / 2.0, w.height() as f64 / 2.0);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        }
    });
    w.add_controller(keys);
    CONTEXT.with(|c| {
        let mut list = c.borrow_mut();
        list.retain(|(w, _)| w.upgrade().is_some());
        list.push((w.downgrade(), open));
    });
}

/// Watches the window for the clicks above. Call once.
pub fn watch(window: &adw::ApplicationWindow) {
    let legacy = gtk::EventControllerLegacy::new();
    legacy.set_propagation_phase(gtk::PropagationPhase::Capture);
    let win = window.downgrade();
    legacy.connect_event(move |_, event| {
        match event.event_type() {
            gdk::EventType::ButtonPress => {
                // A press that reaches us was not swallowed by a closing menu.
                PENDING.with(|c| c.borrow_mut().take());
                if let Some(window) = win.upgrade() {
                    leave_text_field(&window, event);
                }
            }
            gdk::EventType::ButtonRelease => {
                if let Some(p) = PENDING.with(|c| c.borrow_mut().take()) {
                    replay(&p, event);
                }
            }
            _ => {}
        }
        glib::Propagation::Proceed
    });
    window.add_controller(legacy);
    // Every dialog that opens gets its dark surroundings to listen for a click.
    window.connect_visible_dialog_notify(|w| {
        if let Some(dialog) = w.visible_dialog() {
            close_on_dimming(&dialog);
            // libadwaita may make the area a moment later, when it lays the dialog out.
            glib::idle_add_local_once(move || close_on_dimming(&dialog));
        }
    });
}

/// A click on an empty spot leaves the text field you typed in, like in a browser. GTK keeps the
/// focus there until something else takes it.
fn leave_text_field(window: &adw::ApplicationWindow, event: &gdk::Event) {
    let Some(focus) = GtkWindowExt::focus(window).filter(|f| f.is::<gtk::Text>()) else { return };
    if window.visible_dialog().is_some() || event.surface() != window.surface() {
        return;
    }
    let Some((x, y)) = event.position() else { return };
    let (tx, ty) = window.surface_transform();
    // The field the text belongs to, like the search entry around it.
    let field = focus.parent().unwrap_or_else(|| focus.clone());
    let hit = window.pick(x - tx, y - ty, gtk::PickFlags::DEFAULT);
    if hit.is_some_and(|h| h == field || h.is_ancestor(&field)) {
        return;
    }
    // After the press: a button or a pad that takes the focus has taken it then.
    let window = window.downgrade();
    glib::idle_add_local_once(move || {
        if let Some(window) = window.upgrade()
            && GtkWindowExt::focus(&window).as_ref() == Some(&focus)
        {
            GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
        }
    });
}

/// A click on the dark area around a dialog closes it, or makes it glow when closing would lose
/// something. The area is libadwaita's own, made when the dialog first shows.
fn close_on_dimming(dialog: &adw::Dialog) {
    let Some(dimming) = find_css(dialog.upcast_ref(), "dimming") else { return };
    if dimming.has_css_class("vn-closes") {
        return;
    }
    dimming.add_css_class("vn-closes");
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    let start = Rc::new(Cell::new((0.0, 0.0)));
    click.connect_pressed({
        let start = start.clone();
        move |_, _, x, y| start.set((x, y))
    });
    let dialog = dialog.downgrade();
    click.connect_released(move |_, n_press, x, y| {
        let (sx, sy) = start.get();
        let Some(dialog) = dialog.upgrade() else { return };
        // A drag moves the window, a double click maximizes it.
        if n_press != 1 || (x - sx).hypot(y - sy) > SLOP {
            return;
        }
        if dialog.has_css_class("vn-stay") || !dialog.can_close() {
            nudge(&dialog);
        } else {
            dialog.close();
        }
    });
    dimming.add_controller(click);
}

/// Does what the press that closed a menu would have done, now that it is released.
fn replay(p: &Pending, release: &gdk::Event) {
    let Some((x, y)) = release.position() else { return };
    if p.when.elapsed() > Duration::from_millis(1500)
        || release.surface().as_ref() != Some(&p.surface)
        || (x - p.x).hypot(y - p.y) > SLOP
    {
        return;
    }
    let Some(native) = gtk::Native::for_surface(&p.surface) else { return };
    let (tx, ty) = native.surface_transform();
    let (x, y) = (x - tx, y - ty);
    let Some(hit) = native.pick(x, y, gtk::PickFlags::DEFAULT) else { return };
    if let Some(opener) = p.opener.as_ref().and_then(|o| o.upgrade())
        && (hit == opener || hit.is_ancestor(&opener))
    {
        return;
    }
    let button = release.downcast_ref::<gdk::ButtonEvent>().map(|b| b.button()).unwrap_or(0);
    // Later: GTK is still busy with this release.
    glib::idle_add_local_once(move || match button {
        gdk::BUTTON_PRIMARY => activate(&hit),
        gdk::BUTTON_SECONDARY => open_context(&hit, native.upcast_ref(), x, y),
        _ => {}
    });
}

/// Clicks the first thing at or around `hit` that takes clicks.
fn activate(hit: &gtk::Widget) {
    let mut w = Some(hit.clone());
    while let Some(widget) = w {
        if !widget.is_sensitive() || widget.is::<gtk::Native>() || widget.is::<gtk::Range>() {
            return;
        }
        if let Some(l) = widget.downcast_ref::<gtk::EditableLabel>() {
            l.start_editing();
            return;
        }
        if widget.is::<gtk::Text>() {
            widget.grab_focus();
            return;
        }
        if widget.is::<gtk::Button>()
            || widget.is::<gtk::Switch>()
            || widget.is::<gtk::CheckButton>()
            || widget.is::<gtk::FlowBoxChild>()
            || widget.is::<gtk::ListBoxRow>()
        {
            widget.activate();
            return;
        }
        w = widget.parent();
    }
}

/// Opens the right-click menu of what is at or around `hit`.
fn open_context(hit: &gtk::Widget, native: &gtk::Widget, x: f64, y: f64) {
    let found = CONTEXT.with(|c| {
        let list = c.borrow();
        let mut w = Some(hit.clone());
        while let Some(widget) = w {
            if let Some((_, open)) = list.iter().find(|(r, _)| r.upgrade().as_ref() == Some(&widget)) {
                return Some((widget, open.clone()));
            }
            w = widget.parent();
        }
        None
    });
    if let Some((widget, open)) = found
        && let Some(p) = native.compute_point(&widget, &gtk::graphene::Point::new(x as f32, y as f32))
    {
        open(p.x() as f64, p.y() as f64);
    }
}

/// Makes a dialog pulse and glow: it stays open, it needs a button.
pub fn nudge(dialog: &adw::Dialog) {
    let Some(sheet) = find_css(dialog.upcast_ref(), "sheet") else { return };
    // A new animation name starts the animation again, the same name would not.
    let (old, new) =
        if sheet.has_css_class("vn-nudge-a") { ("vn-nudge-a", "vn-nudge-b") } else { ("vn-nudge-b", "vn-nudge-a") };
    sheet.remove_css_class(old);
    sheet.add_css_class(new);
}

fn find_css(w: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    let mut child = w.first_child();
    while let Some(c) = child {
        if c.css_name() == name {
            return Some(c);
        }
        if let Some(found) = find_css(&c, name) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}
