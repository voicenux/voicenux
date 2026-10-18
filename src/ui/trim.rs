//! Cuts the start and end of a sound, like a video clip: the waveform with two handles, a time
//! ruler under it, and a playhead that says where listening starts. Listening plays in the
//! headphones only.
//!
//! Mouse: drag a handle to cut, drag the playhead, click into the cut or the ruler to move it,
//! click outside the cut to move the nearest handle there. The playhead may leave the cut: it
//! sticks to the ends of the cut for a moment, then tears away. Ctrl with the wheel zooms, the wheel
//! alone scrolls along a zoomed sound. Keys work wherever the focus is, only typing a time keeps
//! them: Space listens or pauses, ← and → move the playhead (Shift for bigger steps, Ctrl for
//! smaller ones), Home and End or ↑ and ↓ jump to the ends of the cut, I and O set the start and
//! end at the playhead, + and − zoom, 0 shows all, Ctrl+Z undoes, Ctrl+S saves, Escape cancels.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib, graphene, gsk, pango};

use super::visuals::{BRAND, Canvas, rgba};
use super::{label, value_field};
use crate::i18n::{fill, t, t_dyn};
use crate::soundboard::{self, RATE, SoundData};
use crate::state::{Change, St};

/// Width of a handle.
const HANDLE: f32 = 14.0;
/// Height of the time ruler under the waveform.
const RULER: f32 = 30.0;
/// Height of the waveform.
const WAVE: f32 = 150.0;
/// The shortest sound a cut leaves, in seconds.
const SHORTEST: f32 = 0.05;
/// How finely the waveform is kept, at least and at most. Long sounds get more, for zooming.
const PEAKS: (usize, usize) = (1600, 120_000);
/// The shortest stretch the editor zooms in to, in seconds.
const CLOSEST: f32 = 0.05;
/// How far the pointer may be from the playhead to grab it.
const HEAD_REACH: f32 = 7.0;
/// How close to an end of the cut a dragged playhead sticks to it, in pixels.
const STICK: f32 = 12.0;
/// Labels on the ruler are at least this far apart, in pixels.
const LABEL_GAP: f32 = 70.0;
const ACCENT: (f32, f32, f32) = (0.23, 0.51, 0.96);
const ACCENT_LIGHT: (f32, f32, f32) = (0.42, 0.66, 1.0);
const WHITE: (f32, f32, f32) = (1.0, 1.0, 1.0);

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Start,
    End,
}

/// What is under the pointer.
#[derive(Clone, Copy, PartialEq)]
enum Zone {
    /// The time ruler: a click moves the playhead there.
    Ruler,
    Handle(Side),
    Head,
    /// Inside the cut: a click moves the playhead there.
    Inside,
    /// Outside the cut: a click moves the nearest handle there.
    Outside,
}

/// What a drag moves.
#[derive(Clone, Copy, PartialEq)]
enum Grab {
    /// A handle, and how far the pointer is from its edge.
    Handle(Side, f32),
    Head,
}

struct Trim {
    st: St,
    id: String,
    data: Arc<SoundData>,
    peaks: Vec<f32>,
    /// All of the sound, in seconds.
    length: f32,
    /// The sound without its silent edges: what plays without a cut.
    whole: (f32, f32),
    start: Cell<f32>,
    end: Cell<f32>,
    /// The cut as it is saved, to know if there is something to lose.
    saved: (f32, f32),
    /// Earlier cuts for Ctrl+Z, and undone ones for Ctrl+Shift+Z.
    undo: RefCell<Vec<(f32, f32)>>,
    redo: RefCell<Vec<(f32, f32)>>,
    dialog: glib::WeakRef<adw::Dialog>,
    wave: Canvas,
    /// The part of the sound on screen: its value is where it starts, its page size how long it
    /// is. All of it, until you zoom.
    view: gtk::Adjustment,
    /// Scrolls a zoomed sound. Clear while all of it shows.
    scrollbar: gtk::Scrollbar,
    /// The time of what is dragged or pointed at, over the waveform.
    bubble: gtk::Label,
    start_field: gtk::EditableLabel,
    end_field: gtk::EditableLabel,
    cut_length: gtk::Label,
    /// Where the playhead is, next to the play button.
    clock: gtk::Label,
    total: gtk::Label,
    play: gtk::Button,
    /// When listening started, while it plays.
    playing: Cell<Option<Instant>>,
    /// The playhead, in seconds of the whole sound.
    head: Cell<f32>,
    /// Where listening started. The playhead goes back there when the sound is over.
    from: Cell<f32>,
    /// Where listening stops: the end of the cut, or of the sound behind the cut.
    until: Cell<f32>,
    /// Listening went on when a drag began, so it goes on after.
    resume: Cell<bool>,
    hover: Cell<Option<Zone>>,
    /// Where the pointer is, in pixels.
    pointer: Cell<(f32, f32)>,
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

    // The editor draws itself; it exists only after the widgets, so they reach it through this.
    let editor: Rc<std::cell::OnceCell<std::rc::Weak<Trim>>> = Rc::default();
    let with = {
        let editor = editor.clone();
        move || editor.get().and_then(|e| e.upgrade())
    };

    let wave = Canvas::new(-1, (RULER + WAVE) as i32, {
        let with = with.clone();
        move |w, s, width, height| {
            if let Some(e) = with() {
                e.draw(w, s, width, height);
            }
        }
    });
    wave.set_hexpand(true);
    wave.set_focusable(true);
    wave.add_css_class("vn-trim-wave");
    wave.update_property(&[gtk::accessible::Property::Label(t("Waveform"))]);
    let bubble = label("", &["vn-trim-bubble", "numeric"]);
    bubble.set_halign(gtk::Align::Start);
    bubble.set_valign(gtk::Align::Start);
    bubble.set_margin_top(6);
    bubble.set_can_target(false);
    bubble.set_visible(false);
    let wave_box = gtk::Overlay::new();
    wave_box.set_child(Some(&wave));
    wave_box.add_overlay(&bubble);

    let play = gtk::Button::new();
    play.add_css_class("vn-gradient-button");
    play.set_tooltip_text(Some(t("Listen or pause (Space)")));

    let field = |side: Side| {
        let (w1, w2) = (with.clone(), with.clone());
        value_field(
            move || w1().map(|e| format!("{:.2}", e.time(side))).unwrap_or_default(),
            move |f, text| {
                let Some(e) = w2() else { return };
                if let Ok(v) = text.replace(',', ".").trim_end_matches('s').trim().parse::<f32>()
                    && v.is_finite()
                    && (v - e.time(side)).abs() > 1e-4
                {
                    e.remember();
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

    let dialog = adw::Dialog::builder().content_width(680).build();
    let whole = soundboard::audible(&data);
    let (start, end) = if sound.is_trimmed() { (sound.start, sound.end.unwrap_or(length)) } else { whole };
    let start = start.clamp(0.0, length - SHORTEST);
    let mut end = end.clamp(SHORTEST, length);
    if end <= start {
        end = length;
    }
    let view = gtk::Adjustment::new(0.0, 0.0, length as f64, 0.1, 1.0, length as f64);
    let scrollbar = gtk::Scrollbar::new(gtk::Orientation::Horizontal, Some(&view));
    scrollbar.add_css_class("vn-trim-scroll");
    scrollbar.set_opacity(0.0);
    scrollbar.set_can_target(false);
    let e = Rc::new(Trim {
        st: st.clone(),
        id: id.to_string(),
        peaks: soundboard::peaks(&data, (data.frames.len() / 32).clamp(PEAKS.0, PEAKS.1)),
        data,
        length,
        whole,
        start: Cell::new(start),
        end: Cell::new(end),
        saved: (start, end),
        undo: RefCell::default(),
        redo: RefCell::default(),
        dialog: dialog.downgrade(),
        wave: wave.clone(),
        view: view.clone(),
        scrollbar: scrollbar.clone(),
        bubble,
        start_field: start_field.clone(),
        end_field: end_field.clone(),
        cut_length: label("", &["numeric"]),
        clock: label("", &["vn-trim-clock", "numeric"]),
        total: label(&format!("/ {}", seconds(length)), &["dim-label", "numeric"]),
        play: play.clone(),
        playing: Cell::new(None),
        head: Cell::new(start),
        from: Cell::new(start),
        until: Cell::new(end),
        resume: Cell::new(false),
        hover: Cell::new(None),
        pointer: Cell::new((0.0, 0.0)),
        dragging: Cell::new(None),
    });
    let _ = editor.set(Rc::downgrade(&e));
    e.pointer_events();
    e.wheel();
    view.connect_value_changed({
        let with = with.clone();
        move |_| {
            if let Some(e) = with() {
                e.refresh();
            }
        }
    });
    play.connect_clicked({
        let with = with.clone();
        move |_| {
            if let Some(e) = with() {
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
        let with = with.clone();
        move |_| {
            if let Some(e) = with() {
                e.remember();
                e.set(Side::Start, e.whole.0);
                e.set(Side::End, e.whole.1);
            }
        }
    });
    // Like a video player: play, and where the playhead is.
    let transport = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    transport.append(&play);
    let clock = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    clock.append(&e.clock);
    clock.append(&e.total);
    transport.append(&clock);
    let zoom_button = |icon: &str, tip: &str, key: &str, factor: f32| {
        let b = gtk::Button::from_icon_name(icon);
        b.add_css_class("circular");
        b.set_valign(gtk::Align::Center);
        b.set_tooltip_text(Some(&format!("{tip} ({key})")));
        let with = with.clone();
        b.connect_clicked(move |_| {
            if let Some(e) = with() {
                e.zoom(factor, e.head.get());
            }
        });
        b
    };
    let tools = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    tools.append(&zoom_button("zoom-out-symbolic", t("Zoom out"), "−", 1.0 / 1.6));
    tools.append(&zoom_button("zoom-in-symbolic", t("Zoom in"), "+", 1.6));
    reset.set_margin_start(4);
    tools.append(&reset);
    let buttons = gtk::CenterBox::new();
    buttons.set_start_widget(Some(&transport));
    buttons.set_end_widget(Some(&tools));

    let headphones = label(t("Listening plays in your headphones only."), &["dim-label", "caption"]);
    headphones.set_ellipsize(gtk::pango::EllipsizeMode::End);
    headphones.set_xalign(0.0);
    headphones.set_hexpand(true);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
    body.set_margin_top(8);
    body.set_margin_bottom(14);
    body.set_margin_start(20);
    body.set_margin_end(20);
    let wave_and_bar = gtk::Box::new(gtk::Orientation::Vertical, 2);
    wave_and_bar.append(&wave_box);
    scrollbar.set_margin_start(HANDLE as i32);
    scrollbar.set_margin_end(HANDLE as i32);
    wave_and_bar.append(&scrollbar);
    body.append(&wave_and_bar);
    body.append(&times);
    body.append(&buttons);
    body.append(&key_hints());

    // Cancel and Save at the bottom right, where other desktops have them.
    let cancel = gtk::Button::with_label(t("Cancel"));
    cancel.set_tooltip_text(Some(t("Close without saving (Esc)")));
    let save = gtk::Button::with_label(t("Save"));
    save.add_css_class("suggested-action");
    save.set_tooltip_text(Some(&super::with_key(t("Save the cut"), "S")));
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    actions.append(&cancel);
    actions.append(&save);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    footer.add_css_class("vn-dialog-footer");
    footer.append(&headphones);
    footer.append(&actions);

    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_title_widget(Some(&adw::WindowTitle::new(t("Trim sound"), &t_dyn(&sound.name))));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&body));
    view.add_bottom_bar(&footer);
    dialog.set_child(Some(&view));
    // Enter saves.
    dialog.set_default_widget(Some(&save));

    cancel.connect_clicked({
        let dialog = dialog.downgrade();
        move |_| {
            if let Some(d) = dialog.upgrade() {
                d.force_close();
            }
        }
    });
    let save_and_close = Rc::new({
        let (with, dialog) = (with.clone(), dialog.downgrade());
        move || {
            if let (Some(e), Some(d)) = (with(), dialog.upgrade()) {
                e.save();
                d.force_close();
            }
        }
    });
    save.connect_clicked({
        let save_and_close = save_and_close.clone();
        move |_| save_and_close()
    });
    // Before any button sees them, so a focused button never takes Space or the arrows. Only
    // typing a time keeps its keys.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let (with, dialog) = (with.clone(), dialog.downgrade());
        move |c, key, _, mods| {
            let (Some(e), Some(d)) = (with(), dialog.upgrade()) else { return glib::Propagation::Proceed };
            // Keys in a menu over the dialog, like the one of a time field, belong to the menu.
            if c.current_event().and_then(|ev| ev.surface()) != d.native().and_then(|n| n.surface()) {
                return glib::Propagation::Proceed;
            }
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
            if ctrl && matches!(key, gdk::Key::s | gdk::Key::S) {
                save_and_close();
                return glib::Propagation::Stop;
            }
            if d.focus().is_some_and(|f| f.is::<gtk::Text>()) {
                return glib::Propagation::Proceed;
            }
            let step = if shift {
                1.0
            } else if ctrl {
                0.01
            } else {
                0.1
            };
            match key {
                gdk::Key::space | gdk::Key::KP_Space => e.toggle_play(),
                gdk::Key::Left | gdk::Key::KP_Left => e.seek(e.head.get() - step),
                gdk::Key::Right | gdk::Key::KP_Right => e.seek(e.head.get() + step),
                gdk::Key::Home | gdk::Key::KP_Home | gdk::Key::Up | gdk::Key::KP_Up => e.seek(e.start.get()),
                gdk::Key::End | gdk::Key::KP_End | gdk::Key::Down | gdk::Key::KP_Down => e.seek(e.end.get()),
                gdk::Key::i | gdk::Key::I if !ctrl => {
                    e.remember();
                    e.set(Side::Start, e.head.get());
                }
                gdk::Key::o | gdk::Key::O if !ctrl => {
                    e.remember();
                    e.set(Side::End, e.head.get());
                }
                gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add if !ctrl => e.zoom(1.6, e.head.get()),
                gdk::Key::minus | gdk::Key::KP_Subtract if !ctrl => e.zoom(1.0 / 1.6, e.head.get()),
                gdk::Key::_0 | gdk::Key::KP_0 if !ctrl => e.zoom(0.0, 0.0),
                gdk::Key::z | gdk::Key::Z if ctrl && shift => e.step_history(false),
                gdk::Key::z | gdk::Key::Z if ctrl => e.step_history(true),
                gdk::Key::y | gdk::Key::Y if ctrl => e.step_history(false),
                gdk::Key::Escape => d.force_close(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });
    dialog.add_controller(keys);
    // The playhead moves while listening.
    wave.add_tick_callback({
        let with = with.clone();
        move |_, _| {
            let Some(e) = with() else { return glib::ControlFlow::Break };
            e.follow();
            glib::ControlFlow::Continue
        }
    });
    e.update();
    // Closing the window with a cut that is not saved: the dialog pulses instead.
    dialog.connect_close_attempt(super::popup::nudge);
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
        (vec![t("Spacebar")], t("Listen")),
        (vec!["←", "→"], t("Move the playhead")),
        (vec!["I", "O"], t("Set start and end")),
        (vec![t("Ctrl"), "Z"], t("Undo")),
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

/// A good step between labels on the ruler, and how many small ticks it has, for `per_second`
/// pixels in a second.
fn ruler_steps(per_second: f32) -> (f32, u32) {
    const STEPS: [(f32, u32); 16] = [
        (0.05, 5),
        (0.1, 5),
        (0.2, 4),
        (0.25, 5),
        (0.5, 5),
        (1.0, 5),
        (2.0, 4),
        (5.0, 5),
        (10.0, 5),
        (15.0, 3),
        (30.0, 6),
        (60.0, 6),
        (120.0, 4),
        (300.0, 5),
        (600.0, 6),
        (1800.0, 6),
    ];
    STEPS.into_iter().find(|(step, _)| step * per_second >= LABEL_GAP).unwrap_or(STEPS[STEPS.len() - 1])
}

/// A ruler label: "1.5 s", or "1:30" in sounds of a minute or more.
fn ruler_label(v: f32, step: f32, long: bool) -> String {
    // As many decimals as the step needs: 0.25 needs two, 0.5 one.
    let decimals = (0..3).find(|d| ((step * 10f32.powi(*d)).fract()).abs() < 1e-3).unwrap_or(2) as usize;
    if long {
        let whole = v.floor() as u32;
        let rest = if decimals > 0 { format!("{:.*}", decimals, v.fract()).trim_start_matches('0').to_string() } else { String::new() };
        format!("{}:{:02}{rest}", whole / 60, whole % 60)
    } else {
        format!("{v:.decimals$} s")
    }
}

/// A rectangle with round corners, filled.
fn fill_round(s: &gtk::Snapshot, rect: graphene::Rect, r: f32, color: &gdk::RGBA) {
    let rounded = gsk::RoundedRect::from_rect(rect, r);
    s.push_rounded_clip(&rounded);
    s.append_color(color, &rect);
    s.pop();
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
        self.stop();
        self.update();
    }

    /// Keeps the cut as it is now for Ctrl+Z. Called before every change.
    fn remember(&self) {
        let now = (self.start.get(), self.end.get());
        let mut undo = self.undo.borrow_mut();
        if undo.last() != Some(&now) {
            undo.push(now);
        }
        self.redo.borrow_mut().clear();
    }

    /// Goes back one cut, or forward again.
    fn step_history(&self, back: bool) {
        let (from, to) = if back { (&self.undo, &self.redo) } else { (&self.redo, &self.undo) };
        let Some((start, end)) = from.borrow_mut().pop() else { return };
        to.borrow_mut().push((self.start.get(), self.end.get()));
        self.start.set(start);
        self.end.set(end);
        self.stop();
        self.update();
    }

    /// Moves the playhead, anywhere in the sound.
    fn set_head(&self, v: f32) {
        self.head.set(v.clamp(0.0, self.length));
        self.refresh();
    }

    /// A time near an end of the cut snaps to it, so the playhead sticks there for a moment
    /// before it tears away.
    fn stick(&self, t: f32, w: f32) -> f32 {
        let x = self.x(t, w);
        [self.start.get(), self.end.get()]
            .into_iter()
            .find(|edge| (x - self.x(*edge, w)).abs() <= STICK)
            .unwrap_or(t)
            .clamp(0.0, self.length)
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
        let (start, end) = (self.start.get(), self.end.get());
        self.start_field.set_text(&seconds(start));
        self.end_field.set_text(&seconds(end));
        self.cut_length.set_label(&seconds(end - start));
        // A cut that is not saved yet stays open on a click next to the dialog.
        if let Some(d) = self.dialog.upgrade() {
            d.set_can_close((start, end) == self.saved);
        }
        self.refresh();
    }

    /// Redraws, and puts the cursor, the bubble, the clock and the play button in step.
    fn refresh(&self) {
        self.wave.queue_draw();
        self.show_play();
        self.show_clock();
        let w = self.wave.width() as f32;
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
            (None, Some(Zone::Inside | Zone::Ruler)) => Some(self.aim(w)),
            _ => None,
        };
        match shown {
            Some(time) => {
                self.bubble.set_label(&seconds(time));
                let (_, width, _, _) = self.bubble.measure(gtk::Orientation::Horizontal, -1);
                let x = self.x(time, w) - width as f32 / 2.0;
                self.bubble.set_margin_start(x.clamp(0.0, (w - width as f32).max(0.0)) as i32);
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

    fn show_clock(&self) {
        let text = seconds(self.head.get());
        if self.clock.label() != text {
            self.clock.set_label(&text);
        }
    }

    /// The part of the sound on screen: where it starts, and how long it is.
    fn shown(&self) -> (f32, f32) {
        (self.view.value() as f32, self.view.page_size() as f32)
    }

    /// Where a time is on the waveform, and back.
    fn x(&self, t: f32, w: f32) -> f32 {
        let (from, span) = self.shown();
        HANDLE + (t - from) / span * (w - 2.0 * HANDLE)
    }

    fn t(&self, x: f32, w: f32) -> f32 {
        let (from, span) = self.shown();
        from + ((x - HANDLE) / (w - 2.0 * HANDLE)).clamp(0.0, 1.0) * span
    }

    /// Zooms in by `factor` (out below 1) and keeps `around` where it is on screen. A factor of
    /// 0 shows all of the sound again.
    fn zoom(&self, factor: f32, around: f32) {
        let (from, span) = self.shown();
        let w = (self.wave.width() as f32 - 2.0 * HANDLE).max(1.0);
        // So close that a column of the waveform still has a peak of its own.
        let closest = CLOSEST.max(self.length * (w / 4.0) / self.peaks.len() as f32).min(self.length);
        let new_span = if factor <= 0.0 { self.length } else { (span / factor).clamp(closest, self.length) };
        let at = ((around - from) / span).clamp(0.0, 1.0);
        let new_from = (around - at * new_span).clamp(0.0, self.length - new_span);
        self.view.configure(
            new_from as f64,
            0.0,
            self.length as f64,
            (new_span / 20.0) as f64,
            (new_span * 0.9) as f64,
            new_span as f64,
        );
        let zoomed = new_span < self.length * 0.999;
        self.scrollbar.set_opacity(if zoomed { 1.0 } else { 0.0 });
        self.scrollbar.set_can_target(zoomed);
        self.refresh();
    }

    /// Moves a zoomed view by `dt` seconds.
    fn pan(&self, dt: f32) {
        let (from, span) = self.shown();
        self.view.set_value((from + dt).clamp(0.0, self.length - span) as f64);
    }

    /// Keeps the playhead on screen while it plays, a page at a time.
    fn keep_in_view(&self) {
        let (from, span) = self.shown();
        let head = self.head.get();
        if head > from + span || head < from {
            self.pan(head - from - span * 0.05);
        }
    }

    /// Ctrl with the wheel zooms at the pointer, the wheel alone scrolls a zoomed sound.
    fn wheel(self: &Rc<Self>) {
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.connect_scroll({
            let e = Rc::downgrade(self);
            move |c, dx, dy| {
                let Some(e) = e.upgrade() else { return glib::Propagation::Proceed };
                let w = e.wave.width() as f32;
                let (_, span) = e.shown();
                let wheel = c.unit() == gdk::ScrollUnit::Wheel;
                if c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                    let at = e.t(e.pointer.get().0, w);
                    let steps = if wheel { dy as f32 } else { dy as f32 / 40.0 };
                    e.zoom(1.25f32.powf(-steps), at);
                } else {
                    // Wheels scroll a tenth of the screen a notch, touchpads follow the fingers.
                    let per_px = span / (w - 2.0 * HANDLE).max(1.0);
                    let d = if dx != 0.0 { dx } else { dy } as f32;
                    e.pan(if wheel { d * span * 0.1 } else { d * per_px });
                }
                glib::Propagation::Stop
            }
        });
        self.wave.add_controller(scroll);
    }

    /// Where a click at the pointer would put the playhead.
    fn aim(&self, w: f32) -> f32 {
        self.stick(self.t(self.pointer.get().0, w), w)
    }

    fn zone(&self, x: f32, y: f32) -> Zone {
        let w = self.wave.width() as f32;
        let (sx, ex, hx) = (self.x(self.start.get(), w), self.x(self.end.get(), w), self.x(self.head.get(), w));
        // The playhead's knob sits in the ruler, the rest of the ruler seeks.
        if y >= self.wave.height() as f32 - RULER {
            return if (x - hx).abs() <= HEAD_REACH + 2.0 { Zone::Head } else { Zone::Ruler };
        }
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
            move |_, x, y| {
                if let Some(e) = e.upgrade() {
                    e.pointer.set((x as f32, y as f32));
                    e.hover.set(Some(e.zone(x as f32, y as f32)));
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
            move |g, x, y| {
                let Some(e) = e.upgrade() else { return };
                let (x, y) = (x as f32, y as f32);
                g.set_state(gtk::EventSequenceState::Claimed);
                e.wave.grab_focus();
                // Listening pauses while something moves, and goes on after.
                e.resume.set(e.playing.get().is_some());
                e.stop();
                let w = e.wave.width() as f32;
                let grab = match e.zone(x, y) {
                    Zone::Head | Zone::Inside | Zone::Ruler => Grab::Head,
                    Zone::Handle(side) => Grab::Handle(side, e.x(e.time(side), w) - x),
                    Zone::Outside => Grab::Handle(if x < e.x(e.start.get(), w) { Side::Start } else { Side::End }, 0.0),
                };
                if matches!(grab, Grab::Handle(..)) {
                    e.remember();
                }
                e.pointer.set((x, y));
                e.dragging.set(Some(grab));
                match grab {
                    Grab::Head => e.set_head(e.stick(e.t(x, w), w)),
                    Grab::Handle(side, offset) => e.set(side, e.t(x + offset, w)),
                }
            }
        });
        drag.connect_drag_update({
            let e = Rc::downgrade(self);
            move |g, dx, dy| {
                let (Some(e), Some((x, y))) = (e.upgrade(), g.start_point()) else { return };
                let (x, y) = ((x + dx) as f32, (y + dy) as f32);
                let w = e.wave.width() as f32;
                e.pointer.set((x, y));
                match e.dragging.get() {
                    Some(Grab::Head) => e.set_head(e.stick(e.t(x, w), w)),
                    Some(Grab::Handle(side, offset)) => e.set(side, e.t(x + offset, w)),
                    None => {}
                }
            }
        });
        drag.connect_drag_end({
            let e = Rc::downgrade(self);
            move |_, _, _| {
                let Some(e) = e.upgrade() else { return };
                // A drag that ended where it began is nothing to undo.
                let now = (e.start.get(), e.end.get());
                if matches!(e.dragging.get(), Some(Grab::Handle(..))) && e.undo.borrow().last() == Some(&now) {
                    e.undo.borrow_mut().pop();
                }
                e.dragging.set(None);
                let (x, y) = e.pointer.get();
                e.hover.set(Some(e.zone(x, y)));
                e.refresh();
                if e.resume.take() {
                    e.toggle_play();
                }
            }
        });
        self.wave.add_controller(drag);
    }

    /// Listens from the playhead, or pauses where it is.
    fn toggle_play(&self) {
        if self.playing.get().is_some() {
            self.stop();
            self.show_play();
            return;
        }
        // At the end of the cut, start over. Behind the cut, play on to the end of the sound, so
        // you hear what the cut leaves out.
        let (start, end, head) = (self.start.get(), self.end.get(), self.head.get());
        let (from, until) = if (head - end).abs() <= 0.02 || head >= self.length - 0.02 {
            (start, end)
        } else if head > end {
            (head, self.length)
        } else {
            (head, end)
        };
        self.from.set(from);
        self.until.set(until);
        self.head.set(from);
        let cut = soundboard::trim(&self.data, from, Some(until));
        self.st.preview(Arc::new(cut), 1.0);
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
            Some(p) => self.head.set(self.from.get() + p * (self.until.get() - self.from.get())),
            // The engine needs a moment to start it.
            None if started.elapsed() > Duration::from_millis(250) => {
                self.playing.set(None);
                self.head.set(self.from.get());
                self.show_play();
            }
            None => {}
        }
        self.keep_in_view();
        self.show_clock();
        self.wave.queue_draw();
    }

    fn save(&self) {
        self.stop();
        let Some(mut sound) = self.st.library.borrow().find(&self.id).cloned() else { return };
        let (start, end) = (self.start.get(), self.end.get());
        // A hair from the edge counts as no cut, and so does all of it but the silence.
        let near = |a: f32, b: f32| (a - b).abs() < 0.005;
        let whole = near(start, self.whole.0) && near(end, self.whole.1);
        sound.start = if start < 0.005 || whole { 0.0 } else { start };
        sound.end = if end > self.length - 0.005 || whole { None } else { Some(end) };
        self.st.update_sound(sound);
    }

    fn draw(&self, widget: &gtk::Widget, s: &gtk::Snapshot, w: f32, h: f32) {
        let fg = widget.color();
        let ink = (fg.red(), fg.green(), fg.blue());
        let (sx, ex) = (self.x(self.start.get(), w), self.x(self.end.get(), w));
        let (left, right) = (HANDLE, w - HANDLE);
        let bottom = h - RULER;
        let mid = bottom / 2.0;
        let dragging = self.dragging.get();
        let hover = if dragging.is_none() { self.hover.get() } else { None };
        // Zoomed in, the handles and the frame can lie outside.
        s.push_clip(&graphene::Rect::new(0.0, 0.0, w, h));

        self.draw_ruler(widget, s, w, h, ink);

        // Ground.
        fill_round(s, graphene::Rect::new(left, 0.0, right - left, bottom), 10.0, &rgba(ink, 0.05));

        // Waveform: a bar every 4 pixels, colored inside the cut and grey outside.
        let (inside, outside) = (gsk::PathBuilder::new(), gsk::PathBuilder::new());
        let columns = ((right - left) / 4.0).max(1.0) as usize;
        let (from, span) = self.shown();
        let n = self.peaks.len();
        let index = |t: f32| ((t / self.length * n as f32) as usize).min(n - 1);
        for c in 0..columns {
            let a = index(from + span * c as f32 / columns as f32);
            let b = index(from + span * (c + 1) as f32 / columns as f32).max(a + 1).min(n);
            let peak = self.peaks[a..b].iter().fold(0.0f32, |m, p| m.max(*p));
            let x = left + 2.0 + c as f32 * 4.0;
            let half = (peak.powf(0.7) * (bottom / 2.0 - 14.0)).max(1.0);
            let path = if x >= sx && x <= ex { &inside } else { &outside };
            path.move_to(x, mid - half);
            path.line_to(x, mid + half);
        }
        let bars = gsk::Stroke::new(2.5);
        bars.set_line_cap(gsk::LineCap::Round);
        s.append_stroke(&outside.to_path(), &bars, &rgba(ink, 0.22));
        s.push_stroke(&inside.to_path(), &bars);
        s.append_linear_gradient(
            &graphene::Rect::new(sx, 0.0, (ex - sx).max(1.0), bottom),
            &graphene::Point::new(sx, 0.0),
            &graphene::Point::new(ex.max(sx + 1.0), 0.0),
            &[
                gsk::ColorStop::new(0.0, rgba(BRAND[0], 1.0)),
                gsk::ColorStop::new(0.5, rgba(BRAND[1], 1.0)),
                gsk::ColorStop::new(1.0, rgba(BRAND[2], 1.0)),
            ],
        );
        s.pop();

        // Where a click would put the playhead.
        if matches!(hover, Some(Zone::Inside | Zone::Ruler)) {
            let x = self.x(self.aim(w), w);
            s.append_color(&rgba(WHITE, 0.35), &graphene::Rect::new(x - 0.5, 4.0, 1.0, h - 10.0));
        }

        // The frame of the cut, with a handle on each side. The one under the pointer lights up.
        let lit = |side: Side| match (dragging, hover) {
            (Some(Grab::Handle(sd, _)), _) => sd == side,
            (None, Some(Zone::Handle(sd))) => sd == side,
            _ => false,
        };
        let frame = gsk::RoundedRect::from_rect(graphene::Rect::new(sx - 1.5, 0.0, ex - sx + 3.0, bottom), 0.0);
        s.append_border(&frame, &[3.0; 4], &[rgba(ACCENT, 1.0); 4]);
        for (side, x) in [(Side::Start, sx - HANDLE), (Side::End, ex)] {
            let c = if lit(side) { ACCENT_LIGHT } else { ACCENT };
            fill_round(s, graphene::Rect::new(x, 0.0, HANDLE, bottom), 6.0, &rgba(c, 1.0));
            for dx in [-2.0, 2.0] {
                fill_round(s, graphene::Rect::new(x + HANDLE / 2.0 + dx - 1.0, mid - 10.0, 2.0, 20.0), 1.0, &rgba(WHITE, 0.9));
            }
        }

        // Playhead: a line, and a knob in the ruler to grab it. It grows under the pointer.
        let x = self.x(self.head.get(), w);
        let big = dragging == Some(Grab::Head) || hover == Some(Zone::Head);
        // Stuck to an end of the cut while dragged: the knob takes the color of the cut.
        let head = self.head.get();
        let stuck = dragging == Some(Grab::Head) && (head == self.start.get() || head == self.end.get());
        let knob_color = if stuck { ACCENT_LIGHT } else { WHITE };
        s.append_color(&rgba(WHITE, 1.0), &graphene::Rect::new(x - 1.0, 4.0, 2.0, h - 12.0));
        let (half, tall) = if big { (8.0, 17.0) } else { (6.5, 14.0) };
        // It points up at the waveform from the ruler below.
        let foot = h - 3.0;
        let knob = gsk::PathBuilder::new();
        knob.move_to(x - half, foot);
        knob.line_to(x + half, foot);
        knob.line_to(x + half, foot - tall + half);
        knob.line_to(x, foot - tall);
        knob.line_to(x - half, foot - tall + half);
        knob.close();
        let knob = knob.to_path();
        let round = gsk::Stroke::new(2.0);
        round.set_line_join(gsk::LineJoin::Round);
        s.append_stroke(&knob, &round, &rgba(knob_color, 1.0));
        s.append_fill(&knob, gsk::FillRule::Winding, &rgba(knob_color, 1.0));
        s.append_stroke(&knob, &gsk::Stroke::new(1.0), &rgba((0.0, 0.0, 0.0), 0.25));
        s.pop();
    }

    /// The time ruler under the waveform: the cut tinted, ticks hanging down from a line, and a
    /// label on every big tick.
    fn draw_ruler(&self, widget: &gtk::Widget, s: &gtk::Snapshot, w: f32, h: f32, ink: (f32, f32, f32)) {
        let (left, right) = (HANDLE, w - HANDLE);
        let base = h - RULER + 4.0;
        let (sx, ex) = (self.x(self.start.get(), w), self.x(self.end.get(), w));
        let corner = graphene::Size::new(5.0, 5.0);
        let none = graphene::Size::new(0.0, 0.0);
        let cut = gsk::RoundedRect::new(graphene::Rect::new(sx, base + 1.0, ex - sx, h - base - 1.0), none, none, corner, corner);
        s.push_rounded_clip(&cut);
        s.append_color(&rgba(ACCENT, 0.14), cut.bounds());
        s.pop();
        s.append_color(&rgba(ink, 0.18), &graphene::Rect::new(left, base, right - left, 1.0));

        let (from, span) = self.shown();
        let per_second = (right - left) / span;
        let (step, parts) = ruler_steps(per_second);
        let small = step / parts as f32;
        let long = self.length >= 60.0;
        let mut font = widget.pango_context().font_description().unwrap_or_default();
        font.set_size((font.size() as f32 * 0.78) as i32);
        font.set_weight(pango::Weight::Semibold);
        let (first, last) = ((from / small).ceil() as u32, ((from + span) / small).floor() as u32);
        for i in first..=last {
            let v = i as f32 * small;
            let x = self.x(v, w).round();
            let major = i % parts == 0;
            let (len, alpha) = if major { (8.0, 0.5) } else { (4.0, 0.25) };
            s.append_color(&rgba(ink, alpha), &graphene::Rect::new(x - 0.5, base + 1.0, 1.0, len));
            if !major {
                continue;
            }
            let layout = widget.create_pango_layout(Some(&ruler_label(v, step, long)));
            layout.set_font_description(Some(&font));
            let (_, logical) = layout.pixel_extents();
            // A label that would stick out on the right is left out.
            if x + 4.0 + logical.width() as f32 > right {
                continue;
            }
            s.save();
            s.translate(&graphene::Point::new(x + 4.0, base + 9.0 - logical.height() as f32 * 0.45));
            s.append_layout(&layout, &rgba(ink, 0.6));
            s.restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruler_labels() {
        assert_eq!(ruler_label(1.5, 0.5, false), "1.5 s");
        assert_eq!(ruler_label(2.0, 1.0, false), "2 s");
        assert_eq!(ruler_label(0.25, 0.25, false), "0.25 s");
        assert_eq!(ruler_label(90.0, 30.0, true), "1:30");
        assert_eq!(ruler_label(61.5, 0.5, true), "1:01.5");
    }

    #[test]
    fn ruler_steps_leave_room_for_labels() {
        for per_second in [5.0, 40.0, 120.0, 900.0, 4000.0] {
            let (step, parts) = ruler_steps(per_second);
            assert!(step * per_second >= LABEL_GAP || step == 1800.0, "{per_second}");
            assert!(parts >= 3);
        }
    }
}
