//! Nicer scrolling. Mouse wheels glide instead of jumping (touchpads already scroll smoothly), and
//! a middle click starts autoscroll: move the mouse up or down to scroll, click again to stop.
//! Hold the middle button and drag to scroll only while it is held. Like in browsers, a mark
//! stays where the click was, and the mouse pointer turns into the scroll cursors of the theme.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{cairo, gdk, glib};

/// How fast a glide reaches its target: the time it takes for about two thirds of the way.
const GLIDE: f64 = 0.07;
/// Autoscroll does nothing this close to where the middle click was. Like Chrome.
const DEAD_ZONE: f64 = 15.0;
/// Size of the mark where the middle click was.
const MARK: i32 = 28;
/// How far the value may be off the one we set without something else having moved it. Lists
/// like the picture grid put it on their own steps, a pixel or two off.
const SLACK: f64 = 6.0;

struct Scroller {
    /// Weak: the scrolled window owns the controllers that own this.
    sw: glib::WeakRef<gtk::ScrolledWindow>,
    adj: gtk::Adjustment,
    target: Cell<f64>,
    /// The value we set last, before a list rounds it. Glides go on from here, or a list would
    /// round every small step back. Something else moved it when it is far off: the glide stops.
    set: Cell<f64>,
    /// Autoscroll: where the middle click was, and where the pointer is.
    anchor: Cell<Option<f64>>,
    pointer: Cell<f64>,
    pan: Cell<Option<Pan>>,
    cursors: [gdk::Cursor; 3],
    /// The mark and the overlay it sits in.
    mark: RefCell<Option<(gtk::Overlay, gtk::DrawingArea)>>,
    /// While the middle button is held, GTK takes the cursor from the widget that got the press
    /// and ignores the one of the scrolled window. So that widget gets it too, until the release.
    /// Kept with its own cursor, to give it back.
    held: RefCell<Option<(gtk::Widget, Option<gdk::Cursor>)>>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
    last_frame: Cell<i64>,
}

impl Scroller {
    fn max(&self) -> f64 {
        (self.adj.upper() - self.adj.page_size()).max(self.adj.lower())
    }

    fn scrollable(&self) -> bool {
        self.max() > self.adj.lower()
    }

    fn set_value(&self, v: f64) {
        self.set.set(v);
        self.adj.set_value(v);
    }

    fn stop_auto(&self) {
        if self.anchor.take().is_some() {
            self.target.set(self.adj.value());
            self.pan.set(None);
            if let Some(sw) = self.sw.upgrade() {
                sw.set_cursor(None);
            }
            if let Some((overlay, mark)) = self.mark.take() {
                overlay.remove_overlay(&mark);
            }
            self.release();
        }
    }

    /// The middle button is up: the widget that got the press gets its own cursor back.
    fn release(&self) {
        if let Some((w, cursor)) = self.held.take() {
            w.set_cursor(cursor.as_ref());
        }
    }

    /// Puts the mark at a point of the scrolled window, in the nearest overlay around it.
    fn show_mark(&self, x: f64, y: f64) {
        let Some(sw) = self.sw.upgrade() else { return };
        let Some(overlay) = sw.ancestor(gtk::Overlay::static_type()).and_downcast::<gtk::Overlay>() else { return };
        // A popover has its own surface: an overlay of the window behind it is no use.
        if overlay.native() != sw.native() {
            return;
        }
        let Some(p) = sw.compute_point(&overlay, &gtk::graphene::Point::new(x as f32, y as f32)) else { return };
        let mark = gtk::DrawingArea::builder()
            .content_width(MARK)
            .content_height(MARK)
            .can_target(false)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_start((p.x() as i32 - MARK / 2).max(0))
            .margin_top((p.y() as i32 - MARK / 2).max(0))
            .css_classes(["vn-autoscroll-mark"])
            .build();
        mark.set_draw_func(|_, cr, w, _| draw_mark(cr, w as f64));
        overlay.add_overlay(&mark);
        *self.mark.borrow_mut() = Some((overlay, mark));
    }

    fn set_cursor(&self, pan: Pan) {
        if self.pan.get() != Some(pan)
            && let Some(sw) = self.sw.upgrade()
        {
            self.pan.set(Some(pan));
            sw.set_cursor(Some(&self.cursors[pan as usize]));
            if let Some((w, _)) = self.held.borrow().as_ref() {
                w.set_cursor(Some(&self.cursors[pan as usize]));
            }
        }
    }

    /// One frame of autoscroll or glide. False when there is nothing left to do.
    fn step(&self, dt: f64) -> bool {
        if (self.adj.value() - self.set.get()).abs() > SLACK {
            // A scrollbar drag or the keyboard took over.
            self.target.set(self.adj.value());
            self.set.set(self.adj.value());
        }
        if let Some(anchor) = self.anchor.get() {
            let d = self.pointer.get() - anchor;
            let far = d.abs() > DEAD_ZONE;
            self.set_cursor(if !far {
                Pan::Both
            } else if d < 0.0 {
                Pan::Up
            } else {
                Pan::Down
            });
            // Pixels a second, like Chrome: 0.04 × distance^2.2.
            let speed = if far { d.signum() * 0.04 * d.abs().powf(2.2) } else { 0.0 };
            let v = (self.set.get() + speed * dt).clamp(self.adj.lower(), self.max());
            self.target.set(v);
            self.set_value(v);
            return true;
        }
        let at = self.set.get();
        let left = self.target.get() - at;
        if left.abs() < 0.5 {
            self.set_value(self.target.get());
            return false;
        }
        self.set_value(at + left * (1.0 - (-dt / GLIDE).exp()));
        true
    }

    fn run(self: &Rc<Self>) {
        let Some(sw) = self.sw.upgrade() else { return };
        if self.tick.borrow().is_some() {
            return;
        }
        self.set.set(self.adj.value());
        self.last_frame.set(0);
        let s = self.clone();
        let id = sw.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            let last = s.last_frame.replace(now);
            let dt = if last == 0 { 1.0 / 60.0 } else { ((now - last) as f64 / 1e6).min(0.05) };
            if s.step(dt) {
                return glib::ControlFlow::Continue;
            }
            // Returning Break removes the callback, so only forget its id.
            drop(s.tick.take());
            glib::ControlFlow::Break
        });
        *self.tick.borrow_mut() = Some(id);
    }
}

/// Adds both to a scrolled window, or to the first one inside `widget`.
pub fn smooth_scroll(widget: &impl IsA<gtk::Widget>) {
    let Some(sw) = find_scroll(widget.as_ref()) else { return };
    let s = Rc::new(Scroller {
        adj: sw.vadjustment(),
        sw: sw.downgrade(),
        target: Cell::new(0.0),
        set: Cell::new(0.0),
        anchor: Cell::new(None),
        pointer: Cell::new(0.0),
        pan: Cell::new(None),
        cursors: [Pan::Both, Pan::Up, Pan::Down].map(pan_cursor),
        mark: RefCell::default(),
        held: RefCell::default(),
        tick: RefCell::default(),
        last_frame: Cell::new(0),
    });
    wheel(&sw, &s);
    autoscroll(&sw, &s);
}

/// Events from a popover inside the scrolled window belong to the popover.
fn own(c: &impl IsA<gtk::EventController>) -> bool {
    let native = c.widget().and_then(|w| w.native()).and_then(|n| n.surface());
    c.current_event().and_then(|e| e.surface()) == native
}

fn wheel(sw: &gtk::ScrolledWindow, s: &Rc<Scroller>) {
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    let s = s.clone();
    scroll.connect_scroll(move |c, _, dy| {
        s.stop_auto();
        let mods = gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK;
        if c.current_event_state().intersects(mods) || !own(c) || !s.scrollable() {
            return glib::Propagation::Proceed;
        }
        if c.unit() != gdk::ScrollUnit::Wheel {
            // A touchpad over a slider scrolls the page, like in a browser, instead of moving
            // the slider. Elsewhere GTK scrolls it, with its own swing.
            if !over_slider(c) {
                return glib::Propagation::Proceed;
            }
            s.target.set((s.adj.value() + dy).clamp(s.adj.lower(), s.max()));
            s.set_value(s.target.get());
            return glib::Propagation::Stop;
        }
        // Keep going from where the glide is heading, so fast wheel turns add up.
        let from = if s.tick.borrow().is_some() { s.target.get() } else { s.adj.value() };
        let to = (from + dy * s.adj.page_size().powf(2.0 / 3.0)).clamp(s.adj.lower(), s.max());
        s.target.set(to);
        if c.widget().is_some_and(|w| w.settings().is_gtk_enable_animations()) {
            s.run();
        } else {
            s.set_value(to);
        }
        glib::Propagation::Stop
    });
    sw.add_controller(scroll);
}

fn autoscroll(sw: &gtk::ScrolledWindow, s: &Rc<Scroller>) {
    // Any button: a click while autoscroll runs stops it.
    let drag = gtk::GestureDrag::new();
    drag.set_button(0);
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    // Whether this press started autoscroll (a press that stops it does nothing more).
    let started = Rc::new(Cell::new(false));
    drag.connect_drag_begin({
        let s = s.clone();
        let started = started.clone();
        move |g, x, y| {
            started.set(false);
            if s.anchor.get().is_some() {
                s.stop_auto();
                g.set_state(gtk::EventSequenceState::Claimed);
                return;
            }
            if g.current_button() != gdk::BUTTON_MIDDLE || !own(g) || !s.scrollable() || over_text(g, x, y) {
                g.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            g.set_state(gtk::EventSequenceState::Claimed);
            started.set(true);
            s.anchor.set(Some(y));
            s.pointer.set(y);
            if let Some(w) = g.widget().and_then(|sw| sw.pick(x, y, gtk::PickFlags::DEFAULT)) {
                let own = w.cursor();
                *s.held.borrow_mut() = Some((w, own));
            }
            s.set_cursor(Pan::Both);
            s.show_mark(x, y);
            s.run();
        }
    });
    drag.connect_drag_update({
        let s = s.clone();
        let started = started.clone();
        move |g, _, dy| {
            if started.get()
                && let Some((_, y)) = g.start_point()
            {
                s.pointer.set(y + dy);
            }
        }
    });
    drag.connect_drag_end({
        let s = s.clone();
        move |_, dx, dy| {
            // Held and dragged: stop on release. A plain click keeps it going.
            if started.get() && dx.hypot(dy) > DEAD_ZONE {
                s.stop_auto();
            }
            s.release();
        }
    });
    sw.add_controller(drag);

    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    motion.connect_motion({
        let s = s.clone();
        move |_, _, y| s.pointer.set(y)
    });
    sw.add_controller(motion);

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let s = s.clone();
        move |_, key, _, _| {
            if key == gdk::Key::Escape && s.anchor.get().is_some() {
                s.stop_auto();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    sw.add_controller(keys);

    let s = s.clone();
    sw.connect_unmap(move |_| s.stop_auto());
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pan {
    Both,
    Up,
    Down,
}

/// The scroll cursors of the cursor theme, the ones browsers use too.
fn pan_cursor(pan: Pan) -> gdk::Cursor {
    let name = match pan {
        Pan::Both => "all-scroll",
        Pan::Up => "n-resize",
        Pan::Down => "s-resize",
    };
    gdk::Cursor::from_name(name, gdk::Cursor::from_name("default", None).as_ref()).expect("no cursor")
}

/// The mark where the middle click was: a round badge with a dot and arrows up and down.
fn draw_mark(cr: &cairo::Context, size: f64) {
    // Draw in a 24 by 24 box.
    cr.scale(size / 24.0, size / 24.0);
    let c = 12.0;
    cr.arc(c, c, 10.5, 0.0, std::f64::consts::TAU);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.92);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.6);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.8);
    cr.arc(c, c, 1.6, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    for dir in [-1.0, 1.0] {
        cr.move_to(c, c + dir * 8.0);
        cr.line_to(c - 3.6, c + dir * 3.8);
        cr.line_to(c + 3.6, c + dir * 3.8);
        cr.close_path();
        let _ = cr.fill();
    }
}

/// Whether the pointer of a scroll is over a slider in the scrolled window.
fn over_slider(c: &gtk::EventControllerScroll) -> bool {
    let (Some(sw), Some(event)) = (c.widget(), c.current_event()) else { return false };
    let (Some(native), Some((x, y))) = (sw.native(), event.position()) else { return false };
    let (tx, ty) = native.surface_transform();
    let point = gtk::graphene::Point::new((x - tx) as f32, (y - ty) as f32);
    let Some(p) = native.compute_point(&sw, &point) else { return false };
    let mut w = sw.pick(p.x() as f64, p.y() as f64, gtk::PickFlags::DEFAULT);
    while let Some(widget) = w {
        if widget.is::<gtk::Range>() {
            return true;
        }
        if widget == sw {
            break;
        }
        w = widget.parent();
    }
    false
}

/// A middle click on text pastes, so it must not start autoscroll there.
fn over_text(g: &gtk::GestureDrag, x: f64, y: f64) -> bool {
    let Some(sw) = g.widget() else { return false };
    let mut w = sw.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(widget) = w {
        if widget.is::<gtk::Text>() || widget.is::<gtk::TextView>() {
            return true;
        }
        if widget == sw {
            break;
        }
        w = widget.parent();
    }
    false
}

fn find_scroll(w: &gtk::Widget) -> Option<gtk::ScrolledWindow> {
    if let Some(sw) = w.downcast_ref::<gtk::ScrolledWindow>() {
        return Some(sw.clone());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(sw) = find_scroll(&c) {
            return Some(sw);
        }
        child = c.next_sibling();
    }
    None
}
