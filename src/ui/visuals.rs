//! Animated drawings: the voice orb, level meters and the little live bars.

use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{cairo, glib};

use super::rgb;
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

/// Horizontal meter made of rounded segments, green to red.
pub fn level_meter(st: &St, read: fn(&crate::state::Levels) -> f32, segments: usize) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    let seg_w = 5.0;
    let gap = 3.0;
    area.set_content_width((segments as f64 * (seg_w + gap) - gap) as i32);
    area.set_content_height(12);
    area.set_valign(gtk::Align::Center);
    let peak_hold = Rc::new(Cell::new((0.0f32, 0.0f64)));
    let st1 = st.clone();
    let hold1 = peak_hold.clone();
    area.set_draw_func(move |a, cr, _w, h| {
        let st = &st1;
        let peak_hold = &hold1;
        let l = st.levels.get();
        let v = read(&l);
        let (mut hold, at) = peak_hold.get();
        if v >= hold || l.time - at > 1.2 {
            hold = v;
            peak_hold.set((v, l.time));
        }
        let (fr, fgc, fb) = fg(a);
        let lit = (v * segments as f32).round() as usize;
        let hold_seg = ((hold * segments as f32).round() as usize).saturating_sub(1);
        for i in 0..segments {
            let x = i as f64 * (seg_w + gap);
            let pos = i as f64 / (segments - 1) as f64;
            let (r, g, b) = if pos < 0.6 {
                (0.18, 0.76, 0.49)
            } else if pos < 0.85 {
                (0.96, 0.77, 0.18)
            } else {
                (0.9, 0.16, 0.22)
            };
            if i < lit || (i == hold_seg && hold > 0.05) {
                cr.set_source_rgba(r, g, b, 1.0);
            } else {
                cr.set_source_rgba(fr, fgc, fb, 0.12);
            }
            rounded(cr, x, 0.0, seg_w, h as f64, 2.0);
            let _ = cr.fill();
        }
    });
    let st2 = st.clone();
    let last = Cell::new(-1.0f32);
    let hold_last = peak_hold.clone();
    let last_hold = Cell::new(-1.0f32);
    redraw_when(&area, move || {
        let v = read(&st2.levels.get());
        let a = moved(&last, v);
        let b = moved(&last_hold, hold_last.get().0);
        a || b
    });
    area
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// The big animated circle behind the selected voice. It breathes with your voice.
pub struct Orb {
    pub widget: gtk::Overlay,
    image: gtk::Image,
    color: Rc<Cell<(f64, f64, f64)>>,
    pop: Rc<Cell<f64>>,
    anim: adw::SpringAnimation,
}

impl Orb {
    pub fn new(st: &St, size: i32) -> Self {
        let area = gtk::DrawingArea::new();
        area.set_content_width(size);
        area.set_content_height(size);
        let color = Rc::new(Cell::new((0.48, 0.36, 1.0)));
        let pop = Rc::new(Cell::new(1.0f64));
        let image = gtk::Image::new();
        image.set_pixel_size(size * 46 / 100);
        image.set_halign(gtk::Align::Center);
        image.set_valign(gtk::Align::Center);
        image.set_can_target(false);

        let st2 = st.clone();
        let c2 = color.clone();
        let p2 = pop.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let l = st2.levels.get();
            let on = st2.config.borrow().voice_on && !st2.muted.get();
            let level = if on { l.output as f64 } else { 0.0 };
            let t = l.time;
            let (r, g, b) = if on { c2.get() } else { (0.55, 0.55, 0.6) };
            let cx = w as f64 / 2.0;
            let cy = h as f64 / 2.0;
            let scale = p2.get();
            let half = w.min(h) as f64 / 2.0;
            let base = half * 0.56 * scale;

            // Glow, fading out before the edge of the drawing.
            let glow = cairo::RadialGradient::new(cx, cy, base * 0.6, cx, cy, half);
            glow.add_color_stop_rgba(0.0, r, g, b, 0.30 + 0.35 * level);
            glow.add_color_stop_rgba(0.55 + 0.25 * level, r, g, b, 0.08 + 0.1 * level);
            glow.add_color_stop_rgba(1.0, r, g, b, 0.0);
            let _ = cr.set_source(&glow);
            cr.arc(cx, cy, half, 0.0, 2.0 * PI);
            let _ = cr.fill();

            // Wobbly rings that grow with the voice.
            for i in 0..3 {
                let fi = i as f64;
                let radius = (base * (1.12 + fi * 0.16) + level * base * (0.18 + fi * 0.08)).min(half * 0.92);
                let amp = base * (0.015 + 0.06 * level) * (1.0 + fi * 0.4);
                let speed = 0.9 + fi * 0.45;
                let lobes = 3.0 + fi * 2.0;
                cr.new_path();
                let steps = 96;
                for s in 0..=steps {
                    let a = s as f64 / steps as f64 * 2.0 * PI;
                    let rr = radius
                        + amp
                            * ((a * lobes + t * speed * if i % 2 == 0 { 1.0 } else { -1.0 }).sin()
                                + 0.5 * (a * (lobes + 2.0) - t * speed * 1.7).sin());
                    let (x, y) = (cx + rr * a.cos(), cy + rr * a.sin());
                    if s == 0 {
                        cr.move_to(x, y);
                    } else {
                        cr.line_to(x, y);
                    }
                }
                cr.close_path();
                cr.set_source_rgba(r, g, b, 0.42 - fi * 0.11);
                cr.set_line_width(2.0 - fi * 0.4);
                let _ = cr.stroke();
            }

            // A gradient arc that circles around.
            let arc_r = base * 1.04;
            let start = t * 0.9;
            let lin = cairo::LinearGradient::new(cx - arc_r, cy - arc_r, cx + arc_r, cy + arc_r);
            lin.add_color_stop_rgba(0.0, 0.48, 0.36, 1.0, 0.0);
            lin.add_color_stop_rgba(0.5, 0.75, 0.30, 1.0, 0.9);
            lin.add_color_stop_rgba(1.0, 1.0, 0.31, 0.64, 0.9);
            let _ = cr.set_source(&lin);
            cr.set_line_width(3.5);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.new_path();
            cr.arc(cx, cy, arc_r, start, start + PI * (0.7 + 0.5 * level));
            let _ = cr.stroke();

            // Disc behind the picture.
            let disc = cairo::RadialGradient::new(cx, cy - base * 0.3, base * 0.1, cx, cy, base);
            disc.add_color_stop_rgba(0.0, r, g, b, 0.55);
            disc.add_color_stop_rgba(1.0, r * 0.55, g * 0.55, b * 0.55, 0.35);
            let _ = cr.set_source(&disc);
            cr.arc(cx, cy, base * (0.98 + 0.04 * level), 0.0, 2.0 * PI);
            let _ = cr.fill();
        });
        // Full speed while there is sound or the pop runs, a calm 15 fps drift otherwise.
        let st3 = st.clone();
        let p3 = pop.clone();
        let frame = Cell::new(0u32);
        redraw_when(&area, move || {
            frame.set(frame.get().wrapping_add(1));
            let busy = st3.levels.get().output > 0.02 || (p3.get() - 1.0).abs() > 0.001;
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
        Self { widget, image, color, pop, anim }
    }

    pub fn set_voice(&self, icon: &str, color: &str, animate: bool) {
        self.image.set_paintable(Some(&super::texture(icon)));
        self.color.set(rgb(color));
        if animate {
            self.anim.set_value_from(0.55);
            self.anim.play();
        } else {
            self.pop.set(1.0);
        }
    }
}

/// Four little bars that dance with the voice, on the active voice card.
pub fn live_bars(st: &St) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(18);
    area.set_content_height(14);
    let st1 = st.clone();
    area.set_draw_func(move |_, cr, _w, h| {
        let st = &st1;
        let l = st.levels.get();
        let on = st.config.borrow().voice_on && !st.muted.get();
        let level = if on { l.output as f64 } else { 0.0 };
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
        for i in 0..4 {
            let fi = i as f64;
            let wobble = 0.55 + 0.45 * (l.time * 9.0 + fi * 1.7).sin().abs();
            let bh = (0.22 + 0.78 * level * wobble) * h as f64;
            rounded(cr, fi * 4.5, h as f64 - bh, 3.0, bh, 1.5);
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

/// A ring that fills while a sound plays.
pub fn progress_ring(progress: Rc<Cell<f32>>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(84);
    area.set_content_height(84);
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
        lin.add_color_stop_rgb(0.0, 0.48, 0.36, 1.0);
        lin.add_color_stop_rgb(1.0, 1.0, 0.31, 0.64);
        let _ = cr.set_source(&lin);
        cr.set_line_width(4.0);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.arc(cx, cy, r, -PI / 2.0, -PI / 2.0 + 2.0 * PI * p);
        let _ = cr.stroke();
    });
    area
}
