//! The one click for AI voices: a dialog that says what the voice needs, downloads it with its
//! progress and turns the voice on. The engine comes along the first time.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::label;
use crate::ai::{self, install};
use crate::i18n::{fill, t, t_dyn};
use crate::state::{Change, St};

thread_local! {
    /// One download at a time.
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

enum Step {
    Engine(usize),
    Voice(f32),
    Done,
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
        return t("AI voices need an NVIDIA graphics card, or Run on the processor in Settings.").into();
    }
    if !ai::ready(model) {
        return t("Not downloaded yet. Pick the voice to get it.").into();
    }
    match ai::bridge::status() {
        Status::Starting => t("Starting the AI…").into(),
        Status::Loading(name) => fill(t("Loading {}…"), &[&t_dyn(name)]),
        Status::Running if ai::on_cpu() => t("The AI runs on your processor. It adds up to a second of delay.").into(),
        Status::Running => t("The AI runs on your graphics card. It adds about 0.3 s of delay.").into(),
        Status::Failed(e) => fill(t("The AI did not work: {}"), &[&e]),
        Status::Off | Status::Missing => String::new(),
    }
}

/// Asks to download what the AI voice `id` needs, then turns it on.
pub fn offer(st: &St, parent: &impl IsA<gtk::Widget>, id: &str) {
    if BUSY.get() {
        return;
    }
    let Some(v) = st.voice(id) else { return };
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
    let size = if engine {
        t("The voice is about 60 MB.").to_string()
    } else {
        fill(
            t("The first AI voice also needs the AI engine: about {} GB, once. Each voice is about 60 MB more."),
            &[&install::engine_gb().to_string()],
        )
    };
    let runs = if ai::on_cpu() {
        t("It runs on your processor and adds up to a second of delay.")
    } else {
        t("It runs on your NVIDIA graphics card and adds about 0.3 s of delay.")
    };
    let text = label(&format!("{size} {runs}"), &["dim-label"]);
    text.set_wrap(true);
    text.set_justify(gtk::Justification::Center);
    text.set_max_width_chars(44);
    let bar = gtk::ProgressBar::new();
    bar.set_visible(false);
    let what = label("", &["dim-label", "caption"]);
    what.set_visible(false);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let cancel = gtk::Button::with_label(t("Cancel"));
    cancel.add_css_class("pill");
    let go = gtk::Button::with_label(t("Download"));
    go.add_css_class("vn-gradient-button");
    buttons.append(&cancel);
    buttons.append(&go);
    for w in
        [pic.upcast_ref::<gtk::Widget>(), title.upcast_ref(), text.upcast_ref(), bar.upcast_ref(), what.upcast_ref()]
    {
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
    cancel.connect_clicked({
        let dialog = dialog.clone();
        move |_| {
            dialog.close();
        }
    });

    let started = Rc::new(Cell::new(false));
    go.connect_clicked({
        let (st, id, dialog) = (st.clone(), id.to_string(), dialog.clone());
        let (bar, what, text, go, cancel) = (bar.clone(), what.clone(), text.clone(), go.clone(), cancel.clone());
        move |_| {
            if started.replace(true) {
                return;
            }
            BUSY.set(true);
            // A dialog that would lose its download stays open until it is done.
            dialog.set_can_close(false);
            go.set_sensitive(false);
            cancel.set_sensitive(false);
            bar.set_visible(true);
            what.set_visible(true);
            bar.set_fraction(0.0);
            let (tx, rx) = async_channel::unbounded();
            std::thread::spawn(move || {
                let send = |s: Step| {
                    let _ = tx.send_blocking(s);
                };
                let result = (|| {
                    if !install::installed() {
                        install::install_engine(&|n| send(Step::Engine(n)))?;
                    }
                    if !model.downloaded() {
                        install::download_model(model, &|f| send(Step::Voice(f)))?;
                    }
                    anyhow::Ok(())
                })();
                send(match result {
                    Ok(()) => Step::Done,
                    Err(e) => {
                        log::warn!("AI download: {e:#}");
                        Step::Failed(e.to_string())
                    }
                });
            });
            let (st, id, dialog, bar, what, text, go, cancel) = (
                st.clone(),
                id.clone(),
                dialog.clone(),
                bar.clone(),
                what.clone(),
                text.clone(),
                go.clone(),
                cancel.clone(),
            );
            let started = started.clone();
            glib::spawn_future_local(async move {
                // The engine takes most of the bar, the voice the rest.
                let share = if engine { 0.0 } else { 0.9 };
                while let Ok(step) = rx.recv().await {
                    match step {
                        Step::Engine(n) => {
                            what.set_label(step_text(n));
                            bar.set_fraction(share * (n - 1) as f64 / install::STEPS as f64);
                        }
                        Step::Voice(f) => {
                            what.set_label(&fill(t("Getting {}…"), &[&t_dyn(model.name)]));
                            bar.set_fraction(share + (1.0 - share) * f as f64);
                        }
                        Step::Done => {
                            BUSY.set(false);
                            dialog.set_can_close(true);
                            dialog.close();
                            st.notify(Change::Voices);
                            st.set_voice(&id);
                            return;
                        }
                        Step::Failed(e) => {
                            BUSY.set(false);
                            started.set(false);
                            dialog.set_can_close(true);
                            bar.set_visible(false);
                            what.set_label(&fill(t("That did not work: {}"), &[&e]));
                            text.set_label(t("Check your connection and the free space, then try again."));
                            go.set_label(t("Try again"));
                            go.set_sensitive(true);
                            cancel.set_sensitive(true);
                            return;
                        }
                    }
                }
            });
        }
    });
    dialog.present(Some(parent));
}
