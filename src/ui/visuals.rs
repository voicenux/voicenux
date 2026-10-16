//! Animated drawings: the voice orb, level meters and the progress ring of a sound.
//! The orb and the meters draw with GtkSnapshot, so the GPU does the work and they stay smooth on
//! fast screens.

use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;

use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{cairo, gdk, glib, graphene, gsk};

use super::rgb;
use crate::state::St;

/// The brand gradient, from deep blue to cyan.
pub const BRAND: [(f32, f32, f32); 3] = [(0.15, 0.39, 0.92), (0.23, 0.51, 0.96), (0.13, 0.83, 0.93)];

type DrawFn = Box<dyn Fn(&gtk::Widget, &gtk::Snapshot, f32, f32)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Canvas {
        pub draw: RefCell<Option<DrawFn>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "VnCanvas";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Canvas {}

    impl WidgetImpl for Canvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let w = self.obj();
            if let Some(draw) = self.draw.borrow().as_ref() {
                draw(w.upcast_ref(), snapshot, w.width() as f32, w.height() as f32);
            }
        }
    }
}

glib::wrapper! {
    /// A widget that draws with a function, like a DrawingArea, but into a GtkSnapshot.
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Canvas {
    pub fn new(width: i32, height: i32, draw: impl Fn(&gtk::Widget, &gtk::Snapshot, f32, f32) + 'static) -> Self {
        let c: Self = glib::Object::new();
        c.set_size_request(width, height);
        *c.imp().draw.borrow_mut() = Some(Box::new(draw));
        c
    }
}

/// Redraws on every frame where `changed` says something moved. Idle drawings cost nothing.
fn redraw_when(widget: &impl IsA<gtk::Widget>, changed: impl Fn() -> bool + 'static) {
    widget.add_tick_callback(move |w, _| {
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

pub fn rgba((r, g, b): (f32, f32, f32), a: f32) -> gdk::RGBA {
    gdk::RGBA::new(r, g, b, a)
}

pub fn circle(cx: f32, cy: f32, r: f32) -> gsk::RoundedRect {
    gsk::RoundedRect::from_rect(graphene::Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r), r)
}

/// Horizontal meter made of rounded segments, green to red.
pub fn level_meter(st: &St, read: fn(&crate::state::Levels) -> f32, segments: usize) -> Canvas {
    let (seg_w, gap) = (5.0f32, 3.0f32);
    let width = segments as f32 * (seg_w + gap) - gap;
    let st1 = st.clone();
    let area = Canvas::new(width as i32, 12, move |w, s, _, h| {
        let lit = (read(&st1.levels.get()) * segments as f32).round() as usize;
        let fg = w.color();
        for i in 0..segments {
            let pos = i as f32 / (segments - 1) as f32;
            let color = if i >= lit {
                gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.12)
            } else if pos < 0.6 {
                gdk::RGBA::new(0.18, 0.76, 0.49, 1.0)
            } else if pos < 0.85 {
                gdk::RGBA::new(0.96, 0.77, 0.18, 1.0)
            } else {
                gdk::RGBA::new(0.9, 0.16, 0.22, 1.0)
            };
            let seg = gsk::RoundedRect::from_rect(graphene::Rect::new(i as f32 * (seg_w + gap), 0.0, seg_w, h), 2.0);
            s.push_rounded_clip(&seg);
            s.append_color(&color, seg.bounds());
            s.pop();
        }
    });
    area.set_valign(gtk::Align::Center);
    let st2 = st.clone();
    let last = Cell::new(-1.0f32);
    redraw_when(&area, move || moved(&last, read(&st2.levels.get())));
    area
}

/// The big animated circle behind the selected voice. It breathes with your voice.
pub struct Orb {
    pub widget: gtk::Overlay,
    image: gtk::Image,
    color: Rc<Cell<(f32, f32, f32)>>,
    pop: Rc<Cell<f64>>,
    anim: adw::SpringAnimation,
}

impl Orb {
    pub fn new(st: &St, size: i32) -> Self {
        let color = Rc::new(Cell::new(BRAND[1]));
        let pop = Rc::new(Cell::new(1.0f64));
        let image = gtk::Image::new();
        image.set_pixel_size(size * 46 / 100);
        image.set_halign(gtk::Align::Center);
        image.set_valign(gtk::Align::Center);
        image.set_can_target(false);

        let st2 = st.clone();
        let c2 = color.clone();
        let p2 = pop.clone();
        let area = Canvas::new(size, size, move |_, s, w, h| {
            let l = st2.levels.get();
            let on = st2.config.borrow().voice_on && !st2.muted.get();
            let level = if on { l.output } else { 0.0 };
            let t = l.time;
            let c = if on { c2.get() } else { (0.55, 0.55, 0.6) };
            let (cx, cy) = (w / 2.0, h / 2.0);
            let center = graphene::Point::new(cx, cy);
            let half = w.min(h) / 2.0;
            let base = half * 0.56 * p2.get() as f32;

            // Glow, fading out before the edge of the drawing.
            s.append_radial_gradient(
                &graphene::Rect::new(0.0, 0.0, w, h),
                &center,
                half,
                half,
                (base * 0.6 / half).clamp(0.0, 0.9),
                1.0,
                &[
                    gsk::ColorStop::new(0.0, rgba(c, 0.30 + 0.35 * level)),
                    gsk::ColorStop::new(0.55 + 0.25 * level, rgba(c, 0.08 + 0.1 * level)),
                    gsk::ColorStop::new(1.0, rgba(c, 0.0)),
                ],
            );

            // Rings that turn and breathe, more with the voice.
            for i in 0..3 {
                let fi = i as f32;
                let radius = (base * (1.12 + fi * 0.16) + level * base * (0.18 + fi * 0.08)).min(half * 0.9);
                let speed = 0.9 + i as f64 * 0.45;
                let squash = (0.025 + 0.07 * level) * (1.0 + fi * 0.4) * (t * speed).sin() as f32;
                let turn = (t * speed * 24.0 * if i % 2 == 0 { 1.0 } else { -1.0 }) % 360.0;
                s.save();
                s.translate(&center);
                s.rotate(turn as f32);
                s.scale(1.0 + squash, 1.0 - squash);
                let width = 2.0 - fi * 0.4;
                s.append_border(&circle(0.0, 0.0, radius), &[width; 4], &[rgba(c, 0.42 - fi * 0.11); 4]);
                s.restore();
            }

            // A gradient arc that circles around, with a round head. In the color of the voice, a
            // little lighter at the head.
            let light = (c.0 + (1.0 - c.0) * 0.35, c.1 + (1.0 - c.1) * 0.35, c.2 + (1.0 - c.2) * 0.35);
            let arc_r = base * 1.04;
            let ring = circle(cx, cy, arc_r + 1.75);
            let len = 0.35 + 0.25 * level;
            let turn = (t * 0.9).to_degrees() % 360.0;
            s.push_mask(gsk::MaskMode::Alpha);
            s.append_border(&ring, &[3.5; 4], &[gdk::RGBA::WHITE; 4]);
            s.pop();
            s.append_conic_gradient(
                ring.bounds(),
                &center,
                turn as f32,
                &[
                    gsk::ColorStop::new(0.0, rgba(c, 0.0)),
                    gsk::ColorStop::new(len * 0.5, rgba(c, 0.6)),
                    gsk::ColorStop::new(len, rgba(light, 0.95)),
                    gsk::ColorStop::new(len + 0.001, rgba(light, 0.0)),
                    gsk::ColorStop::new(1.0, rgba(light, 0.0)),
                ],
            );
            s.pop();
            let head = (turn + len as f64 * 360.0).to_radians();
            let dot = circle(cx + arc_r * head.sin() as f32, cy - arc_r * head.cos() as f32, 1.75);
            s.push_rounded_clip(&dot);
            s.append_color(&rgba(light, 0.95), dot.bounds());
            s.pop();

            // Disc behind the picture.
            let disc = circle(cx, cy, base * (0.98 + 0.04 * level));
            s.push_rounded_clip(&disc);
            s.append_radial_gradient(
                disc.bounds(),
                &graphene::Point::new(cx, cy - base * 0.3),
                base * 1.3,
                base * 1.3,
                0.0,
                1.0,
                &[
                    gsk::ColorStop::new(0.0, rgba(c, 0.55)),
                    gsk::ColorStop::new(1.0, rgba((c.0 * 0.55, c.1 * 0.55, c.2 * 0.55), 0.35)),
                ],
            );
            s.pop();
        });
        // The arc always turns. On the GPU that is cheap, so draw every frame.
        redraw_when(&area, || true);

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
        let (r, g, b) = rgb(color);
        self.color.set((r as f32, g as f32, b as f32));
        if animate {
            self.anim.set_value_from(0.55);
            self.anim.play();
        } else {
            self.pop.set(1.0);
        }
    }
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
        let ((r0, g0, b0), (r1, g1, b1)) = (BRAND[0], BRAND[2]);
        lin.add_color_stop_rgb(0.0, r0 as f64, g0 as f64, b0 as f64);
        lin.add_color_stop_rgb(1.0, r1 as f64, g1 as f64, b1 as f64);
        let _ = cr.set_source(&lin);
        cr.set_line_width(4.0);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.arc(cx, cy, r, -PI / 2.0, -PI / 2.0 + 2.0 * PI * p);
        let _ = cr.stroke();
    });
    area
}
