//! First start: four short pages that get the microphone and apps set up.

use std::rc::Rc;

use adw::prelude::*;

use super::visuals::level_meter;
use super::{icon, label, logo};
use crate::i18n::t;
use crate::state::St;

fn page(pic: gtk::Widget, title: &str, text: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
    b.set_valign(gtk::Align::Center);
    b.set_hexpand(true);
    b.set_margin_start(36);
    b.set_margin_end(36);
    pic.set_halign(gtk::Align::Center);
    pic.set_margin_bottom(6);
    b.append(&pic);
    let h = label(title, &["vn-hero-title"]);
    h.set_wrap(true);
    h.set_justify(gtk::Justification::Center);
    b.append(&h);
    let p = label(text, &["dim-label"]);
    p.set_wrap(true);
    p.set_justify(gtk::Justification::Center);
    p.set_max_width_chars(46);
    b.append(&p);
    b
}

pub fn show(st: &St, parent: &adw::ApplicationWindow) {
    let carousel = adw::Carousel::new();
    carousel.set_vexpand(true);
    carousel.set_allow_scroll_wheel(false);

    carousel.append(&page(
        logo(112).upcast(),
        t("Welcome to Voicenux"),
        t("Change your voice in real time: in Discord, games, streams and calls. Here is how to get going, it takes a minute."),
    ));

    let mic = page(
        icon("studio-microphone", 96).upcast(),
        t("Your microphone"),
        t("Say something. The bar should move. If not, pick another microphone."),
    );
    let devices = gtk::DropDown::from_strings(&[]);
    let model = gtk::StringList::new(&[]);
    let names: Vec<Option<String>> =
        std::iter::once(None).chain(st.inputs.borrow().iter().map(|d| Some(d.name.clone()))).collect();
    model.append(t("System default"));
    for d in st.inputs.borrow().iter() {
        model.append(&d.description);
    }
    devices.set_model(Some(&model));
    let chosen = st.config.borrow().input.clone();
    devices.set_selected(names.iter().position(|n| *n == chosen).unwrap_or(0) as u32);
    devices.set_halign(gtk::Align::Center);
    devices.set_margin_top(10);
    devices.connect_selected_notify({
        let st = st.clone();
        move |d| {
            let pick = names.get(d.selected() as usize).cloned().flatten();
            st.update(|c| c.input = pick);
        }
    });
    mic.append(&devices);
    let meter = level_meter(st, |l| l.input, 24);
    meter.set_halign(gtk::Align::Center);
    meter.set_margin_top(8);
    mic.append(&meter);
    carousel.append(&mic);

    let apps = page(
        icon("headphone", 96).upcast(),
        t("Use it in your apps"),
        t("Apps hear your new voice through the “Voicenux Microphone”. Make it the default, or pick it in each app."),
    );
    let default = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    default.set_halign(gtk::Align::Center);
    default.set_margin_top(10);
    let sw = gtk::Switch::new();
    sw.set_active(true);
    default.append(&label(t("Use as default microphone"), &["heading"]));
    default.append(&sw);
    apps.append(&default);
    carousel.append(&apps);

    let try_it = page(
        icon("sparkles", 96).upcast(),
        t("Try a voice"),
        t(
            "Click any voice and turn on Hear myself to hear it. Wear headphones, or your speakers feed back into the microphone.",
        ),
    );
    carousel.append(&try_it);

    let dots = adw::CarouselIndicatorDots::new();
    dots.set_carousel(Some(&carousel));
    let back = gtk::Button::with_label(t("Back"));
    back.add_css_class("pill");
    let next = gtk::Button::with_label(t("Next"));
    next.add_css_class("vn-gradient-button");
    let bar = gtk::CenterBox::new();
    bar.set_start_widget(Some(&back));
    bar.set_center_widget(Some(&dots));
    bar.set_end_widget(Some(&next));
    bar.set_margin_start(24);
    bar.set_margin_end(24);
    bar.set_margin_bottom(20);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = adw::HeaderBar::new();
    header.set_show_title(false);
    body.append(&header);
    body.append(&carousel);
    body.append(&bar);

    let dialog = adw::Dialog::builder().content_width(560).content_height(560).child(&body).build();
    let finish = Rc::new({
        let st = st.clone();
        let dialog = dialog.clone();
        let sw = sw.clone();
        move || {
            let default = sw.is_active();
            st.update(|c| {
                c.onboarded = true;
                c.default_mic = default;
            });
            dialog.close();
        }
    });
    let update = {
        let carousel = carousel.clone();
        let back = back.clone();
        let next = next.clone();
        move || {
            let pos = carousel.position().round() as u32;
            back.set_opacity(if pos == 0 { 0.0 } else { 1.0 });
            back.set_sensitive(pos > 0);
            next.set_label(if pos + 1 >= carousel.n_pages() { t("Let’s go") } else { t("Next") });
        }
    };
    update();
    carousel.connect_position_notify(move |_| update());
    back.connect_clicked({
        let carousel = carousel.clone();
        move |_| {
            let pos = carousel.position().round() as u32;
            if pos > 0 {
                carousel.scroll_to(&carousel.nth_page(pos - 1), true);
            }
        }
    });
    next.connect_clicked({
        let carousel = carousel.clone();
        let finish = finish.clone();
        move |_| {
            let pos = carousel.position().round() as u32;
            if pos + 1 >= carousel.n_pages() {
                finish();
            } else {
                carousel.scroll_to(&carousel.nth_page(pos + 1), true);
            }
        }
    });
    dialog.connect_closed({
        let st = st.clone();
        move |_| {
            if !st.config.borrow().onboarded {
                st.update(|c| c.onboarded = true);
            }
        }
    });
    dialog.present(Some(parent));
}
