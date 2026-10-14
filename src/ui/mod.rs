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

use gtk::prelude::*;
use gtk::{gdk, glib};

pub const RESOURCE: &str = "/io/github/voicenux/Voicenux";

/// An icon from `data/icons` (Phosphor, duotone). It takes the CSS color of the widget.
pub fn icon(name: &str, size: i32) -> gtk::Image {
    let img = gtk::Image::from_icon_name(&icon_name(name));
    img.set_pixel_size(size);
    img
}

/// An icon on a rounded square tinted in the color of the category around it.
pub fn tile(name: &str, size: i32, icon_size: i32) -> (gtk::Box, gtk::Image) {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("vn-tile");
    b.set_size_request(size, size);
    b.set_halign(gtk::Align::Center);
    b.set_valign(gtk::Align::Center);
    let img = icon(name, icon_size);
    img.set_vexpand(true);
    img.set_halign(gtk::Align::Center);
    img.set_valign(gtk::Align::Center);
    b.append(&img);
    // The icon fills the tile, but the tile must not make its parents stretch.
    b.set_vexpand(false);
    b.set_hexpand(false);
    (b, img)
}

/// Icon names of `data/icons`. Unknown names (an old saved voice) get the sparkles.
pub fn icon_name(name: &str) -> String {
    let path = format!("{RESOURCE}/icons/scalable/actions/vn-{name}-symbolic.svg");
    if gtk::gio::resources_get_info(&path, gtk::gio::ResourceLookupFlags::NONE).is_ok() {
        format!("vn-{name}-symbolic")
    } else {
        "vn-sparkles-symbolic".to_string()
    }
}

/// All icons there are, for the icon picker.
pub fn all_icons() -> Vec<String> {
    let mut names: Vec<String> = gtk::gio::resources_enumerate_children(
        &format!("{RESOURCE}/icons/scalable/actions/"),
        gtk::gio::ResourceLookupFlags::NONE,
    )
    .map(|l| l.iter().map(|s| s.trim_start_matches("vn-").trim_end_matches("-symbolic.svg").to_string()).collect())
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

/// Plays the entrance animation on a widget, `delay` steps of 16 ms late.
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
/// CSS made from code: entrance delays and the gradient of every category.
pub fn generated_css() -> String {
    let mut css: String = (0..40).map(|i| format!(".vn-d{i} {{ animation-delay: {}ms; }}\n", i * 16)).collect();
    for c in crate::voice::CATEGORIES {
        let (a, b) = c.colors;
        css.push_str(&format!(".cat-{} {{ --cat-a: {a}; --cat-b: {b}; }}\n", c.id));
    }
    css
}

/// The gradient colors of a category, for drawing.
pub fn cat_colors(category: &str) -> ((f64, f64, f64), (f64, f64, f64)) {
    let (a, b) = crate::voice::category(category).colors;
    (rgb(a), rgb(b))
}

/// "#ff8a5c" to (r, g, b) in 0..1.
pub fn rgb(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("80"), 16).unwrap_or(128) as f64 / 255.0;
    (p(0), p(2), p(4))
}

/// A button showing an icon. Clicking it opens a grid of all icons to choose from.
pub fn icon_picker(current: &str, size: i32, on_pick: impl Fn(&str) + 'static) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    let shown = move |name: &str| {
        let (b, _) = tile(name, size + size / 2, size);
        b.add_css_class("cat-custom");
        b
    };
    button.set_child(Some(&shown(current)));
    button.set_tooltip_text(Some(crate::i18n::t("Change icon")));
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
            button.set_child(Some(&shown(&name)));
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
