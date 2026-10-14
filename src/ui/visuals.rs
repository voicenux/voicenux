//! Live drawings: level meter, the voice orb, progress rings and the little live bars.
//! They take their color from CSS and only redraw while something moves.

use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{cairo, glib};

use crate::state::St;

fn fg(widget: &impl IsA<gtk::Widget>) -> (f64, f64, f64) {
    let c = widget.as_ref().color();
    (c.red() as f64, c.green() as f64, c.blue() as f64)
}

/// Redraws on every frame where `changed` says something moved. Idle drawings cost nothing.
fn redraw_when(area: &gtk::DrawingArea, changed: impl Fn() -> bool + 'static) {
    area.add_tick_callback(move |w, _| {
        if w.is_mapped() && changed() {
            w.queue_draw();
        }
        glib::ControlFlow::Continue
    });
}

/// True when `v` moved away from what was drawn last.
fn moved(last: &Cell<f32>, v: f32) -> bool {
    if (last.get() - v).abs() > 0.004 {
        last.set(v);
        true
    } else {
        false
    }
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// Horizontal meter made of segments: green, then yellow, then red near clipping.
pub fn level_meter(st: &St, read: fn(&crate::state::Levels) -> f32, segments: usize) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    let seg_w = 4.0;
    let gap = 3.0;
    area.set_content_width((segments as f64 * (seg_w + gap) - gap) as i32);
    area.set_content_height(10);
    area.set_valign(gtk::Align::Center);
    let st1 = st.clone();
    area.set_draw_func(move |a, cr, _w, h| {
        let v = read(&st1.levels.get());
        let (fr, fgc, fb) = fg(a);
        let lit = (v * segments as f32).round() as usize;
        for i in 0..segments {
            let x = i as f64 * (seg_w + gap);
            let pos = i as f64 / (segments - 1) as f64;
            if i < lit {
                let (r, g, b) = if pos < 0.65 {
                    (0.18, 0.76, 0.49)
                } else if pos < 0.88 {
                    (0.90, 0.65, 0.04)
                } else {
                    (0.88, 0.11, 0.14)
                };
                cr.set_source_rgb(r, g, b);
            } else {
                cr.set_source_rgba(fr, fgc, fb, 0.14);
            }
            rounded(cr, x, 0.0, seg_w, h as f64, 2.0);
            let _ = cr.fill();
        }
    });
    let st2 = st.clone();
    let last = Cell::new(-1.0f32);
    redraw_when(&area, move || moved(&last, read(&st2.levels.get())));
    area
}

type Rgb = (f64, f64, f64);

/// The big animated circle around the selected voice. It breathes with your voice.
pub struct Orb {
    pub widget: gtk::Overlay,
    image: gtk::Image,
    colors: Rc<Cell<(Rgb, Rgb)>>,
    pop: Rc<Cell<f64>>,
    anim: adw::SpringAnimation,
}

impl Orb {
    pub fn new(st: &St, size: i32) -> Self {
        let area = gtk::DrawingArea::new();
        area.set_content_width(size);
        area.set_content_height(size);
        let colors = Rc::new(Cell::new(((1.0, 0.7, 0.36), (1.0, 0.42, 0.17))));
        let pop = Rc::new(Cell::new(1.0f64));
        let image = gtk::Image::new();
        image.set_pixel_size(size * 30 / 100);
        image.add_css_class("vn-orb-icon");
        image.set_halign(gtk::Align::Center);
        image.set_valign(gtk::Align::Center);
        image.set_can_target(false);

        let st1 = st.clone();
        let c1 = colors.clone();
        let p1 = pop.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let l = st1.levels.get();
            let on = st1.config.borrow().voice_on && !st1.muted.get();
            let level = if on { l.output as f64 } else { 0.0 };
            let t = l.time;
            let ((ar, ag, ab), (br, bg, bb)) = if on { c1.get() } else { ((0.62, 0.62, 0.66), (0.42, 0.42, 0.46)) };
            let cx = w as f64 / 2.0;
            let cy = h as f64 / 2.0;
            let half = w.min(h) as f64 / 2.0;
            let base = half * 0.5 * p1.get();

            // Glow, fading out before the edge of the drawing.
            let glow = cairo::RadialGradient::new(cx, cy, base * 0.6, cx, cy, half);
            glow.add_color_stop_rgba(0.0, br, bg, bb, 0.35 + 0.35 * level);
            glow.add_color_stop_rgba(0.55 + 0.25 * level, ar, ag, ab, 0.1 + 0.12 * level);
            glow.add_color_stop_rgba(1.0, ar, ag, ab, 0.0);
            let _ = cr.set_source(&glow);
            cr.arc(cx, cy, half, 0.0, 2.0 * PI);
            let _ = cr.fill();

            // Wobbly rings that grow with the voice.
            for i in 0..3 {
                let fi = i as f64;
                let radius = (base * (1.14 + fi * 0.17) + level * base * (0.2 + fi * 0.09)).min(half * 0.93);
                let amp = base * (0.015 + 0.065 * level) * (1.0 + fi * 0.4);
                let speed = 0.9 + fi * 0.45;
                let lobes = 3.0 + fi * 2.0;
                cr.new_path();
                for s in 0..=96 {
                    let a = s as f64 / 96.0 * 2.0 * PI;
                    let dir = if i % 2 == 0 { 1.0 } else { -1.0 };
                    let rr = radius
                        + amp
                            * ((a * lobes + t * speed * dir).sin() + 0.5 * (a * (lobes + 2.0) - t * speed * 1.7).sin());
                    let (x, y) = (cx + rr * a.cos(), cy + rr * a.sin());
                    if s == 0 {
                        cr.move_to(x, y);
                    } else {
                        cr.line_to(x, y);
                    }
                }
                cr.close_path();
                let (r, g, b) = if i == 1 { (ar, ag, ab) } else { (br, bg, bb) };
                cr.set_source_rgba(r, g, b, 0.5 - fi * 0.13);
                cr.set_line_width(2.2 - fi * 0.5);
                let _ = cr.stroke();
            }

            // An arc in the category gradient that circles around.
            let arc_r = base * 1.05;
            let start = t * 0.9;
            let lin = cairo::LinearGradient::new(cx - arc_r, cy - arc_r, cx + arc_r, cy + arc_r);
            lin.add_color_stop_rgba(0.0, ar, ag, ab, 0.0);
            lin.add_color_stop_rgba(0.5, ar, ag, ab, 0.95);
            lin.add_color_stop_rgba(1.0, br, bg, bb, 0.95);
            let _ = cr.set_source(&lin);
            cr.set_line_width(4.0);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.new_path();
            cr.arc(cx, cy, arc_r, start, start + PI * (0.7 + 0.5 * level));
            let _ = cr.stroke();

            // The disc behind the icon, in the category gradient.
            let r0 = base * (0.9 + 0.05 * level);
            let disc = cairo::LinearGradient::new(cx - r0, cy - r0, cx + r0, cy + r0);
            disc.add_color_stop_rgb(0.0, ar, ag, ab);
            disc.add_color_stop_rgb(1.0, br, bg, bb);
            let _ = cr.set_source(&disc);
            cr.arc(cx, cy, r0, 0.0, 2.0 * PI);
            let _ = cr.fill();
            // A soft shine on top.
            let shine = cairo::LinearGradient::new(cx, cy - r0, cx, cy);
            shine.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.28);
            shine.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
            let _ = cr.set_source(&shine);
            cr.arc(cx, cy, r0, 0.0, 2.0 * PI);
            let _ = cr.fill();
        });
        // Full speed while there is sound or the pop runs, a calm 15 fps drift otherwise.
        let st2 = st.clone();
        let p2 = pop.clone();
        let frame = Cell::new(0u32);
        redraw_when(&area, move || {
            frame.set(frame.get().wrapping_add(1));
            let busy = st2.levels.get().output > 0.02 || (p2.get() - 1.0).abs() > 0.001;
            busy || frame.get().is_multiple_of(4)
        });

        let widget = gtk::Overlay::new();
        widget.set_child(Some(&area));
        widget.add_overlay(&image);
        widget.set_halign(gtk::Align::Center);

        let target = adw::CallbackAnimationTarget::new({
            let pop = pop.clone();
            let image = image.clone();
            move |v| {
                pop.set(v);
                image.set_opacity(v.clamp(0.0, 1.0));
            }
        });
        let anim = adw::SpringAnimation::new(&widget, 0.6, 1.0, adw::SpringParams::new(0.45, 1.0, 180.0), target);
        Self { widget, image, colors, pop, anim }
    }

    pub fn set_voice(&self, icon: &str, category: &str, animate: bool) {
        self.image.set_icon_name(Some(&super::icon_name(icon)));
        self.colors.set(super::cat_colors(category));
        if animate {
            self.anim.set_value_from(0.55);
            self.anim.play();
        } else {
            self.pop.set(1.0);
        }
    }
}

/// A ring in the brand gradient that fills while a sound plays.
pub fn progress_ring(progress: Rc<Cell<f32>>, size: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.set_can_target(false);
    area.set_draw_func(move |_, cr, w, h| {
        let p = progress.get() as f64;
        if p <= 0.0 {
            return;
        }
        let cx = w as f64 / 2.0;
        let cy = h as f64 / 2.0;
        let r = w.min(h) as f64 / 2.0 - 3.0;
        let lin = cairo::LinearGradient::new(0.0, 0.0, w as f64, h as f64);
        lin.add_color_stop_rgb(0.0, 1.0, 0.70, 0.28);
        lin.add_color_stop_rgb(1.0, 1.0, 0.29, 0.24);
        let _ = cr.set_source(&lin);
        cr.set_line_width(4.0);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.arc(cx, cy, r, -PI / 2.0, -PI / 2.0 + 2.0 * PI * p);
        let _ = cr.stroke();
    });
    area
}

/// Four little bars that move with your voice, on the card of the voice in use.
pub fn live_bars(st: &St) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.add_css_class("vn-live-bars");
    area.set_content_width(16);
    area.set_content_height(12);
    let st1 = st.clone();
    area.set_draw_func(move |a, cr, _w, h| {
        let l = st1.levels.get();
        let on = st1.config.borrow().voice_on && !st1.muted.get();
        let level = if on { l.output as f64 } else { 0.0 };
        let (r, g, b) = fg(a);
        cr.set_source_rgb(r, g, b);
        for i in 0..4 {
            let fi = i as f64;
            let wobble = 0.55 + 0.45 * (l.time * 8.0 + fi * 1.7).sin().abs();
            let bh = (0.25 + 0.75 * level * wobble) * h as f64;
            rounded(cr, fi * 4.0, h as f64 - bh, 2.5, bh, 1.25);
            let _ = cr.fill();
        }
    });
    let st2 = st.clone();
    let last = Cell::new(-1.0f32);
    redraw_when(&area, move || {
        let v = st2.levels.get().output;
        v > 0.02 || moved(&last, v)
    });
    area
}
