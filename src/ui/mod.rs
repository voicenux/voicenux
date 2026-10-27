//! The window and everything in it.

pub mod advanced;
pub mod ai;
pub mod board;
pub mod bottombar;
pub mod emoji;
mod find_sounds;
mod glyph;
pub mod help;
pub mod keys;
pub mod onboarding;
pub mod popup;
mod scroll;
pub mod settings;
pub mod soundboard;
pub mod tour;
pub mod trim;
pub mod visuals;
pub mod voicelab;
pub mod voices;
pub mod window;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

pub use emoji::icon_picker;
pub use scroll::smooth_scroll;

pub const RESOURCE: &str = "/io/github/voicenux/Voicenux";

thread_local! {
    static TEXTURES: RefCell<HashMap<String, gdk::Texture>> = RefCell::new(HashMap::new());
    /// The pop-in each tile is on, by tile, so an older one stops when a new one starts.
    static ENTERING: RefCell<(u64, HashMap<usize, u64>)> = RefCell::default();
    /// What sliders do, from Settings: the voice changes while you drag, the wheel moves them.
    static SLIDERS: Cell<(bool, bool)> = const { Cell::new((true, true)) };
    /// The slider under the pointer, see `wheel_slider`.
    static HOVERED: RefCell<glib::WeakRef<gtk::Scale>> = RefCell::default();
}

/// Takes what sliders do from the settings. Call when they change.
pub fn follow_settings(c: &crate::config::Config) {
    SLIDERS.set((c.live_sliders, c.wheel_sliders));
}

/// Whether the voice changes while you drag one of its sliders.
pub fn live_sliders() -> bool {
    SLIDERS.get().0
}

/// Whether the mouse wheel moves sliders.
pub fn wheel_sliders() -> bool {
    SLIDERS.get().1
}

/// The mouse wheel on a slider, when Settings allows it: a notch of the wheel moves it by `step`,
/// up for more. In a page that scrolls, `smooth_scroll` decides first whether the turn is for the
/// page or for the slider. Touchpads move it the way GTK does.
pub fn wheel_slider(scale: &gtk::Scale, step: f64) {
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    wheel.connect_scroll(move |c, dx, dy| {
        if !wheel_sliders() {
            return glib::Propagation::Stop;
        }
        let Some(s) = c.widget().and_downcast::<gtk::Scale>() else { return glib::Propagation::Proceed };
        if c.unit() != gdk::ScrollUnit::Wheel {
            return glib::Propagation::Proceed;
        }
        let notches = if dy != 0.0 { -dy } else { dx };
        s.set_value(s.value() + notches * step);
        glib::Propagation::Stop
    });
    scale.add_controller(wheel);
    // The page around it asks for this: a wheel turn on a slider is for the slider.
    let motion = gtk::EventControllerMotion::new();
    motion.connect_enter(|c, _, _| {
        if let Some(s) = c.widget().and_downcast::<gtk::Scale>() {
            HOVERED.with_borrow_mut(|h| h.set(Some(&s)));
        }
    });
    motion.connect_leave(|c| {
        let left = c.widget();
        HOVERED.with_borrow_mut(|h| {
            if h.upgrade().map(|s| s.upcast::<gtk::Widget>()) == left {
                h.set(None);
            }
        });
    });
    scale.add_controller(motion);
}

/// The slider under the pointer, of those with `wheel_slider`.
pub fn hovered_slider() -> Option<gtk::Scale> {
    HOVERED.with_borrow(|h| h.upgrade()).filter(|s| s.is_mapped())
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

/// The PNG file of a picture, like `load_picture` finds it, for the tray menu.
pub fn picture_png(name: &str) -> Vec<u8> {
    let found = [("icons", name), ("emoji", name), ("icons", "sparkles")].into_iter().find_map(|(dir, n)| {
        let path = format!("{RESOURCE}/{dir}/{n}.png");
        gtk::gio::resources_lookup_data(&path, gtk::gio::ResourceLookupFlags::NONE).ok()
    });
    found.map(|b| b.to_vec()).unwrap_or_default()
}

pub fn icon(name: &str, size: i32) -> gtk::Image {
    let img = gtk::Image::from_paintable(Some(&texture(name)));
    img.set_pixel_size(size);
    img
}

/// A Lucide icon (see `glyph`) in the CSS `color`. The sidebar, the settings and the list of keys
/// use these, voices and sounds their emoji.
pub fn line_icon(name: &str, size: i32) -> gtk::Image {
    let img = gtk::Image::from_paintable(Some(&glyph::glyph(name)));
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

/// How long `.vn-entering` takes, and how much later each next tile starts, in ms.
const ENTER_MS: i64 = 480;
const STEP_MS: i64 = 18;
/// How many tiles pop one after the other, with a class `vn-d{n}` each for the delay.
const STEPS: usize = 40;

/// Lets a tile pop in, `place` tiles after the first. Tiles out of sight just show, and from the
/// 40th on, tiles in sight pop with the 40th: a big window shows more than 40, and those must
/// not be there before the ones above them. A tile on a page out of sight pops when the page
/// shows. The first time a page shows, its first frames are slow: the pop waits until the page
/// is in and a frame came in time, so it plays in full. At the end its classes go, or another
/// animation on the tile, like the bounce of a drop, would start it again when it ends.
pub fn animate_in(widget: &impl IsA<gtk::Widget>, place: usize) {
    let w = widget.as_ref();
    let key = w.as_ptr() as usize;
    let mine = ENTERING.with(|e| {
        let (next, on) = &mut *e.borrow_mut();
        *next += 1;
        on.insert(key, *next);
        *next
    });
    stop_entering(w);
    // Drawn but not seen yet, so the slow first frame is done before it moves.
    w.add_css_class("vn-pre");
    pop_when_shown(w, key, mine, place.min(STEPS - 1));
}

/// Whether some of a widget shows in its scrolled window. One without is in sight.
fn in_sight(w: &gtk::Widget) -> bool {
    let Some(scroll) = w.ancestor(gtk::ScrolledWindow::static_type()) else { return true };
    w.compute_bounds(&scroll).is_none_or(|r| r.y() + r.height() > 0.0 && r.y() < scroll.height() as f32)
}

fn pop_when_shown(w: &gtk::Widget, key: usize, mine: u64, place: usize) {
    let current = move || ENTERING.with(|e| e.borrow().1.get(&key) == Some(&mine));
    // Waits for the map, since a clock that ticks for a page out of sight costs power.
    if !w.is_mapped() {
        let handler: std::rc::Rc<RefCell<Option<glib::SignalHandlerId>>> = Default::default();
        let h = handler.clone();
        handler.replace(Some(w.connect_map(move |w| {
            if let Some(id) = h.borrow_mut().take() {
                w.disconnect(id);
            }
            if current() {
                pop_when_shown(w, key, mine, place);
            }
        })));
        return;
    }
    let (last, start, waited) = (Cell::new(None::<i64>), Cell::new(None::<i64>), Cell::new(0));
    let end = (place as i64 * STEP_MS + ENTER_MS) * 1000;
    w.add_tick_callback(move |w, clock| {
        if !current() {
            return glib::ControlFlow::Break;
        }
        if !w.is_mapped() && start.get().is_none() {
            pop_when_shown(w, key, mine, place);
            return glib::ControlFlow::Break;
        }
        let now = clock.frame_time();
        let done = match start.get() {
            _ if !w.is_mapped() => true,
            None => {
                let in_time = last.replace(Some(now)).is_some_and(|t| now - t < 40_000);
                waited.set(waited.get() + 1);
                if (in_time && !in_transition(w)) || waited.get() > 30 {
                    if !in_sight(w) {
                        stop_entering(w);
                        ENTERING.with(|e| e.borrow_mut().1.remove(&key));
                        return glib::ControlFlow::Break;
                    }
                    w.remove_css_class("vn-pre");
                    w.add_css_class(&format!("vn-d{place}"));
                    w.add_css_class("vn-entering");
                    start.set(Some(now));
                }
                false
            }
            Some(t) => now - t >= end,
        };
        if !done {
            return glib::ControlFlow::Continue;
        }
        stop_entering(w);
        ENTERING.with(|e| e.borrow_mut().1.remove(&key));
        glib::ControlFlow::Break
    });
}

/// Whether a stack around the widget still fades to another page.
fn in_transition(w: &gtk::Widget) -> bool {
    std::iter::successors(w.parent(), |p| p.parent())
        .any(|p| p.downcast_ref::<gtk::Stack>().is_some_and(|s| s.is_transition_running()))
}

fn stop_entering(w: &gtk::Widget) {
    w.remove_css_class("vn-pre");
    w.remove_css_class("vn-entering");
    for i in 0..STEPS {
        w.remove_css_class(&format!("vn-d{i}"));
    }
}

/// A button on a tile, like the star of a voice card. A press squishes the tile, so a click near
/// the edge of the button used to end next to it and do nothing. With the pointer on the button,
/// the tile holds still.
pub fn tile_button(button: &impl IsA<gtk::Widget>, tile: &gtk::FlowBoxChild) {
    let motion = gtk::EventControllerMotion::new();
    let tile = tile.downgrade();
    motion.connect_contains_pointer_notify(move |m| {
        let Some(tile) = tile.upgrade() else { return };
        if m.contains_pointer() {
            tile.add_css_class("vn-on-button");
        } else {
            tile.remove_css_class("vn-on-button");
        }
    });
    button.add_controller(motion);
}

/// What a [`random_button`] picks from: its icon, its text and its tooltip.
pub type RandomFrom = (&'static str, &'static str, String);

/// A button that picks something at random, with an arrow to choose from what: all of them, or
/// your favorites. It shows and remembers the choice: `remember` keeps it, `pick` gets true for
/// favorites. Choosing only switches, it picks nothing yet.
pub fn random_button(
    from: [RandomFrom; 2],
    favorites: bool,
    remember: impl Fn(bool) + 'static,
    pick: impl Fn(bool) + 'static,
) -> adw::SplitButton {
    let content = adw::ButtonContent::new();
    let button = adw::SplitButton::builder().child(&content).build();
    button.set_dropdown_tooltip(crate::i18n::t("Pick from"));
    let mode = Rc::new(Cell::new(favorites));
    let popover = gtk::Popover::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    list.add_css_class("vn-menu-list");
    let mut items = Vec::new();
    for (icon, text, _) in &from {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.append(&gtk::Image::from_icon_name(icon));
        let l = gtk::Label::new(Some(text));
        l.set_xalign(0.0);
        l.set_hexpand(true);
        row.append(&l);
        let check = gtk::Image::from_icon_name("object-select-symbolic");
        row.append(&check);
        let item = gtk::Button::builder().child(&row).css_classes(["flat", "vn-menu-item"]).build();
        list.append(&item);
        items.push((item, check));
    }
    popover.set_child(Some(&list));
    popup::menu(&popover);
    button.set_popover(Some(&popover));
    let show = Rc::new({
        let (content, button, mode) = (content.clone(), button.downgrade(), mode.clone());
        let checks: Vec<gtk::Image> = items.iter().map(|(_, c)| c.clone()).collect();
        move || {
            let (icon, text, tip) = &from[mode.get() as usize];
            content.set_icon_name(icon);
            content.set_label(text);
            if let Some(b) = button.upgrade() {
                b.set_tooltip_text(Some(tip));
            }
            for (i, check) in checks.iter().enumerate() {
                check.set_opacity(if (i == 1) == mode.get() { 1.0 } else { 0.0 });
            }
        }
    });
    show();
    let remember = Rc::new(remember);
    for (i, (item, _)) in items.iter().enumerate() {
        item.connect_clicked({
            let (mode, show, remember, popover) = (mode.clone(), show.clone(), remember.clone(), popover.downgrade());
            move |_| {
                if let Some(p) = popover.upgrade() {
                    p.popdown();
                }
                mode.set(i == 1);
                remember(i == 1);
                show();
            }
        });
    }
    button.connect_clicked(move |_| pick(mode.get()));
    button
}

/// A tooltip with its key, like "Random voice (Ctrl+R)".
pub fn with_key(tip: &str, key: &str) -> String {
    format!("{tip} ({}+{key})", crate::i18n::t("Ctrl"))
}

/// Lower case and without umlauts, so "mause" finds "Mäuse" too.
pub fn fold(s: &str) -> String {
    s.to_lowercase().replace('ä', "a").replace('ö', "o").replace('ü', "u").replace('ß', "ss")
}

/// A search field with a little "Ctrl F" key cap at its end while it is empty and not focused.
/// Escape empties it, a second Escape leaves it. It goes at the start of a header bar, in line
/// with the page below. In the middle it would leave no room to drag the window.
pub fn search_field(placeholder: &str) -> (gtk::Overlay, gtk::SearchEntry) {
    // Narrow when the window is, wider when there is room.
    let entry = gtk::SearchEntry::builder().placeholder_text(placeholder).width_chars(10).max_width_chars(26).build();
    entry.add_css_class("vn-search");
    entry.add_css_class("vn-capped");
    let caps = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    caps.append(&label(crate::i18n::t("Ctrl"), &["vn-key"]));
    caps.append(&label("F", &["vn-key"]));
    caps.set_halign(gtk::Align::End);
    caps.set_valign(gtk::Align::Center);
    caps.set_margin_end(10);
    caps.set_can_target(false);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&entry));
    overlay.add_overlay(&caps);
    let focus = gtk::EventControllerFocus::new();
    let sync = {
        let (entry, caps, focus) = (entry.downgrade(), caps.clone(), focus.downgrade());
        move || {
            if let (Some(e), Some(f)) = (entry.upgrade(), focus.upgrade()) {
                let show = e.text().is_empty() && !f.contains_focus();
                caps.set_visible(show);
                // Keeps the placeholder clear of the key caps.
                if show {
                    e.add_css_class("vn-capped");
                } else {
                    e.remove_css_class("vn-capped");
                }
            }
        }
    };
    focus.connect_contains_focus_notify({
        let sync = sync.clone();
        move |_| sync()
    });
    entry.connect_changed(move |_| sync());
    entry.add_controller(focus);
    entry.connect_stop_search(|e| {
        if e.text().is_empty() {
            e.emit_move_focus(gtk::DirectionType::TabForward);
        } else {
            e.set_text("");
        }
    });
    (overlay, entry)
}

/// Notches for a slider, in its own numbers. While you drag it, the slider holds still at a notch
/// until the pointer is a few pixels past, then tears away. Keys and typed values go where they
/// go. Each notch gets a little tick under the slider.
pub fn notches(scale: &gtk::Scale, points: &[f64]) {
    /// How far from a notch it still holds, in pixels.
    const REACH: f64 = 8.0;
    let adj = scale.adjustment();
    let mut points: Vec<f64> = points.iter().copied().filter(|p| *p > adj.lower() && *p < adj.upper()).collect();
    points.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    if points.is_empty() {
        return;
    }
    for p in &points {
        scale.add_mark(*p, gtk::PositionType::Bottom, None);
    }
    let held = std::rc::Rc::new(std::cell::Cell::new(false));
    let press = gtk::EventControllerLegacy::new();
    press.set_propagation_phase(gtk::PropagationPhase::Capture);
    press.connect_event({
        let held = held.clone();
        move |_, e| {
            use gdk::EventType::*;
            match e.event_type() {
                ButtonPress | TouchBegin => held.set(true),
                ButtonRelease | TouchEnd | TouchCancel => held.set(false),
                _ => {}
            }
            glib::Propagation::Proceed
        }
    });
    scale.add_controller(press);
    scale.connect_value_changed(move |s| {
        if !held.get() {
            return;
        }
        let adj = s.adjustment();
        let per_px = (adj.upper() - adj.lower()) / s.width().max(1) as f64;
        let v = s.value();
        if let Some(p) = points.iter().find(|p| (v - **p).abs() <= REACH * per_px)
            && v != *p
        {
            // Calls the other handlers again, with the notch.
            s.set_value(*p);
        }
    });
}

/// A number someone typed: "1,5", "+3 dB", "80 %". `units` are cut off the end first.
pub fn typed_number(text: &str, units: &[&str]) -> Option<f64> {
    let mut t = text.trim().replace(',', ".").to_lowercase();
    for u in units {
        if let Some(rest) = t.strip_suffix(u) {
            t = rest.trim().to_string();
        }
    }
    t.trim_start_matches('+').parse::<f64>().ok().filter(|v| v.is_finite())
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
    (0..STEPS).map(|i| format!(".vn-d{i} {{ animation-delay: {}ms; }}\n", i as i64 * STEP_MS)).collect()
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

#[cfg(test)]
mod tests {
    use super::typed_number;

    #[test]
    fn typed_numbers() {
        assert_eq!(typed_number("1,5", &[]), Some(1.5));
        assert_eq!(typed_number("+3 dB", &["db"]), Some(3.0));
        assert_eq!(typed_number(" 80 % ", &["%"]), Some(80.0));
        assert_eq!(typed_number("-12db", &["db"]), Some(-12.0));
        assert_eq!(typed_number("loud", &["db"]), None);
        assert_eq!(typed_number("inf", &[]), None);
    }
}
