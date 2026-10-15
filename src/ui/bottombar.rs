//! The bar at the bottom: mic, the three big switches and who is listening.

use adw::prelude::*;

use super::{popup, label};
use super::visuals::level_meter;
use crate::i18n::{fill, t};
use crate::state::{Change, St};

fn pill(icon: &str, text: &str, tip: &str, key: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.set_child(Some(&adw::ButtonContent::builder().icon_name(icon).label(text).build()));
    b.add_css_class("vn-pill");
    b.set_tooltip_text(Some(&super::with_key(tip, key)));
    b
}

/// The bar, and its texts that can go in a narrow window: the microphone name and who listens.
pub fn build(st: &St) -> (gtk::CenterBox, [gtk::Widget; 2]) {
    // A center box keeps the switches in the exact middle, whatever the sides are wide.
    let bar = gtk::CenterBox::new();
    bar.add_css_class("vn-bottombar");

    // Left: mute, level, device.
    let left = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    left.set_margin_end(12);
    let mute = gtk::Button::from_icon_name("audio-input-microphone-symbolic");
    mute.add_css_class("vn-mute");
    mute.set_valign(gtk::Align::Center);
    mute.set_tooltip_text(Some(&super::with_key(t("Mute microphone"), "M")));
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
    device.set_max_width_chars(20);
    meter_box.append(&meter);
    meter_box.append(&device);
    left.append(&mute);
    left.append(&meter_box);

    // Middle: the switches.
    let mid = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    mid.set_halign(gtk::Align::Center);
    let voice = pill("audio-input-microphone-symbolic", t("Voice changer"), t("Turn your voice changer on or off"), "T");
    let hear = pill("audio-headphones-symbolic", t("Hear myself"), t("Hear your changed voice in your headphones"), "H");
    let bg = pill("weather-few-clouds-symbolic", t("Background"), t("Play the background sounds of a voice"), "B");
    mid.append(&voice);
    mid.append(&hear);
    mid.append(&bg);

    // Right: is anyone using us?
    let right = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    right.set_margin_start(12);
    let live = gtk::MenuButton::new();
    live.add_css_class("flat");
    let live_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    live_box.add_css_class("vn-live");
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("vn-live-dot");
    dot.set_valign(gtk::Align::Center);
    let live_label = gtk::Label::new(None);
    live_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    live_label.set_max_width_chars(22);
    live_box.append(&dot);
    live_box.append(&live_label);
    live.set_child(Some(&live_box));
    live.set_popover(Some(&help_popover()));
    right.append(&live);

    bar.set_start_widget(Some(&left));
    bar.set_center_widget(Some(&mid));
    bar.set_end_widget(Some(&right));

    let sync = {
        let st = st.clone();
        let (voice, hear, bg, mute, device, live, live_box, live_label) = (
            voice.clone(),
            hear.clone(),
            bg.clone(),
            mute.clone(),
            device.clone(),
            live.clone(),
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
                mute.set_icon_name("microphone-sensitivity-muted-symbolic");
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
            // For narrow windows, where only the dot is left.
            live.set_tooltip_text(Some(&live_label.label()));
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
    (bar, [device.upcast(), live_label.upcast()])
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
    popup::menu(&p);
    p.set_child(Some(&b));
    p
}
