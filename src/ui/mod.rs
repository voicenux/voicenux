//! The window and everything in it.

pub mod bottombar;
mod emoji;
mod find_sounds;
pub mod keys;
pub mod onboarding;
pub mod popup;
mod scroll;
mod trim;
pub mod settings;
pub mod soundboard;
pub mod visuals;
pub mod voicelab;
pub mod voices;
pub mod window;

use std::cell::RefCell;
use std::collections::HashMap;

use adw::prelude::*;
use gtk::{gdk, glib};

pub use emoji::icon_picker;
pub use scroll::smooth_scroll;

pub const RESOURCE: &str = "/io/github/voicenux/Voicenux";

thread_local! {
    static TEXTURES: RefCell<HashMap<String, gdk::Texture>> = RefCell::new(HashMap::new());
}

/// A picture from `data/icons`, cached.
pub fn texture(name: &str) -> gdk::Texture {
    TEXTURES.with(|t| {
        if let Some(tex) = t.borrow().get(name) {
            return tex.clone();
        }
        let tex = load_picture(name);
        t.borrow_mut().insert(name.to_string(), tex.clone());
        tex
    })
}

/// A picture from `data/icons` or `data/emoji`, not cached. Sparkles when there is none.
pub fn load_picture(name: &str) -> gdk::Texture {
    for dir in ["icons", "emoji"] {
        let path = format!("{RESOURCE}/{dir}/{name}.png");
        if gtk::gio::resources_get_info(&path, gtk::gio::ResourceLookupFlags::NONE).is_ok() {
            return gdk::Texture::from_resource(&path);
        }
    }
    gdk::Texture::from_resource(&format!("{RESOURCE}/icons/sparkles.png"))
}

pub fn icon(name: &str, size: i32) -> gtk::Image {
    let img = gtk::Image::from_paintable(Some(&texture(name)));
    img.set_pixel_size(size);
    img
}

pub fn logo(size: i32) -> gtk::Image {
    let tex = gdk::Texture::from_resource(&format!("{RESOURCE}/logo.svg"));
    let img = gtk::Image::from_paintable(Some(&tex));
    img.set_pixel_size(size);
    img
}

pub fn cat_class(category: &str) -> String {
    format!("cat-{category}")
}

/// "#ff8a5c" to (r, g, b) in 0..1.
pub fn rgb(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("80"), 16).unwrap_or(128) as f64 / 255.0;
    (p(0), p(2), p(4))
}

/// Peak level to a meter position, 0 at -60 dBFS and 1 at 0 dBFS.
pub fn meter(peak: f32) -> f32 {
    ((20.0 * peak.max(1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    for c in classes {
        l.add_css_class(c);
    }
    l
}

/// Removes every child of a box-like widget.
pub fn clear(widget: &impl IsA<gtk::Widget>) {
    let w = widget.as_ref();
    while let Some(child) = w.first_child() {
        if let Some(b) = w.downcast_ref::<gtk::Box>() {
            b.remove(&child);
        } else if let Some(f) = w.downcast_ref::<gtk::FlowBox>() {
            f.remove(&child);
        } else if let Some(l) = w.downcast_ref::<gtk::ListBox>() {
            l.remove(&child);
        } else if let Some(wb) = w.downcast_ref::<adw::WrapBox>() {
            wb.remove(&child);
        } else {
            child.unparent();
        }
    }
}

/// Plays the entrance animation on a widget, `delay` steps of 18 ms late.
pub fn animate_in(widget: &impl IsA<gtk::Widget>, delay: usize) {
    let w = widget.as_ref();
    for i in 0..40 {
        w.remove_css_class(&format!("vn-d{i}"));
    }
    w.add_css_class(&format!("vn-d{}", delay.min(39)));
    // A new animation name starts the animation again, the same name would not.
    let (old, new) =
        if w.has_css_class("vn-entering-a") { ("vn-entering-a", "vn-entering-b") } else { ("vn-entering-b", "vn-entering-a") };
    w.remove_css_class(old);
    w.add_css_class(new);
}

/// A value that looks like a label. A click opens it for typing: `start` gives the text to begin
/// with, `done` gets the field and what was typed, and must set the shown text again. Enter or a
/// click anywhere else puts the value in, Escape keeps the old one.
pub fn value_field(
    start: impl Fn() -> String + 'static,
    done: impl Fn(&gtk::EditableLabel, &str) + 'static,
) -> gtk::EditableLabel {
    let field = gtk::EditableLabel::new("");
    field.add_css_class("vn-value-field");
    // Narrow, so the field hugs the shown text. Longer typing scrolls.
    field.set_width_chars(3);
    field.set_max_width_chars(4);
    field.set_alignment(0.5);
    field.set_tooltip_text(Some(crate::i18n::t("Click to type a value")));
    // While typing: a click watcher on the window or popover the field is in.
    let outside: std::rc::Rc<RefCell<Option<(gtk::Widget, gtk::GestureClick)>>> = Default::default();
    field.connect_editing_notify(move |f| {
        if f.is_editing() {
            f.set_text(&start());
            f.select_region(0, -1);
            if let Some(native) = f.native() {
                let native: gtk::Widget = native.upcast();
                *outside.borrow_mut() = Some((native.clone(), click_outside(f, &native)));
            }
        } else {
            if let Some((native, click)) = outside.take() {
                native.remove_controller(&click);
            }
            done(f, &f.text());
        }
    });
    field
}

/// Stops typing in `field` when a click lands anywhere else in `native`. The click itself goes on
/// as usual.
fn click_outside(field: &gtk::EditableLabel, native: &gtk::Widget) -> gtk::GestureClick {
    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let field = field.downgrade();
    click.connect_pressed(move |g, _, x, y| {
        let (Some(f), Some(w)) = (field.upgrade(), g.widget()) else { return };
        let hit = w.pick(x, y, gtk::PickFlags::DEFAULT);
        if hit.is_some_and(|h| h == *f.upcast_ref::<gtk::Widget>() || h.is_ancestor(&f)) {
            return;
        }
        // Later, not while this controller is still busy with the click.
        glib::idle_add_local_once(move || {
            if f.is_editing() {
                f.stop_editing(true);
            }
        });
    });
    native.add_controller(click.clone());
    click
}

/// CSS for the staggered entrance delays.
pub fn delay_css() -> String {
    (0..40).map(|i| format!(".vn-d{i} {{ animation-delay: {}ms; }}\n", i * 18)).collect()
}

/// Renders a widget (the window) into a PNG file.
pub fn snapshot(widget: &gtk::Widget, path: &str) -> anyhow::Result<()> {
    let (w, h) = (widget.width(), widget.height());
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snap = gtk::Snapshot::new();
    paintable.snapshot(&snap, w as f64, h as f64);
    let node = snap.to_node().ok_or_else(|| anyhow::anyhow!("nothing to render"))?;
    let renderer = widget.native().and_then(|n| n.renderer()).ok_or_else(|| anyhow::anyhow!("no renderer"))?;
    let bounds = gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
    renderer.render_texture(&node, Some(&bounds)).save_to_png(path)?;
    Ok(())
}
