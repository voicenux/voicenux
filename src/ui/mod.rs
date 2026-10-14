//! The window and everything in it.

pub mod bottombar;
pub mod keys;
pub mod onboarding;
pub mod settings;
pub mod soundboard;
pub mod visuals;
pub mod voicelab;
pub mod voices;
pub mod window;

use std::cell::RefCell;
use std::collections::HashMap;

use gtk::prelude::*;
use gtk::{gdk, glib};

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
        let path = format!("{RESOURCE}/icons/{name}.png");
        let tex = if gtk::gio::resources_get_info(&path, gtk::gio::ResourceLookupFlags::NONE).is_ok() {
            gdk::Texture::from_resource(&path)
        } else {
            gdk::Texture::from_resource(&format!("{RESOURCE}/icons/sparkles.png"))
        };
        t.borrow_mut().insert(name.to_string(), tex.clone());
        tex
    })
}

pub fn icon(name: &str, size: i32) -> gtk::Image {
    let img = gtk::Image::from_paintable(Some(&texture(name)));
    img.set_pixel_size(size);
    img
}

/// All pictures there are, for the icon picker.
pub fn all_icons() -> Vec<String> {
    let mut names: Vec<String> =
        gtk::gio::resources_enumerate_children(&format!("{RESOURCE}/icons/"), gtk::gio::ResourceLookupFlags::NONE)
            .map(|l| l.iter().map(|s| s.trim_end_matches(".png").to_string()).collect())
            .unwrap_or_default();
    names.sort();
    names
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
    let w = widget.as_ref().clone();
    w.remove_css_class("vn-entering");
    for i in 0..40 {
        w.remove_css_class(&format!("vn-d{i}"));
    }
    glib::idle_add_local_once(move || {
        w.add_css_class("vn-entering");
        w.add_css_class(&format!("vn-d{}", delay.min(39)));
    });
}

/// CSS for the staggered entrance delays.
pub fn delay_css() -> String {
    (0..40).map(|i| format!(".vn-d{i} {{ animation-delay: {}ms; }}\n", i * 18)).collect()
}

/// A button showing a picture. Clicking it opens a grid of all pictures to choose from.
pub fn icon_picker(current: &str, size: i32, on_pick: impl Fn(&str) + 'static) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.set_child(Some(&icon(current, size)));
    button.set_tooltip_text(Some(crate::i18n::t("Change picture")));
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(6)
        .max_children_per_line(8)
        .homogeneous(true)
        .row_spacing(4)
        .column_spacing(4)
        .build();
    grid.add_css_class("vn-icon-grid");
    let popover = gtk::Popover::new();
    let on_pick = std::rc::Rc::new(on_pick);
    for name in all_icons() {
        let b = gtk::Button::new();
        b.add_css_class("flat");
        b.set_child(Some(&icon(&name, 40)));
        b.set_tooltip_text(Some(&name.replace('-', " ")));
        let on_pick = on_pick.clone();
        let popover = popover.clone();
        let button = button.clone();
        b.connect_clicked(move |_| {
            button.set_child(Some(&icon(&name, size)));
            popover.popdown();
            on_pick(&name);
        });
        grid.append(&b);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(320)
        .min_content_width(400)
        .child(&grid)
        .build();
    popover.set_child(Some(&scroll));
    button.set_popover(Some(&popover));
    button
}
