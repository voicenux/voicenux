//! The one click for AI voices: a dialog that says what the voice needs, downloads it with its
//! progress and turns the voice on. The engine comes along the first time. The download goes on
//! when the dialog closes: then a small pill in the bottom bar shows it.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;

use super::label;
use crate::ai::{self, install};
use crate::i18n::{fill, t, t_dyn};
use crate::state::{Change, St};

thread_local! {
    /// The download that runs. One at a time.
    static RUNNING: RefCell<Option<Rc<Download>>> = const { RefCell::new(None) };
    /// What shows a download besides its dialog: the pill in the bottom bar.
    static PILLS: RefCell<Vec<Box<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
}

/// A download of an AI voice, with the engine for the first one.
struct Download {
    id: String,
    name: String,
    icon: String,
    fraction: Cell<f64>,
    what: RefCell<String>,
    detail: RefCell<String>,
    /// The dialog, while it is open.
    view: RefCell<Option<View>>,
    /// Stop was pressed, and the parts are being cleared away.
    stopping: Cell<bool>,
}

/// How a download ends.
enum End {
    Done,
    Stopped,
    Failed(String),
}

/// How an open dialog shows a download, and what it does when the download ends.
struct View {
    show: Box<dyn Fn(&Download)>,
    end: Box<dyn Fn(&End)>,
}

impl Download {
    fn changed(&self) {
        if let Some(view) = self.view.borrow().as_ref() {
            (view.show)(self);
        }
        refresh_pills();
    }
}

fn running() -> Option<Rc<Download>> {
    RUNNING.with(|r| r.borrow().clone())
}

fn refresh_pills() {
    PILLS.with(|p| p.borrow().iter().for_each(|f| f()));
}

enum Step {
    /// A step of the engine, counted from 1, with its bytes done and in all.
    Engine(usize, u64, u64),
    Voice(f32),
    Done,
    Stopped,
    Failed(String),
}

fn step_text(n: usize) -> &'static str {
    match n {
        1 => t("Getting the installer…"),
        2 => t("Getting Python…"),
        3 => t("Getting PyTorch. This is the big part…"),
        4 => t("Getting the voice converter…"),
        _ => t("Getting the speech models…"),
    }
}

/// What the AI does right now, for the voice panel. Empty when there is nothing to say.
pub fn status_text(model: &ai::Model) -> String {
    use ai::bridge::Status;
    if !ai::usable() {
        return t("AI voices need an NVIDIA or AMD graphics card, or Run on the processor in Settings.").into();
    }
    if !ai::ready(model) {
        return t("Not downloaded yet. Pick the voice to get it.").into();
    }
    match ai::bridge::status() {
        Status::Starting => t("Starting the AI…").into(),
        Status::Loading(name) => fill(t("Loading {}…"), &[&t_dyn(name)]),
        Status::Running(ms) if ai::on_cpu() => {
            fill(t("The AI runs on your processor. It adds about {} of delay."), &[&seconds(ms)])
        }
        Status::Running(ms) => {
            fill(t("The AI runs on your graphics card. It adds about {} of delay."), &[&seconds(ms)])
        }
        Status::Failed(e) => fill(t("The AI did not work: {}"), &[&e]),
        Status::Off | Status::Missing => String::new(),
    }
}

/// Turns bytes done into "37 % · 1.2 GB of 3.4 GB · about 2 min left". The time left comes from
/// the speed of the last seconds.
#[derive(Default)]
struct Meter {
    seen: VecDeque<(Instant, u64)>,
}

impl Meter {
    fn text(&mut self, done: u64, all: u64) -> String {
        let now = Instant::now();
        self.seen.push_back((now, done));
        while self.seen.len() > 2 && now - self.seen[0].0 > Duration::from_secs(10) {
            self.seen.pop_front();
        }
        // Never 100 % before it is really done.
        let percent = (done as f64 * 100.0 / all.max(1) as f64).floor().min(99.0);
        let mut parts = vec![format!("{percent} %"), fill(t("{} of {}"), &[&size(done), &size(all)])];
        let (then, from) = self.seen[0];
        let (secs, got) = ((now - then).as_secs_f64(), done.saturating_sub(from));
        if secs >= 2.0 && got > 0 {
            parts.push(time_left(all.saturating_sub(done) as f64 * secs / got as f64));
        }
        parts.join("  ·  ")
    }
}

/// "850 MB" or "3.4 GB", with a comma where the language writes one.
fn size(bytes: u64) -> String {
    let text = if bytes < 1_000_000_000 {
        format!("{} MB", bytes / 1_000_000)
    } else {
        format!("{:.1} GB", bytes as f64 / 1e9)
    };
    crate::i18n::decimals(text)
}

/// "about 40 s left" or "about 3 min left", rounded up.
fn time_left(secs: f64) -> String {
    if secs < 60.0 {
        fill(t("about {} s left"), &[&((secs / 5.0).ceil().max(1.0) as u32 * 5).to_string()])
    } else {
        fill(t("about {} min left"), &[&((secs / 60.0).ceil() as u32).to_string()])
    }
}

/// "0.2 s", with a comma where the language writes one.
fn seconds(ms: u32) -> String {
    crate::i18n::decimals(format!("{:.1} s", ms as f32 / 1000.0))
}

/// Asks to download what the AI voice `id` needs, then turns it on. While a download runs, it
/// shows that one: one at a time.
pub fn offer(st: &St, parent: &impl IsA<gtk::Widget>, id: &str) {
    let busy = running();
    // It is going away already.
    if busy.as_ref().is_some_and(|d| d.stopping.get()) {
        return;
    }
    let id = busy.as_ref().map_or_else(|| id.to_string(), |d| d.id.clone());
    let Some(v) = st.voice(&id) else { return };
    let Some(model) = ai::model_of(&v.block_values()) else { return };
    let name = t_dyn(&v.name);
    let engine = install::installed();

    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_top(10);
    body.set_margin_bottom(22);
    body.set_margin_start(24);
    body.set_margin_end(24);
    let pic = super::icon(&v.icon, 72);
    let title = label(&fill(t("Download {}?"), &[&name]), &["title-3"]);
    title.set_wrap(true);
    // What it takes, as a short list. A space that does not break keeps a size on one line.
    let mut facts: Vec<(&str, &str, String)> = Vec::new();
    if !engine {
        let gb = format!("{}\u{a0}GB", install::engine_gb());
        facts.push(("package", t("AI engine"), fill(t("{}, once"), &[&gb])));
    }
    facts.push(("mic-vocal", t("Voice"), format!("{}\u{a0}MB", model.mb)));
    let (on, delay) = match ai::card() {
        _ if ai::on_cpu() => (t("Processor"), t("about half a second")),
        Some(ai::Card::Amd(_)) => (t("AMD graphics card"), t("about 0.3 s")),
        _ => (t("NVIDIA graphics card"), t("about 0.3 s")),
    };
    facts.push(("cpu", t("Runs on"), on.into()));
    facts.push(("timer", t("Delay"), delay.into()));
    let list = gtk::Grid::builder().column_spacing(12).row_spacing(10).halign(gtk::Align::Center).build();
    list.add_css_class("vn-facts");
    list.set_size_request(320, -1);
    for (row, (icon, key, value)) in facts.iter().enumerate() {
        let row = row as i32;
        let pic = super::line_icon(icon, 16);
        pic.add_css_class("dim-label");
        let key = label(key, &["dim-label"]);
        key.set_xalign(0.0);
        let value = label(value, &["vn-fact-value"]);
        value.set_xalign(1.0);
        value.set_hexpand(true);
        list.attach(&pic, 0, row, 1, 1);
        list.attach(&key, 1, row, 1, 1);
        list.attach(&value, 2, row, 1, 1);
    }
    // Only when it did not work.
    let hint = label(t("Check your connection and the free space, then try again."), &["dim-label"]);
    hint.set_wrap(true);
    hint.set_justify(gtk::Justification::Center);
    hint.set_max_width_chars(44);
    hint.set_visible(false);
    let bar = gtk::ProgressBar::new();
    bar.set_visible(false);
    let what = label("", &["dim-label", "caption"]);
    what.set_visible(false);
    // How far, as a number: per cent, size and the time left.
    let detail = label("", &["dim-label", "caption", "numeric"]);
    detail.set_visible(false);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let cancel = gtk::Button::with_label(t("Cancel"));
    cancel.add_css_class("pill");
    // While it downloads: stop it, or let the dialog go and the download go on.
    let stop = gtk::Button::with_label(t("Stop download"));
    stop.add_css_class("pill");
    stop.set_visible(false);
    let hide = gtk::Button::with_label(t("In the background"));
    hide.add_css_class("pill");
    hide.set_visible(false);
    let go = gtk::Button::with_label(t("Download"));
    go.add_css_class("vn-gradient-button");
    buttons.append(&cancel);
    buttons.append(&stop);
    buttons.append(&hide);
    buttons.append(&go);
    for w in [
        pic.upcast_ref::<gtk::Widget>(),
        title.upcast_ref(),
        list.upcast_ref(),
        hint.upcast_ref(),
        bar.upcast_ref(),
        what.upcast_ref(),
        detail.upcast_ref(),
    ] {
        body.append(w);
    }
    body.append(&buttons);

    let header = adw::HeaderBar::new();
    header.set_show_title(false);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);
    root.append(&body);
    let dialog = adw::Dialog::builder().content_width(460).child(&root).build();
    dialog.set_default_widget(Some(&go));
    dialog.set_focus(Some(&go));
    for b in [&cancel, &hide] {
        b.connect_clicked({
            let dialog = dialog.downgrade();
            move |_| {
                if let Some(d) = dialog.upgrade() {
                    d.close();
                }
            }
        });
    }
    // Closing only hides a download: it goes on, and the bottom bar shows it.
    dialog.connect_closed(|_| {
        if let Some(d) = running() {
            d.view.replace(None);
            refresh_pills();
        }
    });

    // Shows a download in the dialog, and what to do when it ends while the dialog is open.
    let watch = {
        let (st, id) = (st.clone(), id.clone());
        let (dialog, bar, what, detail, hint) =
            (dialog.downgrade(), bar.clone(), what.clone(), detail.clone(), hint.clone());
        let (go, cancel, stop, hide) = (go.clone(), cancel.clone(), stop.clone(), hide.clone());
        move |d: &Rc<Download>| {
            hint.set_visible(false);
            for w in [
                bar.upcast_ref::<gtk::Widget>(),
                what.upcast_ref(),
                detail.upcast_ref(),
                stop.upcast_ref(),
                hide.upcast_ref(),
            ] {
                w.set_visible(true);
            }
            go.set_visible(false);
            cancel.set_visible(false);
            hide.grab_focus();
            let show = {
                let (bar, what, detail) = (bar.clone(), what.clone(), detail.clone());
                Box::new(move |d: &Download| {
                    bar.set_fraction(d.fraction.get());
                    what.set_label(&d.what.borrow());
                    detail.set_label(&d.detail.borrow());
                })
            };
            let end = {
                let (st, id, dialog) = (st.clone(), id.clone(), dialog.clone());
                let (bar, what, detail, hint) = (bar.clone(), what.clone(), detail.clone(), hint.clone());
                let (go, cancel, stop, hide) = (go.clone(), cancel.clone(), stop.clone(), hide.clone());
                Box::new(move |end: &End| match end {
                    End::Done => {
                        if let Some(dialog) = dialog.upgrade() {
                            dialog.close();
                        }
                        st.set_voice(&id);
                    }
                    End::Stopped => {
                        if let Some(dialog) = dialog.upgrade() {
                            dialog.close();
                        }
                    }
                    End::Failed(e) => {
                        bar.set_visible(false);
                        detail.set_visible(false);
                        stop.set_visible(false);
                        hide.set_visible(false);
                        what.set_label(&fill(t("That did not work: {}"), &[e]));
                        hint.set_visible(true);
                        go.set_label(t("Try again"));
                        go.set_visible(true);
                        cancel.set_visible(true);
                        go.grab_focus();
                    }
                })
            };
            d.view.replace(Some(View { show, end }));
            d.changed();
        }
    };
    if let Some(d) = &busy {
        watch(d);
    }
    stop.connect_clicked({
        let (what, detail, stop, hide) = (what.clone(), detail.clone(), stop.downgrade(), hide.downgrade());
        move |_| {
            let Some(d) = running() else { return };
            install::stop();
            d.stopping.set(true);
            refresh_pills();
            what.set_label(t("Stopping…"));
            detail.set_visible(false);
            for b in [stop.upgrade(), hide.upgrade()].into_iter().flatten() {
                b.set_sensitive(false);
            }
        }
    });
    go.connect_clicked({
        let (st, id, icon, name) = (st.clone(), id.clone(), v.icon.clone(), name.clone());
        move |_| {
            if running().is_some() {
                return;
            }
            let d = Rc::new(Download {
                id: id.clone(),
                name: name.clone(),
                icon: icon.clone(),
                fraction: Cell::new(0.0),
                what: RefCell::default(),
                detail: RefCell::default(),
                view: RefCell::default(),
                stopping: Cell::new(false),
            });
            RUNNING.with(|r| r.replace(Some(d.clone())));
            watch(&d);
            run(&st, d, model, engine);
        }
    });
    dialog.present(Some(parent));
}

/// Downloads on a thread of its own and tells `d` how far it is.
fn run(st: &St, d: Rc<Download>, model: &'static ai::Model, engine: bool) {
    let (tx, rx) = async_channel::unbounded();
    install::begin();
    std::thread::spawn(move || {
        let send = |s: Step| {
            let _ = tx.send_blocking(s);
        };
        let result = (|| {
            if !install::installed() {
                install::install_engine(&|n, done, total| send(Step::Engine(n, done, total)))?;
            }
            if !model.downloaded() {
                install::download_model(model, &|f| send(Step::Voice(f)))?;
            }
            anyhow::Ok(())
        })();
        send(match result {
            Ok(()) => Step::Done,
            Err(e) if e.is::<install::Stopped>() => Step::Stopped,
            Err(e) => {
                log::warn!("AI download: {e:#}");
                Step::Failed(e.to_string())
            }
        });
    });
    let st = st.clone();
    glib::spawn_future_local(async move {
        // The bytes of each part: the steps of the engine when it is needed, then the voice. A
        // step that starts says its real size.
        let mut parts: Vec<u64> =
            if engine { Vec::new() } else { install::step_mb().iter().map(|mb| mb * 1_000_000).collect() };
        parts.push(model.mb as u64 * 1_000_000);
        let voice = parts.len() - 1;
        let mut meter = Meter::default();
        let mut show = {
            let d = d.clone();
            move |part: usize, done: u64, total: u64| {
                if total > 0 {
                    parts[part] = total;
                }
                let all: u64 = parts.iter().sum();
                let done = parts[..part].iter().sum::<u64>() + done.min(parts[part]);
                d.fraction.set(d.fraction.get().max(done as f64 / all as f64));
                d.detail.replace(meter.text(done, all));
                d.changed();
            }
        };
        let result = loop {
            match rx.recv().await {
                Ok(Step::Engine(n, done, total)) => {
                    d.what.replace(step_text(n).into());
                    show(n - 1, done, total);
                }
                Ok(Step::Voice(f)) => {
                    d.what.replace(fill(t("Getting {}…"), &[&d.name]));
                    show(voice, (f as f64 * model.mb as f64 * 1e6) as u64, 0);
                }
                Ok(Step::Done) => break End::Done,
                Ok(Step::Stopped) => break End::Stopped,
                Ok(Step::Failed(e)) => break End::Failed(e),
                Err(_) => break End::Failed(t("That did not work. Try again.").to_string()),
            }
        };
        RUNNING.with(|r| r.replace(None));
        refresh_pills();
        if matches!(result, End::Done) {
            st.notify(Change::Voices);
        }
        // An open dialog ends as it should. Without one, a note says how it went.
        let view = d.view.take();
        if let Some(view) = &view {
            (view.end)(&result);
        }
        match (&result, view.is_some()) {
            (End::Stopped, _) => st.notify(Change::Notice(fill(t("Stopped downloading {}."), &[&d.name]))),
            // Done in the background: the voice waits to be picked.
            (End::Done, false) => st.notify(Change::Notice(fill(t("{} is ready. Pick it to use it."), &[&d.name]))),
            (End::Failed(e), false) => {
                // A note has room for one line: the last one says most.
                let e = e.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default();
                st.notify(Change::Notice(fill(t("Could not get {}: {}"), &[&d.name, e])))
            }
            _ => {}
        }
    });
}

/// While a download runs without its dialog: a small button for the bottom bar with the voice,
/// how far it is and a thin bar. A click opens the dialog again.
pub fn download_pill(st: &St) -> gtk::Button {
    let pic = gtk::Image::new();
    pic.set_pixel_size(24);
    let name = label("", &["vn-download-name", "numeric"]);
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_max_width_chars(22);
    let bar = gtk::ProgressBar::new();
    bar.add_css_class("vn-download-bar");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_valign(gtk::Align::Center);
    text.append(&name);
    text.append(&bar);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.append(&pic);
    content.append(&text);
    let button = gtk::Button::builder().child(&content).css_classes(["flat", "vn-download-pill"]).build();
    button.set_valign(gtk::Align::Center);
    button.set_visible(false);
    let refresh = {
        let (button, pic, name, bar) = (button.downgrade(), pic.downgrade(), name.downgrade(), bar.downgrade());
        move || {
            let (Some(button), Some(pic), Some(name), Some(bar)) =
                (button.upgrade(), pic.upgrade(), name.upgrade(), bar.upgrade())
            else {
                return;
            };
            let d = running().filter(|d| d.view.borrow().is_none() && !d.stopping.get());
            button.set_visible(d.is_some());
            let Some(d) = d else { return };
            pic.set_paintable(Some(&super::texture(&d.icon)));
            name.set_label(&format!("{}  {} %", d.name, (d.fraction.get() * 100.0).floor().min(99.0)));
            bar.set_fraction(d.fraction.get());
            button.set_tooltip_text(Some(&format!("{}\n{}", d.what.borrow(), d.detail.borrow())));
        }
    };
    PILLS.with(|p| p.borrow_mut().push(Box::new(refresh)));
    button.connect_clicked({
        let st = st.clone();
        move |b| {
            if let Some(d) = running() {
                offer(&st, b, &d.id);
            }
        }
    });
    button
}
