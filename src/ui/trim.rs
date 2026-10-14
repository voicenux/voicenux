//! Cuts the start and end of a sound, like a video clip: its waveform with two handles.
//! Listening plays the cut sound in the headphones only.

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

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Start,
    End,
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
    start_field: gtk::EditableLabel,
    end_field: gtk::EditableLabel,
    cut_length: gtk::Label,
    play: gtk::Button,
    /// When listening started, while it plays.
    playing: Cell<Option<Instant>>,
    playhead: Cell<f32>,
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

    let wave = gtk::DrawingArea::builder().content_height(150).hexpand(true).build();
    wave.add_css_class("vn-trim-wave");
    wave.set_cursor_from_name(Some("ew-resize"));
    let play = gtk::Button::new();
    play.add_css_class("vn-gradient-button");
    play.set_halign(gtk::Align::Center);

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
        f.add_css_class("vn-trim-time");
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
        start_field: start_field.clone(),
        end_field: end_field.clone(),
        cut_length: label("", &["vn-trim-length"]),
        play: play.clone(),
        playing: Cell::new(None),
        playhead: Cell::new(0.0),
    });
    let _ = editor.set(Rc::downgrade(&e));
    if e.end.get() <= e.start.get() {
        e.end.set(length);
    }

    wave.set_draw_func({
        let e = Rc::downgrade(&e);
        move |area, cr, w, h| {
            if let Some(e) = e.upgrade() {
                e.draw(area, cr, w as f64, h as f64);
            }
        }
    });
    e.drag();
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

    let hint = label(
        t("Drag the handles to cut the start and the end. Listening plays in your headphones only."),
        &["dim-label", "caption"],
    );
    hint.set_wrap(true);
    hint.set_justify(gtk::Justification::Center);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
    body.set_margin_top(12);
    body.set_margin_bottom(20);
    body.set_margin_start(20);
    body.set_margin_end(20);
    body.append(&wave);
    body.append(&times);
    body.append(&buttons);
    body.append(&hint);

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
    dialog.set_default_widget(Some(&save));

    cancel.connect_clicked({
        let dialog = dialog.downgrade();
        move |_| {
            if let Some(d) = dialog.upgrade() {
                d.close();
            }
        }
    });
    save.connect_clicked({
        let (e, dialog) = (Rc::downgrade(&e), dialog.downgrade());
        move |_| {
            if let (Some(e), Some(d)) = (e.upgrade(), dialog.upgrade()) {
                e.save();
                d.close();
            }
        }
    });
    // Space listens, like in video editors.
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let e = Rc::downgrade(&e);
        move |_, key, _, _| {
            if key == gdk::Key::space
                && let Some(e) = e.upgrade()
            {
                e.toggle_play();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
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
    dialog.present(Some(parent));
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
        self.stop();
        self.update();
    }

    fn update(&self) {
        self.start_field.set_text(&seconds(self.start.get()));
        self.end_field.set_text(&seconds(self.end.get()));
        self.cut_length.set_label(&seconds(self.end.get() - self.start.get()));
        self.wave.queue_draw();
        self.show_play();
    }

    fn show_play(&self) {
        let (icon, text) = if self.playing.get().is_some() {
            ("media-playback-stop-symbolic", t("Stop"))
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

    fn drag(self: &Rc<Self>) {
        let drag = gtk::GestureDrag::new();
        // The side being moved, and how far the pointer is from its edge.
        let grab: Rc<Cell<Option<(Side, f64)>>> = Rc::default();
        drag.connect_drag_begin({
            let (e, grab) = (Rc::downgrade(self), grab.clone());
            move |g, x, _| {
                let Some(e) = e.upgrade() else { return };
                let w = e.wave.width() as f64;
                let (sx, ex) = (e.x(e.start.get(), w), e.x(e.end.get(), w));
                // The middles of the handles: they sit just outside the cut.
                let side = if (x - (sx - HANDLE / 2.0)).abs() <= (x - (ex + HANDLE / 2.0)).abs() {
                    Side::Start
                } else {
                    Side::End
                };
                let edge = if side == Side::Start { sx } else { ex };
                // On a handle, keep where it was grabbed. Anywhere else, the handle jumps there.
                let offset = if (x - edge).abs() <= HANDLE * 1.5 { edge - x } else { 0.0 };
                grab.set(Some((side, offset)));
                g.set_state(gtk::EventSequenceState::Claimed);
                e.set(side, e.t(x + offset, w));
            }
        });
        drag.connect_drag_update({
            let (e, grab) = (Rc::downgrade(self), grab.clone());
            move |g, dx, _| {
                let (Some(e), Some((side, offset)), Some((x, _))) = (e.upgrade(), grab.get(), g.start_point()) else {
                    return;
                };
                e.set(side, e.t(x + dx + offset, e.wave.width() as f64));
            }
        });
        drag.connect_drag_end(move |_, _, _| grab.set(None));
        self.wave.add_controller(drag);
    }

    fn toggle_play(&self) {
        if self.playing.get().is_some() {
            self.stop();
            self.show_play();
            return;
        }
        let cut = soundboard::trim(&self.data, self.start.get(), Some(self.end.get()));
        self.st.preview(Arc::new(cut));
        self.playing.set(Some(Instant::now()));
        self.playhead.set(0.0);
        self.show_play();
    }

    fn stop(&self) {
        if self.playing.take().is_some() {
            self.st.stop_preview();
        }
    }

    /// Moves the playhead, and notices when the sound is over.
    fn follow(&self) {
        let Some(started) = self.playing.get() else { return };
        match self.st.preview_progress() {
            Some(p) => self.playhead.set(p),
            // The engine needs a moment to start it.
            None if started.elapsed() > Duration::from_millis(250) => {
                self.playing.set(None);
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
        let (sx, ex) = (self.x(self.start.get(), w), self.x(self.end.get(), w));
        let (left, right) = (HANDLE, w - HANDLE);

        // Ground.
        rounded(cr, left, 0.0, right - left, h, 10.0);
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.05);
        let _ = cr.fill();

        // Waveform: a bar every 4 pixels, colored inside the cut and grey outside.
        let grad = cairo::LinearGradient::new(sx, 0.0, ex, 0.0);
        grad.add_color_stop_rgb(0.0, 0.15, 0.39, 0.92);
        grad.add_color_stop_rgb(0.5, 0.23, 0.51, 0.96);
        grad.add_color_stop_rgb(1.0, 0.13, 0.83, 0.93);
        let columns = ((right - left) / 4.0).max(1.0) as usize;
        let mid = h / 2.0;
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
                cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.22);
            }
            let _ = cr.stroke();
        }

        // The frame of the cut, with a handle on each side.
        let accent = (0.23, 0.51, 0.96);
        cr.set_source_rgb(accent.0, accent.1, accent.2);
        cr.set_line_width(3.0);
        cr.rectangle(sx, 1.5, ex - sx, h - 3.0);
        let _ = cr.stroke();
        for x in [sx - HANDLE, ex] {
            rounded(cr, x, 0.0, HANDLE, h, 6.0);
            cr.set_source_rgb(accent.0, accent.1, accent.2);
            let _ = cr.fill();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
            cr.set_line_width(2.0);
            for dx in [-2.0, 2.0] {
                cr.move_to(x + HANDLE / 2.0 + dx, mid - 10.0);
                cr.line_to(x + HANDLE / 2.0 + dx, mid + 10.0);
            }
            let _ = cr.stroke();
        }

        // Playhead.
        if self.playing.get().is_some() {
            let x = sx + self.playhead.get() as f64 * (ex - sx);
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(2.0);
            cr.move_to(x, 4.0);
            cr.line_to(x, h - 4.0);
            let _ = cr.stroke();
            cr.arc(x, 6.0, 4.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
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
