//! The bar at the bottom: mic, the three big switches and who is listening.

use adw::prelude::*;

use super::label;
use super::visuals::level_meter;
use crate::i18n::{fill, t};
use crate::state::{Change, St};

fn pill(icon: &str, text: &str, tip: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.set_child(Some(&adw::ButtonContent::builder().icon_name(icon).label(text).build()));
    b.add_css_class("vn-pill");
    b.set_tooltip_text(Some(tip));
    b
}

pub fn build(st: &St) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bar.add_css_class("vn-bottombar");

    // Left: mute, level, device.
    let left = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    left.set_hexpand(true);
    left.set_halign(gtk::Align::Start);
    let mute = gtk::Button::from_icon_name("audio-input-microphone-symbolic");
    mute.add_css_class("vn-mute");
    mute.set_valign(gtk::Align::Center);
    mute.set_tooltip_text(Some(t("Mute microphone")));
    mute.connect_clicked({
        let st = st.clone();
        move |_| st.set_muted(!st.muted.get())
    });
    let meter_box = gtk::Box::new(gtk::Orientation::Vertical, 3);
    meter_box.set_valign(gtk::Align::Center);
    let meter = level_meter(st, |l| l.input, 18);
    let device = label("", &["dim-label", "vn-device"]);
    device.set_xalign(0.0);
    device.set_ellipsize(gtk::pango::EllipsizeMode::End);
    device.set_max_width_chars(26);
    meter_box.append(&meter);
    meter_box.append(&device);
    left.append(&mute);
    left.append(&meter_box);

    // Middle: the switches.
    let mid = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    mid.set_halign(gtk::Align::Center);
    let voice = pill("audio-input-microphone-symbolic", t("Voice changer"), t("Turn your voice changer on or off"));
    let hear = pill("audio-headphones-symbolic", t("Hear myself"), t("Hear your changed voice in your headphones"));
    let bg = pill("weather-few-clouds-symbolic", t("Background"), t("Play the background sounds of a voice"));
    mid.append(&voice);
    mid.append(&hear);
    mid.append(&bg);

    // Right: is anyone using us?
    let right = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    right.set_hexpand(true);
    right.set_halign(gtk::Align::End);
    let live = gtk::MenuButton::new();
    live.add_css_class("flat");
    let live_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    live_box.add_css_class("vn-live");
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("vn-live-dot");
    dot.set_valign(gtk::Align::Center);
    let live_label = gtk::Label::new(None);
    live_box.append(&dot);
    live_box.append(&live_label);
    live.set_child(Some(&live_box));
    live.set_popover(Some(&help_popover()));
    right.append(&live);

    bar.append(&left);
    bar.append(&mid);
    bar.append(&right);

    let sync = {
        let st = st.clone();
        let (voice, hear, bg, mute, device, live_box, live_label) = (
            voice.clone(),
            hear.clone(),
            bg.clone(),
            mute.clone(),
            device.clone(),
            live_box.clone(),
            live_label.clone(),
        );
        move || {
            let c = st.config.borrow().clone();
            voice.set_active(c.voice_on);
            hear.set_active(c.hear_myself);
            bg.set_active(c.background);
            if st.muted.get() {
                mute.add_css_class("muted");
                mute.set_icon_name("microphone-disabled-symbolic");
            } else {
                mute.remove_css_class("muted");
                mute.set_icon_name("audio-input-microphone-symbolic");
            }
            let dev = st.active_input.borrow().clone();
            device.set_label(&match dev {
                Some(d) => d.description,
                None if st.error.borrow().is_some() => t("Audio engine stopped").to_string(),
                None => t("No microphone found").to_string(),
            });
            let users = st.users.borrow().clone();
            if users.is_empty() {
                live_box.remove_css_class("on");
                live_label.set_label(t("Not used by any app"));
            } else {
                live_box.add_css_class("on");
                live_label.set_label(&fill(t("Live in {}"), &[&users.join(", ")]));
            }
        }
    };
    sync();

    voice.connect_toggled({
        let st = st.clone();
        move |b| {
            if b.is_active() != st.config.borrow().voice_on {
                st.set_voice_on(b.is_active());
            }
        }
    });
    hear.connect_toggled({
        let st = st.clone();
        move |b| {
            if b.is_active() != st.config.borrow().hear_myself {
                st.set_hear_myself(b.is_active());
            }
        }
    });
    bg.connect_toggled({
        let st = st.clone();
        move |b| {
            if b.is_active() != st.config.borrow().background {
                st.set_background(b.is_active());
            }
        }
    });

    st.subscribe(move |c| {
        if matches!(c, Change::Toggles | Change::Devices | Change::Users | Change::Engine(_) | Change::Settings) {
            sync();
        }
    });
    bar
}

fn help_popover() -> gtk::Popover {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.set_margin_top(12);
    b.set_margin_bottom(12);
    b.set_margin_start(12);
    b.set_margin_end(12);
    b.set_width_request(300);
    let title = label(t("Use your new voice in apps"), &["heading"]);
    title.set_xalign(0.0);
    b.append(&title);
    for text in [
        t("In Discord, OBS, games or your browser, pick “Voicenux Microphone” as the microphone."),
        t("Or turn on “Use as default microphone” in Settings. Then every app gets your changed voice."),
        t("Wear headphones when you use Hear myself, or your speakers feed back into the mic."),
    ] {
        let l = label(text, &[]);
        l.set_wrap(true);
        l.set_xalign(0.0);
        l.set_max_width_chars(40);
        b.append(&l);
    }
    let p = gtk::Popover::new();
    p.set_child(Some(&b));
    p
}
