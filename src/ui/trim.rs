//! Cuts the start and end of a sound, like a video clip: its waveform with two handles, and a
//! playhead that says where listening starts. Listening plays in the headphones only.
//!
//! Mouse: drag a handle to cut, drag the playhead or click into the cut to move it, click
//! outside the cut to move the nearest handle there. Keys: Space listens or pauses, the arrows
//! move the playhead (Shift for bigger steps), Home and End jump to the ends of the cut, I and O
//! set the start and end at the playhead, Ctrl+S saves, Escape cancels.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{cairo, gdk, glib};

use super::{label, value_field};
use crate::i18n::{fill, t, t_dyn};
use crate::soundboard::{self, RATE, SoundData};
use crate::state::{Change, St};

/// Width of a handle.
const HANDLE: f64 = 14.0;
/// The shortest sound a cut leaves, in seconds.
const SHORTEST: f32 = 0.05;
/// How finely the waveform is kept.
const PEAKS: usize = 1600;
/// How far the pointer may be from the playhead to grab it.
const HEAD_REACH: f64 = 7.0;
const ACCENT: (f64, f64, f64) = (0.23, 0.51, 0.96);
const ACCENT_LIGHT: (f64, f64, f64) = (0.42, 0.66, 1.0);

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Start,
    End,
}

/// What is under the pointer on the waveform.
#[derive(Clone, Copy, PartialEq)]
enum Zone {
    Handle(Side),
    Head,
    /// Inside the cut: a click moves the playhead there.
    Inside,
    /// Outside the cut: a click moves the nearest handle there.
    Outside,
}

/// What a drag on the waveform moves.
#[derive(Clone, Copy, PartialEq)]
enum Grab {
    /// A handle, and how far the pointer is from its edge.
    Handle(Side, f64),
    Head,
}

struct Trim {
    st: St,
    id: String,
    data: Arc<SoundData>,
    peaks: Vec<f32>,
    /// All of the sound, in seconds.
    length: f32,
    start: Cell<f32>,
    end: Cell<f32>,
    wave: gtk::DrawingArea,
    /// The time of what is dragged or pointed at, over the waveform.
    bubble: gtk::Label,
    start_field: gtk::EditableLabel,
    end_field: gtk::EditableLabel,
    cut_length: gtk::Label,
    play: gtk::Button,
    /// When listening started, while it plays.
    playing: Cell<Option<Instant>>,
    /// The playhead, in seconds of the whole sound.
    head: Cell<f32>,
    /// Where listening started. The playhead goes back there when the sound is over.
    from: Cell<f32>,
    /// Listening went on when a drag began, so it goes on after.
    resume: Cell<bool>,
    hover: Cell<Option<Zone>>,
    /// Where the pointer is over the waveform, in pixels.
    pointer: Cell<f64>,
    dragging: Cell<Option<Grab>>,
}

/// Opens the editor for a sound over the window of `parent`.
pub fn open(st: &St, id: &str, parent: &impl IsA<gtk::Widget>) {
    let Some(sound) = st.library.borrow().find(id).cloned() else { return };
    let Some(data) = st.sound_uncut(id) else {
        st.notify(Change::Notice(fill(t("Could not play {}"), &[&t_dyn(&sound.name)])));
        return;
    };
    let length = data.frames.len() as f32 / RATE as f32;
    if length < SHORTEST * 2.0 {
        return;
    }

    let wave = gtk::DrawingArea::builder().content_height(150).hexpand(true).focusable(true).build();
    wave.add_css_class("vn-trim-wave");
    wave.update_property(&[gtk::accessible::Property::Label(t("Waveform"))]);
    let bubble = label("", &["vn-trim-bubble", "numeric"]);
    bubble.set_halign(gtk::Align::Start);
    bubble.set_valign(gtk::Align::End);
    bubble.set_margin_bottom(6);
    bubble.set_can_target(false);
    bubble.set_visible(false);
    let wave_box = gtk::Overlay::new();
    wave_box.set_child(Some(&wave));
    wave_box.add_overlay(&bubble);

    let play = gtk::Button::new();
    play.add_css_class("vn-gradient-button");

    // The fields read the times back from the editor, which only exists after them.
    let editor: Rc<std::cell::OnceCell<std::rc::Weak<Trim>>> = Rc::default();
    let field = |side: Side| {
        let (e1, e2) = (editor.clone(), editor.clone());
        value_field(
            move || e1.get().and_then(|e| e.upgrade()).map(|e| format!("{:.2}", e.time(side))).unwrap_or_default(),
            move |f, text| {
                let Some(e) = e2.get().and_then(|e| e.upgrade()) else { return };
                if let Ok(v) = text.replace(',', ".").trim_end_matches('s').trim().parse::<f32>()
                    && v.is_finite()
                {
                    e.set(side, v);
                }
                f.set_text(&seconds(e.time(side)));
            },
        )
    };
    let (start_field, end_field) = (field(Side::Start), field(Side::End));
    for f in [&start_field, &end_field] {
        f.add_css_class("numeric");
        f.set_width_chars(6);
        f.set_max_width_chars(6);
    }

    let e = Rc::new(Trim {
        st: st.clone(),
        id: id.to_string(),
        peaks: soundboard::peaks(&data, PEAKS),
        data,
        length,
        start: Cell::new(sound.start.clamp(0.0, length - SHORTEST)),
        end: Cell::new(sound.end.unwrap_or(length).clamp(SHORTEST, length)),
        wave: wave.clone(),
        bubble,
        start_field: start_field.clone(),
        end_field: end_field.clone(),
        cut_length: label("", &["numeric"]),
        play: play.clone(),
        playing: Cell::new(None),
        head: Cell::new(0.0),
        from: Cell::new(0.0),
        resume: Cell::new(false),
        hover: Cell::new(None),
        pointer: Cell::new(0.0),
        dragging: Cell::new(None),
    });
    let _ = editor.set(Rc::downgrade(&e));
    if e.end.get() <= e.start.get() {
        e.end.set(length);
    }
    e.head.set(e.start.get());

    wave.set_draw_func({
        let e = Rc::downgrade(&e);
        move |area, cr, w, h| {
            if let Some(e) = e.upgrade() {
                e.draw(area, cr, w as f64, h as f64);
            }
        }
    });
    e.pointer_events();
    e.wave_keys();
    play.connect_clicked({
        let e = Rc::downgrade(&e);
        move |_| {
            if let Some(e) = e.upgrade() {
                e.toggle_play();
            }
        }
    });

    // Start, length, end.
    let column = |title: &str, value: &gtk::Widget| {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
        b.append(&label(title, &["caption-heading", "dim-label"]));
        b.append(value);
        b
    };
    let times = gtk::CenterBox::new();
    times.set_start_widget(Some(&column(t("Start"), start_field.upcast_ref())));
    times.set_center_widget(Some(&column(t("Length"), e.cut_length.upcast_ref())));
    times.set_end_widget(Some(&column(t("End"), end_field.upcast_ref())));

    let reset = gtk::Button::with_label(t("Use all of it"));
    reset.add_css_class("pill");
    reset.connect_clicked({
        let e = Rc::downgrade(&e);
        move |_| {
            if let Some(e) = e.upgrade() {
                e.set(Side::Start, 0.0);
                e.set(Side::End, e.length);
            }
        }
    });
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::Center);
    buttons.append(&play);
    buttons.append(&reset);

    let headphones = label(t("Listening plays in your headphones only."), &["dim-label", "caption"]);
    headphones.set_wrap(true);
    headphones.set_justify(gtk::Justification::Center);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
    body.set_margin_top(12);
    body.set_margin_bottom(20);
    body.set_margin_start(20);
    body.set_margin_end(20);
    body.append(&wave_box);
    body.append(&times);
    body.append(&buttons);
    body.append(&key_hints());
    body.append(&headphones);

    let dialog = adw::Dialog::builder().content_width(640).build();
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_title_widget(Some(&adw::WindowTitle::new(t("Trim sound"), &t_dyn(&sound.name))));
    let cancel = gtk::Button::with_label(t("Cancel"));
    let save = gtk::Button::with_label(t("Save"));
    save.add_css_class("suggested-action");
    header.pack_start(&cancel);
    header.pack_end(&save);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&body));
    dialog.set_child(Some(&view));
    // Enter saves.
    dialog.set_default_widget(Some(&save));

    cancel.connect_clicked({
        let dialog = dialog.downgrade();
        move |_| {
            if let Some(d) = dialog.upgrade() {
                d.close();
            }
        }
    });
    let save_and_close = Rc::new({
        let (e, dialog) = (Rc::downgrade(&e), dialog.downgrade());
        move || {
            if let (Some(e), Some(d)) = (e.upgrade(), dialog.upgrade()) {
                e.save();
                d.close();
            }
        }
    });
    save.connect_clicked({
        let save_and_close = save_and_close.clone();
        move |_| save_and_close()
    });
    // Before any button sees it: Space listens, like in video editors, wherever the focus is.
    // Only typing a time keeps its Space.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let (e, dialog) = (Rc::downgrade(&e), dialog.downgrade());
        move |_, key, _, mods| {
            let (Some(e), Some(d)) = (e.upgrade(), dialog.upgrade()) else { return glib::Propagation::Proceed };
            let typing = d.focus().is_some_and(|f| f.is::<gtk::Text>());
            match key {
                gdk::Key::space | gdk::Key::KP_Space if !typing => {
                    e.toggle_play();
                    glib::Propagation::Stop
                }
                gdk::Key::s | gdk::Key::S if mods.contains(gdk::ModifierType::CONTROL_MASK) => {
                    save_and_close();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        }
    });
    dialog.add_controller(keys);
    // The playhead moves while listening.
    wave.add_tick_callback({
        let e = Rc::downgrade(&e);
        move |_, _| {
            let Some(e) = e.upgrade() else { return glib::ControlFlow::Break };
            e.follow();
            glib::ControlFlow::Continue
        }
    });
    e.update();
    // The dialog keeps the editor while it is open.
    dialog.connect_closed(move |_| e.stop());
    // The keys work right away.
    dialog.set_focus(Some(&wave));
    dialog.present(Some(parent));
    wave.grab_focus();
}

/// The keys of the editor, as little key caps.
fn key_hints() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    row.set_halign(gtk::Align::Center);
    for (keys, what) in [
        (vec![t("Space")], t("Listen")),
        (vec!["←", "→"], t("Move the playhead")),
        (vec!["I", "O"], t("Set start and end")),
    ] {
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        for k in keys {
            item.append(&label(k, &["vn-key"]));
        }
        let text = label(what, &["dim-label", "caption"]);
        text.set_margin_start(2);
        item.append(&text);
        row.append(&item);
    }
    row
}

fn seconds(v: f32) -> String {
    format!("{v:.2} s")
}

impl Trim {
    fn time(&self, side: Side) -> f32 {
        match side {
            Side::Start => self.start.get(),
            Side::End => self.end.get(),
        }
    }

    /// Moves one side, but never past the other one.
    fn set(&self, side: Side, v: f32) {
        match side {
            Side::Start => self.start.set(v.clamp(0.0, self.end.get() - SHORTEST)),
            Side::End => self.end.set(v.clamp(self.start.get() + SHORTEST, self.length)),
        }
        self.head.set(self.head.get().clamp(self.start.get(), self.end.get()));
        self.stop();
        self.update();
    }

    /// Moves the playhead, inside the cut.
    fn set_head(&self, v: f32) {
        self.head.set(v.clamp(self.start.get(), self.end.get()));
        self.refresh();
    }

    /// Moves the playhead and goes on listening from there if it was playing.
    fn seek(&self, v: f32) {
        let was = self.playing.get().is_some();
        self.stop();
        self.set_head(v);
        if was {
            self.toggle_play();
        }
    }

    fn update(&self) {
        self.start_field.set_text(&seconds(self.start.get()));
        self.end_field.set_text(&seconds(self.end.get()));
        self.cut_length.set_label(&seconds(self.end.get() - self.start.get()));
        self.refresh();
    }

    /// Redraws, and puts the cursor, the bubble and the play button in step.
    fn refresh(&self) {
        self.wave.queue_draw();
        self.show_play();
        let w = self.wave.width() as f64;
        let cursor = match (self.dragging.get(), self.hover.get()) {
            (Some(Grab::Head), _) => "grabbing",
            (Some(Grab::Handle(..)), _) | (None, Some(Zone::Handle(_))) => "ew-resize",
            (None, Some(Zone::Head)) => "grab",
            _ => "pointer",
        };
        self.wave.set_cursor_from_name(Some(cursor));
        // The time of what is dragged or pointed at.
        let shown = match (self.dragging.get(), self.hover.get()) {
            (Some(Grab::Head), _) | (None, Some(Zone::Head)) => Some(self.head.get()),
            (Some(Grab::Handle(side, _)), _) | (None, Some(Zone::Handle(side))) => Some(self.time(side)),
            (None, Some(Zone::Inside)) => Some(self.t(self.pointer.get(), w)),
            _ => None,
        };
        match shown {
            Some(time) => {
                self.bubble.set_label(&seconds(time));
                let (_, width, _, _) = self.bubble.measure(gtk::Orientation::Horizontal, -1);
                let x = self.x(time, w) - width as f64 / 2.0;
                self.bubble.set_margin_start(x.clamp(0.0, (w - width as f64).max(0.0)) as i32);
                self.bubble.set_visible(true);
            }
            None => self.bubble.set_visible(false),
        }
    }

    fn show_play(&self) {
        let (icon, text) = if self.playing.get().is_some() {
            ("media-playback-pause-symbolic", t("Pause"))
        } else {
            ("media-playback-start-symbolic", t("Listen"))
        };
        self.play.set_child(Some(&adw::ButtonContent::builder().icon_name(icon).label(text).build()));
    }

    /// Where a time is on the waveform, and back.
    fn x(&self, t: f32, w: f64) -> f64 {
        HANDLE + t as f64 / self.length as f64 * (w - 2.0 * HANDLE)
    }

    fn t(&self, x: f64, w: f64) -> f32 {
        ((x - HANDLE) / (w - 2.0 * HANDLE)).clamp(0.0, 1.0) as f32 * self.length
    }

    fn zone(&self, x: f64) -> Zone {
        let w = self.wave.width() as f64;
        let (sx, ex, hx) = (self.x(self.start.get(), w), self.x(self.end.get(), w), self.x(self.head.get(), w));
        // The handles themselves come first, then the playhead, then a little room inside the
        // cut next to the handles.
        if x >= sx - HANDLE - 3.0 && x <= sx {
            Zone::Handle(Side::Start)
        } else if x >= ex && x <= ex + HANDLE + 3.0 {
            Zone::Handle(Side::End)
        } else if (x - hx).abs() <= HEAD_REACH {
            Zone::Head
        } else if x > sx && x <= sx + 4.0 {
            Zone::Handle(Side::Start)
        } else if x >= ex - 4.0 && x < ex {
            Zone::Handle(Side::End)
        } else if x > sx && x < ex {
            Zone::Inside
        } else {
            Zone::Outside
        }
    }

    fn pointer_events(self: &Rc<Self>) {
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion({
            let e = Rc::downgrade(self);
            move |_, x, _| {
                if let Some(e) = e.upgrade() {
                    e.pointer.set(x);
                    e.hover.set(Some(e.zone(x)));
                    e.refresh();
                }
            }
        });
        motion.connect_leave({
            let e = Rc::downgrade(self);
            move |_| {
                if let Some(e) = e.upgrade() {
                    e.hover.set(None);
                    e.refresh();
                }
            }
        });
        self.wave.add_controller(motion);

        let drag = gtk::GestureDrag::new();
        drag.connect_drag_begin({
            let e = Rc::downgrade(self);
            move |g, x, _| {
                let Some(e) = e.upgrade() else { return };
                g.set_state(gtk::EventSequenceState::Claimed);
                e.wave.grab_focus();
                // Listening pauses while something moves, and goes on after.
                e.resume.set(e.playing.get().is_some());
                e.stop();
                let w = e.wave.width() as f64;
                let grab = match e.zone(x) {
                    Zone::Head | Zone::Inside => Grab::Head,
                    Zone::Handle(side) => {
                        let edge = e.x(e.time(side), w);
                        Grab::Handle(side, edge - x)
                    }
                    Zone::Outside => {
                        let (sx, ex) = (e.x(e.start.get(), w), e.x(e.end.get(), w));
                        Grab::Handle(if x < sx { Side::Start } else if x > ex { Side::End } else { Side::Start }, 0.0)
                    }
                };
                e.dragging.set(Some(grab));
                match grab {
                    Grab::Head => e.set_head(e.t(x, w)),
                    Grab::Handle(side, offset) => e.set(side, e.t(x + offset, w)),
                }
            }
        });
        drag.connect_drag_update({
            let e = Rc::downgrade(self);
            move |g, dx, _| {
                let (Some(e), Some((x, _))) = (e.upgrade(), g.start_point()) else { return };
                let w = e.wave.width() as f64;
                e.pointer.set(x + dx);
                match e.dragging.get() {
                    Some(Grab::Head) => e.set_head(e.t(x + dx, w)),
                    Some(Grab::Handle(side, offset)) => e.set(side, e.t(x + dx + offset, w)),
                    None => {}
                }
            }
        });
        drag.connect_drag_end({
            let e = Rc::downgrade(self);
            move |_, _, _| {
                let Some(e) = e.upgrade() else { return };
                e.dragging.set(None);
                e.hover.set(Some(e.zone(e.pointer.get())));
                e.refresh();
                if e.resume.take() {
                    e.toggle_play();
                }
            }
        });
        self.wave.add_controller(drag);
    }

    /// Keys while the waveform has the focus.
    fn wave_keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let e = Rc::downgrade(self);
            move |_, key, _, mods| {
                let Some(e) = e.upgrade() else { return glib::Propagation::Proceed };
                let step = if mods.contains(gdk::ModifierType::SHIFT_MASK) { 1.0 } else { 0.1 };
                match key {
                    gdk::Key::Left | gdk::Key::KP_Left => e.seek(e.head.get() - step),
                    gdk::Key::Right | gdk::Key::KP_Right => e.seek(e.head.get() + step),
                    gdk::Key::Home | gdk::Key::KP_Home => e.seek(e.start.get()),
                    gdk::Key::End | gdk::Key::KP_End => e.seek(e.end.get()),
                    gdk::Key::i | gdk::Key::I => e.set(Side::Start, e.head.get()),
                    gdk::Key::o | gdk::Key::O => e.set(Side::End, e.head.get()),
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            }
        });
        self.wave.add_controller(keys);
    }

    /// Listens from the playhead, or pauses where it is.
    fn toggle_play(&self) {
        if self.playing.get().is_some() {
            self.stop();
            self.show_play();
            return;
        }
        // At the end, start over.
        let from = if self.head.get() >= self.end.get() - 0.02 { self.start.get() } else { self.head.get() };
        self.from.set(from);
        self.head.set(from);
        let cut = soundboard::trim(&self.data, from, Some(self.end.get()));
        self.st.preview(Arc::new(cut));
        self.playing.set(Some(Instant::now()));
        self.show_play();
    }

    fn stop(&self) {
        if self.playing.take().is_some() {
            self.st.stop_preview();
        }
    }

    /// Moves the playhead along, and notices when the sound is over.
    fn follow(&self) {
        let Some(started) = self.playing.get() else { return };
        match self.st.preview_progress() {
            Some(p) => self.head.set(self.from.get() + p * (self.end.get() - self.from.get())),
            // The engine needs a moment to start it.
            None if started.elapsed() > Duration::from_millis(250) => {
                self.playing.set(None);
                self.head.set(self.from.get());
                self.show_play();
            }
            None => {}
        }
        self.wave.queue_draw();
    }

    fn save(&self) {
        self.stop();
        let Some(mut sound) = self.st.library.borrow().find(&self.id).cloned() else { return };
        let (start, end) = (self.start.get(), self.end.get());
        // A hair from the edge counts as no cut.
        sound.start = if start < 0.005 { 0.0 } else { start };
        sound.end = if end > self.length - 0.005 { None } else { Some(end) };
        self.st.update_sound(sound);
    }

    fn draw(&self, area: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64) {
        let fg = area.color();
        let (fr, fgr, fb) = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);
        let (sx, ex) = (self.x(self.start.get(), w), self.x(self.end.get(), w));
        let (left, right) = (HANDLE, w - HANDLE);
        let mid = h / 2.0;

        // Ground.
        rounded(cr, left, 0.0, right - left, h, 10.0);
        cr.set_source_rgba(fr, fgr, fb, 0.05);
        let _ = cr.fill();

        // Waveform: a bar every 4 pixels, colored inside the cut and grey outside.
        let grad = cairo::LinearGradient::new(sx, 0.0, ex, 0.0);
        grad.add_color_stop_rgb(0.0, 0.15, 0.39, 0.92);
        grad.add_color_stop_rgb(0.5, 0.23, 0.51, 0.96);
        grad.add_color_stop_rgb(1.0, 0.13, 0.83, 0.93);
        let columns = ((right - left) / 4.0).max(1.0) as usize;
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_width(2.5);
        for c in 0..columns {
            let a = c * self.peaks.len() / columns;
            let b = ((c + 1) * self.peaks.len() / columns).max(a + 1).min(self.peaks.len());
            let peak = self.peaks[a..b].iter().fold(0.0f32, |m, p| m.max(*p)) as f64;
            let x = left + 2.0 + c as f64 * 4.0;
            let half = (peak.powf(0.7) * (h / 2.0 - 14.0)).max(1.0);
            cr.move_to(x, mid - half);
            cr.line_to(x, mid + half);
            if x >= sx && x <= ex {
                let _ = cr.set_source(&grad);
            } else {
                cr.set_source_rgba(fr, fgr, fb, 0.22);
            }
            let _ = cr.stroke();
        }

        // Where a click would put the playhead.
        if self.dragging.get().is_none() && self.hover.get() == Some(Zone::Inside) {
            let x = self.pointer.get();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
            cr.set_line_width(1.0);
            cr.move_to(x, 6.0);
            cr.line_to(x, h - 6.0);
            let _ = cr.stroke();
        }

        // The frame of the cut, with a handle on each side. The one under the pointer lights up.
        let lit = |side: Side| match (self.dragging.get(), self.hover.get()) {
            (Some(Grab::Handle(s, _)), _) => s == side,
            (None, Some(Zone::Handle(s))) => s == side,
            _ => false,
        };
        cr.set_source_rgb(ACCENT.0, ACCENT.1, ACCENT.2);
        cr.set_line_width(3.0);
        cr.rectangle(sx, 1.5, ex - sx, h - 3.0);
        let _ = cr.stroke();
        for (side, x) in [(Side::Start, sx - HANDLE), (Side::End, ex)] {
            let c = if lit(side) { ACCENT_LIGHT } else { ACCENT };
            rounded(cr, x, 0.0, HANDLE, h, 6.0);
            cr.set_source_rgb(c.0, c.1, c.2);
            let _ = cr.fill();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
            cr.set_line_width(2.0);
            for dx in [-2.0, 2.0] {
                cr.move_to(x + HANDLE / 2.0 + dx, mid - 10.0);
                cr.line_to(x + HANDLE / 2.0 + dx, mid + 10.0);
            }
            let _ = cr.stroke();
        }

        // Playhead, with a knob to grab it. It grows under the pointer.
        let x = self.x(self.head.get(), w);
        let big = self.dragging.get() == Some(Grab::Head) || (self.dragging.get().is_none() && self.hover.get() == Some(Zone::Head));
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_line_width(2.0);
        cr.move_to(x, 12.0);
        cr.line_to(x, h - 4.0);
        let _ = cr.stroke();
        cr.arc(x, 10.0, if big { 7.5 } else { 6.0 }, 0.0, std::f64::consts::TAU);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.3);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    }
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}
