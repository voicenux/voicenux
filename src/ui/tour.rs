//! Tips on the first visit of a page: bubbles that point at things one after the other, like the
//! right click on an empty spot, which nothing on screen shows. Each page shows them once, when it
//! is on screen and no dialog is open. Settings can show them again.
//!
//! The bubbles live in the window, on the layer over the pages: a popover is a surface of its own,
//! the desktop puts it over other apps and cuts it off at the edge of the screen. They follow what
//! they point at every frame, so scrolling or a new window size takes them along. The last one
//! ends in confetti.

use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{cairo, glib};

use super::label;
use crate::i18n::{fill, t};
use crate::state::St;

/// An area in what a tip points at. Asked every frame, so it can follow scrolling and a new
/// window size. None when there is none in sight right now.
pub type Area = Rc<dyn Fn() -> Option<gtk::graphene::Rect>>;

/// One bubble: what it points at, an area in it or all of it, and what it says, in the language
/// of the app already. An area also gets a mark that shows it.
pub struct Tip {
    pub at: gtk::Widget,
    pub area: Option<Area>,
    pub title: &'static str,
    pub text: &'static str,
}

/// The name of the layer over the pages, in the window.
pub const LAYER: &str = "vn-layer";

/// A page by name, its widget, and what makes its tips.
type Page = (&'static str, glib::WeakRef<gtk::Widget>, Rc<dyn Fn() -> Vec<Tip>>);

thread_local! {
    /// The tips of each page, to show them again on request.
    static PAGES: RefCell<Vec<Page>> = RefCell::default();
}

/// Shows the tips of `page` from tip `from` on, counted from 0, also when they were seen. For
/// screenshots: `voicenux --dialog tour:voices:2`.
pub fn show_from(page: &str, from: usize) -> bool {
    let found = PAGES
        .with(|p| p.borrow().iter().find(|(name, ..)| *name == page).map(|(_, w, tips)| (w.upgrade(), tips.clone())));
    let Some((Some(w), tips)) = found else { return false };
    let Some(layer) = layer_of(&w) else { return false };
    show(&layer, Rc::new(tips()), from);
    true
}

/// Height of the arrow under or over a bubble, and how far a bubble keeps from the window edge.
const ARROW: i32 = 9;
const EDGE: i32 = 10;

/// Shows the tips of `page` once. `tips` makes them when they show, so they point at what is on
/// screen then.
pub fn on_first_visit(
    st: &St,
    page: &'static str,
    widget: &impl IsA<gtk::Widget>,
    tips: impl Fn() -> Vec<Tip> + 'static,
) {
    let tips: Rc<dyn Fn() -> Vec<Tip>> = Rc::new(tips);
    PAGES.with(|p| p.borrow_mut().push((page, widget.as_ref().downgrade(), tips.clone())));
    let waiting = Rc::new(Cell::new(false));
    let try_now: Rc<dyn Fn(&gtk::Widget)> = Rc::new({
        let (st, waiting) = (st.clone(), waiting.clone());
        move |w: &gtk::Widget| {
            let seen = st.config.borrow().tours.iter().any(|p| p == page);
            if seen || !st.config.borrow().onboarded || !w.is_mapped() || waiting.replace(true) {
                return;
            }
            // After the page settled in, and only while no dialog is open.
            let (st, tips, waiting, w) = (st.clone(), tips.clone(), waiting.clone(), w.downgrade());
            glib::timeout_add_local_once(Duration::from_millis(700), move || {
                waiting.set(false);
                let Some(w) = w.upgrade() else { return };
                let window = w.root().and_downcast::<adw::ApplicationWindow>();
                if !w.is_mapped() || window.is_some_and(|win| win.visible_dialog().is_some()) {
                    return;
                }
                if st.config.borrow().tours.iter().any(|p| p == page) {
                    return;
                }
                let Some(layer) = layer_of(&w) else { return };
                st.update(|c| c.tours.push(page.to_string()));
                show(&layer, Rc::new(tips()), 0);
            });
        }
    });
    let w = widget.as_ref();
    w.connect_map({
        let try_now = try_now.clone();
        move |w| try_now(w)
    });
    // The welcome, or another dialog, closes: then it is time.
    w.connect_realize({
        let try_now = try_now.clone();
        move |w| {
            let Some(window) = w.root().and_downcast::<adw::ApplicationWindow>() else { return };
            let weak = w.downgrade();
            let try_now = try_now.clone();
            window.connect_visible_dialog_notify(move |win| {
                if win.visible_dialog().is_none()
                    && let Some(w) = weak.upgrade()
                {
                    try_now(&w);
                }
            });
        }
    });
    // Settings asks to show them again.
    st.subscribe({
        let (try_now, w) = (try_now.clone(), w.downgrade());
        move |c| {
            if matches!(c, crate::state::Change::Settings)
                && let Some(w) = w.upgrade()
            {
                try_now(&w);
            }
        }
    });
}

/// The widget called `name` in the window of `w`, for a tip about something on another part of
/// the window, like the bottom bar.
pub fn named(w: &impl IsA<gtk::Widget>, name: &str) -> Option<gtk::Widget> {
    fn find(w: gtk::Widget, name: &str) -> Option<gtk::Widget> {
        if w.widget_name() == name {
            return Some(w);
        }
        std::iter::successors(w.first_child(), |c| c.next_sibling()).find_map(|c| find(c, name))
    }
    find(w.as_ref().root()?.upcast(), name)
}

fn layer_of(w: &gtk::Widget) -> Option<gtk::Overlay> {
    std::iter::successors(w.parent(), |p| p.parent()).find(|p| p.widget_name() == LAYER).and_downcast()
}

/// Shows tip `i` on the layer, and the next one after it.
fn show(layer: &gtk::Overlay, tips: Rc<Vec<Tip>>, i: usize) {
    let Some(tip) = tips.get(i) else { return };
    if !tip.at.is_mapped() {
        show(layer, tips.clone(), i + 1);
        return;
    }
    let last = i + 1 == tips.len();
    let title = label(tip.title, &["heading"]);
    title.set_xalign(0.0);
    let text = label(tip.text, &[]);
    text.set_xalign(0.0);
    text.set_wrap(true);
    text.set_max_width_chars(34);
    let count =
        label(&fill(t("{} of {}"), &[&(i + 1).to_string(), &tips.len().to_string()]), &["dim-label", "caption"]);
    count.set_hexpand(true);
    count.set_xalign(0.0);
    let skip = gtk::Button::with_label(t("Skip"));
    skip.add_css_class("flat");
    skip.set_visible(!last);
    let next = gtk::Button::with_label(if last { t("Got it") } else { t("Next") });
    next.add_css_class("vn-gradient-button");
    next.add_css_class("vn-tip-next");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_margin_top(6);
    row.append(&count);
    row.append(&skip);
    row.append(&next);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("vn-tip");
    card.append(&title);
    card.append(&text);
    card.append(&row);

    // The arrow is on the side of the bubble that faces the thing.
    let (up, down) = (arrow(true), arrow(false));
    let bubble = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bubble.add_css_class("vn-tip-bubble");
    bubble.set_halign(gtk::Align::Start);
    bubble.set_valign(gtk::Align::Start);
    bubble.append(&up);
    bubble.append(&card);
    bubble.append(&down);
    layer.add_overlay(&bubble);
    // A mark that fills the area the tip is about.
    let ring = gtk::DrawingArea::builder()
        .can_target(false)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .visible(false)
        .css_classes(["vn-tip-spot"])
        .build();
    ring.set_draw_func(|_, cr, w, h| draw_mark(cr, w as f64, h as f64));
    layer.add_overlay(&ring);

    let gone = Rc::new(Cell::new(false));
    let end = Rc::new({
        let (layer, bubble, ring, gone) = (layer.downgrade(), bubble.clone(), ring.clone(), gone.clone());
        move || {
            if gone.replace(true) {
                return;
            }
            if let Some(layer) = layer.upgrade() {
                layer.remove_overlay(&bubble);
                layer.remove_overlay(&ring);
            }
        }
    });
    let (at, area) = (tip.at.downgrade(), tip.area.clone());
    let place = {
        let (layer, card, up, down, ring) =
            (layer.downgrade(), card.downgrade(), up.downgrade(), down.downgrade(), ring.downgrade());
        move |bubble: &gtk::Box| -> bool {
            let (Some(layer), Some(at), Some(card), Some(up), Some(down), Some(ring)) =
                (layer.upgrade(), at.upgrade(), card.upgrade(), up.upgrade(), down.upgrade(), ring.upgrade())
            else {
                return false;
            };
            if !at.is_mapped() {
                return false;
            }
            let (lw, lh) = (layer.width(), layer.height());
            let a = area.as_ref().and_then(|f| f());
            let spot = match a {
                Some(a) => at
                    .compute_point(&layer, &gtk::graphene::Point::new(a.x(), a.y()))
                    .map(|p| gtk::graphene::Rect::new(p.x(), p.y(), a.width(), a.height())),
                // The part of it in sight: all of a long list would be mostly off screen.
                None => at
                    .compute_bounds(&layer)
                    .map(|r| r.intersection(&gtk::graphene::Rect::new(0.0, 0.0, lw as f32, lh as f32)).unwrap_or(r)),
            };
            let Some(r) = spot else { return false };
            let (_, tw, _, _) = card.measure(gtk::Orientation::Horizontal, -1);
            let (_, th, _, _) = card.measure(gtk::Orientation::Vertical, tw);
            let (cx, top, bottom) = ((r.x() + r.width() / 2.0) as i32, r.y() as i32, (r.y() + r.height()) as i32);
            // Under the thing when there is room, else over it, and always in the window.
            let below = bottom + ARROW + th <= lh - EDGE || top - ARROW - th < EDGE;
            let y = if below { bottom + 4 } else { top - 4 - ARROW - th };
            let y = y.clamp(EDGE, (lh - EDGE - ARROW - th).max(EDGE));
            let x = (cx - tw / 2).clamp(EDGE, (lw - EDGE - tw).max(EDGE));
            up.set_visible(below);
            down.set_visible(!below);
            let ax = (cx - x - ARROW).clamp(16, (tw - 16 - 2 * ARROW).max(16));
            for a in [&up, &down] {
                if a.margin_start() != ax {
                    a.set_margin_start(ax);
                }
            }
            if bubble.margin_start() != x || bubble.margin_top() != y {
                bubble.set_margin_start(x);
                bubble.set_margin_top(y);
            }
            if ring.is_visible() != a.is_some() {
                ring.set_visible(a.is_some());
            }
            if a.is_some() {
                let (rx, ry, rw, rh) = (r.x() as i32, r.y() as i32, r.width() as i32, r.height() as i32);
                if ring.margin_start() != rx || ring.margin_top() != ry {
                    ring.set_margin_start(rx.max(0));
                    ring.set_margin_top(ry.max(0));
                }
                if ring.content_width() != rw || ring.content_height() != rh {
                    ring.set_content_width(rw.max(1));
                    ring.set_content_height(rh.max(1));
                }
            }
            true
        }
    };
    place(&bubble);
    bubble.add_tick_callback({
        let end = end.clone();
        move |b, _| {
            if place(b) {
                return glib::ControlFlow::Continue;
            }
            // What it points at went away, like the page: the tips are done.
            end();
            glib::ControlFlow::Break
        }
    });
    skip.connect_clicked({
        let end = end.clone();
        move |_| end()
    });
    next.connect_clicked({
        let (layer, bubble) = (layer.downgrade(), bubble.downgrade());
        move |_| {
            let Some(layer) = layer.upgrade() else { return };
            // The confetti bursts from the bubble.
            let from = bubble
                .upgrade()
                .and_then(|b| b.compute_bounds(&layer))
                .map(|r| ((r.x() + r.width() / 2.0) as f64, (r.y() + r.height() / 2.0) as f64));
            end();
            if last {
                confetti(&layer, from.unwrap_or((layer.width() as f64 / 2.0, layer.height() as f64 / 2.0)));
            } else {
                let tips = tips.clone();
                glib::idle_add_local_once(move || show(&layer, tips, i + 1));
            }
        }
    });
}

/// The arrow of a bubble, in its colors, pointing up or down.
fn arrow(up: bool) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::builder()
        .content_width(2 * ARROW)
        .content_height(ARROW)
        .halign(gtk::Align::Start)
        .can_target(false)
        .build();
    // It overlaps the rim of the bubble by a pixel, so the two look like one.
    if up {
        a.set_margin_bottom(-1);
    } else {
        a.set_margin_top(-1);
    }
    a.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let dark = adw::StyleManager::default().is_dark();
        let (tip, base) = if up { (0.5, h + 1.0) } else { (h - 0.5, -1.0) };
        cr.move_to(0.0, base);
        cr.line_to(w / 2.0, tip);
        cr.line_to(w, base);
        cr.close_path();
        if dark {
            cr.set_source_rgba(22.0 / 255.0, 26.0 / 255.0, 35.0 / 255.0, 0.97);
        } else {
            cr.set_source_rgb(1.0, 1.0, 1.0);
        }
        let _ = cr.fill();
        // The rim along the two slanted sides only.
        cr.move_to(0.0, base);
        cr.line_to(w / 2.0, tip);
        cr.line_to(w, base);
        cr.set_line_width(1.5);
        cr.set_source_rgba(0.23, 0.51, 0.96, 0.6);
        let _ = cr.stroke();
    });
    a
}

/// The mark of an area: a glowing strip in a narrow gap, a dashed field in a wider one, in the
/// blue of the app.
fn draw_mark(cr: &cairo::Context, w: f64, h: f64) {
    let narrow = w < 40.0;
    let (inset, r) = if narrow { ((w * 0.2).min(4.0), (w / 2.0).min(8.0)) } else { (6.0, 18.0) };
    let (x, y, w, h) = (inset, inset, (w - 2.0 * inset).max(1.0), (h - 2.0 * inset).max(1.0));
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
    cr.set_source_rgba(0.13, 0.83, 0.93, if narrow { 0.55 } else { 0.12 });
    let _ = cr.fill_preserve();
    cr.set_line_width(2.0);
    if !narrow {
        cr.set_dash(&[8.0, 6.0], 0.0);
    }
    cr.set_source_rgba(0.13, 0.83, 0.93, 0.95);
    let _ = cr.stroke();
}

/// A burst of confetti from `from`, for the end of the tips. It falls out of the window and goes.
fn confetti(layer: &gtk::Overlay, from: (f64, f64)) {
    if !layer.settings().is_gtk_enable_animations() {
        return;
    }
    struct Bit {
        x: f64,
        y: f64,
        vx: f64,
        vy: f64,
        turn: f64,
        spin: f64,
        size: (f64, f64),
        color: (f64, f64, f64),
    }
    const COLORS: [(f64, f64, f64); 6] = [
        (0.23, 0.51, 0.96),
        (0.13, 0.83, 0.93),
        (0.96, 0.45, 0.71),
        (0.98, 0.8, 0.08),
        (0.2, 0.83, 0.6),
        (0.65, 0.55, 0.98),
    ];
    const LIFE: f64 = 2.6;
    let r = glib::random_double_range;
    let bits: Rc<RefCell<Vec<Bit>>> = Rc::new(RefCell::new(
        (0..160)
            .map(|i| {
                // Up and out, in a wide fan.
                let angle = -PI / 2.0 + r(-1.2, 1.2);
                let speed = r(380.0, 980.0);
                Bit {
                    x: from.0 + r(-30.0, 30.0),
                    y: from.1 + r(-10.0, 10.0),
                    vx: angle.cos() * speed,
                    vy: angle.sin() * speed,
                    turn: r(0.0, 2.0 * PI),
                    spin: r(-9.0, 9.0),
                    size: (r(6.0, 11.0), r(3.0, 6.0)),
                    color: COLORS[i % COLORS.len()],
                }
            })
            .collect(),
    ));
    let age = Rc::new(Cell::new(0.0f64));
    let area = gtk::DrawingArea::builder().can_target(false).hexpand(true).vexpand(true).build();
    area.set_draw_func({
        let (bits, age) = (bits.clone(), age.clone());
        move |_, cr, _, _| {
            // Fades out in the last half second.
            let alpha = ((LIFE - age.get()) / 0.5).clamp(0.0, 1.0);
            for b in bits.borrow().iter() {
                cr.save().ok();
                cr.translate(b.x, b.y);
                cr.rotate(b.turn);
                // A flat bit that turns over shows thinner.
                cr.scale(1.0, (b.turn * 1.7).cos().abs().max(0.25));
                cr.rectangle(-b.size.0 / 2.0, -b.size.1 / 2.0, b.size.0, b.size.1);
                cr.set_source_rgba(b.color.0, b.color.1, b.color.2, alpha);
                let _ = cr.fill();
                cr.restore().ok();
            }
        }
    });
    layer.add_overlay(&area);
    let last = Cell::new(0i64);
    let layer = layer.downgrade();
    area.add_tick_callback(move |a, clock| {
        let now = clock.frame_time();
        let dt = match last.replace(now) {
            0 => 1.0 / 60.0,
            t => ((now - t) as f64 / 1e6).min(0.05),
        };
        age.set(age.get() + dt);
        for b in bits.borrow_mut().iter_mut() {
            b.vy += 1500.0 * dt;
            b.vx *= 1.0 - 1.2 * dt;
            b.x += b.vx * dt;
            b.y += b.vy * dt;
            b.turn += b.spin * dt;
        }
        if age.get() >= LIFE {
            if let Some(layer) = layer.upgrade() {
                layer.remove_overlay(a);
            }
            return glib::ControlFlow::Break;
        }
        a.queue_draw();
        glib::ControlFlow::Continue
    });
}
