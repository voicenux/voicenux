//! Settings page. Every option lives here, nobody needs to edit a file.

use std::rc::Rc;

use adw::prelude::*;

use super::label;
use crate::audio::Device;
use crate::i18n::{fill, t, t_dyn};
use crate::shortcuts::{self, ACTIONS};
use crate::state::{Change, St};

fn scale_row(
    title: &str,
    subtitle: &str,
    min: f64,
    max: f64,
    value: f64,
    fmt: fn(f64) -> String,
    on: impl Fn(f64) + 'static,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).subtitle(subtitle).build();
    let shown = label(&fmt(value), &["dim-label", "numeric"]);
    shown.set_width_chars(7);
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, (max - min) / 200.0);
    scale.set_value(value);
    scale.set_draw_value(false);
    scale.set_width_request(220);
    scale.set_valign(gtk::Align::Center);
    scale.connect_value_changed({
        let shown = shown.clone();
        move |s| {
            shown.set_label(&fmt(s.value()));
            on(s.value());
        }
    });
    row.add_suffix(&scale);
    row.add_suffix(&shown);
    row
}

fn db(v: f64) -> String {
    format!("{:+.0} dB", v)
}

fn pct(v: f64) -> String {
    format!("{:.0} %", v * 100.0)
}

/// A combo row for devices, with "System default" on top. Keeps itself up to date.
fn device_row(st: &St, title: &str, inputs: bool) -> adw::ComboRow {
    let row = adw::ComboRow::builder().title(title).build();
    let model = gtk::StringList::new(&[]);
    row.set_model(Some(&model));
    let names: Rc<std::cell::RefCell<Vec<Option<String>>>> = Rc::default();
    let syncing = Rc::new(std::cell::Cell::new(false));
    let sync = {
        let st = st.clone();
        let row = row.clone();
        let model = model.clone();
        let names = names.clone();
        let syncing = syncing.clone();
        move || {
            syncing.set(true);
            let list: Vec<Device> = if inputs { st.inputs.borrow().clone() } else { st.outputs.borrow().clone() };
            let active = if inputs { st.active_input.borrow().clone() } else { st.active_output.borrow().clone() };
            let chosen = if inputs { st.config.borrow().input.clone() } else { st.config.borrow().output.clone() };
            let mut n: Vec<Option<String>> = vec![None];
            let default_label = match &active {
                Some(d) if chosen.is_none() => fill(t("System default ({})"), &[&d.description]),
                _ => t("System default").to_string(),
            };
            let mut labels = vec![default_label];
            for d in &list {
                n.push(Some(d.name.clone()));
                labels.push(d.description.clone());
            }
            // A chosen device that is unplugged right now stays in the list.
            if let Some(c) = &chosen
                && !list.iter().any(|d| &d.name == c)
            {
                n.push(Some(c.clone()));
                labels.push(fill(t("{} (not connected)"), &[c]));
            }
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            model.splice(0, model.n_items(), &refs);
            let pos = n.iter().position(|x| *x == chosen).unwrap_or(0);
            row.set_selected(pos as u32);
            *names.borrow_mut() = n;
            syncing.set(false);
        }
    };
    sync();
    row.connect_selected_notify({
        let st = st.clone();
        let names = names.clone();
        let syncing = syncing.clone();
        move |r| {
            if syncing.get() {
                return;
            }
            let pick = names.borrow().get(r.selected() as usize).cloned().flatten();
            st.update(|c| if inputs { c.input = pick } else { c.output = pick });
        }
    });
    st.subscribe(move |c| {
        if matches!(c, Change::Devices) {
            sync();
        }
    });
    row
}

pub fn build(st: &St, window: &adw::ApplicationWindow) -> adw::ToolbarView {
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(t("Settings"), "")));
    let page = adw::PreferencesPage::new();
    let c = st.config.borrow().clone();

    // Microphone.
    let mic = adw::PreferencesGroup::builder()
        .title(t("Microphone"))
        .description(t("The real microphone you talk into."))
        .build();
    mic.add(&device_row(st, t("Microphone"), true));
    mic.add(&scale_row(
        t("Microphone volume"),
        t("Raise it if your voice is too quiet."),
        -12.0,
        24.0,
        c.input_gain_db as f64,
        db,
        {
            let st = st.clone();
            move |v| st.update(|c| c.input_gain_db = v as f32)
        },
    ));
    let denoise = adw::SwitchRow::builder()
        .title(t("Noise suppression"))
        .subtitle(t("Removes fans, keyboards and other noise around you."))
        .active(c.noise_suppression)
        .build();
    denoise.connect_active_notify({
        let st = st.clone();
        move |r| st.update(|c| c.noise_suppression = r.is_active())
    });
    mic.add(&denoise);
    let gate = adw::SwitchRow::builder()
        .title(t("Noise gate"))
        .subtitle(t("Mutes the microphone while you are quiet."))
        .active(c.gate)
        .build();
    let gate_level =
        scale_row(t("Gate level"), t("Sounds below this level are muted."), -80.0, -20.0, c.gate_db as f64, db, {
            let st = st.clone();
            move |v| st.update(|c| c.gate_db = v as f32)
        });
    gate_level.set_visible(c.gate);
    gate.connect_active_notify({
        let st = st.clone();
        let gate_level = gate_level.clone();
        move |r| {
            gate_level.set_visible(r.is_active());
            st.update(|c| c.gate = r.is_active())
        }
    });
    mic.add(&gate);
    mic.add(&gate_level);
    page.add(&mic);

    // Virtual microphone.
    let virt = adw::PreferencesGroup::builder()
        .title(t("Voicenux Microphone"))
        .description(t("Apps hear your new voice through this microphone. Pick it in Discord, OBS or your game."))
        .build();
    let default = adw::SwitchRow::builder()
        .title(t("Use as default microphone"))
        .subtitle(t("The easiest way: every app that uses the default microphone gets your new voice."))
        .active(c.default_mic)
        .build();
    default.connect_active_notify({
        let st = st.clone();
        move |r| st.update(|c| c.default_mic = r.is_active())
    });
    virt.add(&default);
    virt.add(&scale_row(t("Volume"), t("How loud apps hear you."), -24.0, 12.0, c.mic_volume_db as f64, db, {
        let st = st.clone();
        move |v| st.update(|c| c.mic_volume_db = v as f32)
    }));
    let users = adw::ActionRow::builder().title(t("Used by")).build();
    let users_label = label("", &["dim-label"]);
    users.add_suffix(&users_label);
    let sync_users = {
        let st = st.clone();
        move || {
            let u = st.users.borrow().clone();
            users_label.set_label(&if u.is_empty() { t("No app right now").to_string() } else { u.join(", ") });
        }
    };
    sync_users();
    virt.add(&users);
    page.add(&virt);

    // Hear myself.
    let hear = adw::PreferencesGroup::builder()
        .title(t("Hear myself"))
        .description(t("Where you hear your own changed voice and the soundboard."))
        .build();
    hear.add(&device_row(st, t("Headphones"), false));
    hear.add(&scale_row(
        t("Volume"),
        t("Only for you, apps do not hear this."),
        0.0,
        1.5,
        c.monitor_volume as f64,
        pct,
        {
            let st = st.clone();
            move |v| st.update(|c| c.monitor_volume = v as f32)
        },
    ));
    page.add(&hear);

    // Shortcuts.
    let keys = adw::PreferencesGroup::builder()
        .title(t("Shortcuts"))
        .description(t("Work everywhere, even in games. Your desktop asks once to confirm the keys."))
        .build();
    let setup = gtk::Button::with_label(t("Set up shortcuts"));
    setup.add_css_class("suggested-action");
    setup.set_valign(gtk::Align::Center);
    keys.set_header_suffix(Some(&setup));
    setup.connect_clicked({
        let st = st.clone();
        move |_| shortcuts::configure(&st, &st.keys)
    });
    let key_rows: Vec<(String, adw::ActionRow)> = ACTIONS
        .iter()
        .map(|(id, desc, _)| {
            let row = adw::ActionRow::builder().title(t_dyn(desc)).build();
            keys.add(&row);
            (id.to_string(), row)
        })
        .collect();
    let no_portal = adw::ActionRow::builder()
        .title(t("Your desktop has no global shortcuts"))
        .subtitle(t("Add a shortcut in your desktop's keyboard settings that runs a command like “voicenux toggle” or “voicenux voice robot”."))
        .build();
    keys.add(&no_portal);
    let sync_keys = {
        let st = st.clone();
        let setup = setup.clone();
        let no_portal = no_portal.clone();
        move || {
            let k = st.keys.borrow();
            no_portal.set_visible(k.unavailable);
            setup.set_visible(!k.unavailable);
            setup.set_label(if k.bound.is_empty() { t("Set up shortcuts") } else { t("Change shortcuts") });
            for (id, row) in &key_rows {
                row.set_visible(!k.unavailable);
                let text = k.bound.get(id).cloned().filter(|s| !s.is_empty());
                row.set_subtitle(&text.unwrap_or_else(|| t("Not set").to_string()));
            }
        }
    };
    sync_keys();
    page.add(&keys);

    // Performance.
    let perf = adw::PreferencesGroup::builder().title(t("Performance")).build();
    let low = adw::SwitchRow::builder()
        .title(t("Low latency"))
        .subtitle(t("Less delay between talking and hearing yourself. Pitch effects sound a little rougher."))
        .active(c.low_latency)
        .build();
    low.connect_active_notify({
        let st = st.clone();
        move |r| st.update(|c| c.low_latency = r.is_active())
    });
    perf.add(&low);
    page.add(&perf);

    // App.
    let app = adw::PreferencesGroup::builder().title(t("App")).build();
    let autostart = adw::SwitchRow::builder()
        .title(t("Start with the computer"))
        .subtitle(t("Starts in the background, without a window."))
        .active(c.autostart)
        .build();
    autostart.connect_active_notify({
        let st = st.clone();
        move |r| {
            let on = r.is_active();
            if let Err(e) = crate::app::set_autostart(on) {
                log::error!("autostart: {e}");
            }
            st.update(|c| c.autostart = on)
        }
    });
    app.add(&autostart);
    let tray = adw::SwitchRow::builder()
        .title(t("Keep running when closed"))
        .subtitle(t("Voicenux stays in the tray, so your voice keeps working after you close the window."))
        .active(c.close_to_tray)
        .build();
    if !st.tray.get() {
        tray.set_sensitive(false);
        tray.set_subtitle(t("Needs a tray icon area. On GNOME, install the AppIndicator extension."));
    }
    tray.connect_active_notify({
        let st = st.clone();
        move |r| st.update(|c| c.close_to_tray = r.is_active())
    });
    app.add(&tray);
    let look = adw::ComboRow::builder().title(t("Appearance")).build();
    let looks = ["system", "light", "dark"];
    look.set_model(Some(&gtk::StringList::new(&[t("Follow system"), t("Light"), t("Dark")])));
    look.set_selected(looks.iter().position(|l| *l == c.appearance).unwrap_or(0) as u32);
    look.connect_selected_notify({
        let st = st.clone();
        move |r| st.update(|c| c.appearance = looks[r.selected() as usize].to_string())
    });
    app.add(&look);
    let welcome = adw::ActionRow::builder().title(t("Show the welcome again")).activatable(true).build();
    welcome.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    welcome.connect_activated({
        let st = st.clone();
        let window = window.clone();
        move |_| super::onboarding::show(&st, &window)
    });
    app.add(&welcome);
    let about = adw::ActionRow::builder().title(t("About Voicenux")).activatable(true).build();
    about.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    about.connect_activated({
        let window = window.clone();
        move |_| about_dialog().present(Some(&window))
    });
    app.add(&about);
    page.add(&app);

    st.subscribe(move |c| match c {
        Change::Users => sync_users(),
        Change::Settings => sync_keys(),
        _ => {}
    });

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    view
}

pub fn about_dialog() -> adw::AboutDialog {
    let d = adw::AboutDialog::builder()
        .application_name("Voicenux")
        .application_icon(crate::app::APP_ID)
        .developer_name("Felitendo")
        .version(env!("CARGO_PKG_VERSION"))
        .website("https://github.com/voicenux/voicenux")
        .issue_url("https://github.com/voicenux/voicenux/issues")
        .license_type(gtk::License::Bsd3)
        .comments(t("Change your voice in real time, in any app. Made for Linux and PipeWire."))
        .build();
    d.add_credit_section(
        Some(t("Icons")),
        &["game-icons.net: Lorc, Delapouite, Skoll, Sbed, Carl Olsen and others (CC BY 3.0) https://game-icons.net"],
    );
    d.add_credit_section(
        Some(t("Thanks to")),
        &["Freeverb by Jezar at Dreampoint", "RNNoise by Xiph.Org", "Symphonia", "gtk-rs"],
    );
    d
}
