//! Settings page. Every option lives here, nobody needs to edit a file.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::{label, line_icon, smooth_scroll};
use crate::audio::Device;
use crate::audio::easyeffects::Use as EasyEffects;
use crate::i18n::{fill, t, t_dyn};
use crate::shortcuts::{self, ACTIONS};
use crate::state::{Change, St};

/// How a setting with a slider is shown and typed.
#[derive(Clone, Copy)]
enum Unit {
    Db,
    Percent,
}

impl Unit {
    fn show(self, v: f64) -> String {
        match self {
            Unit::Db => format!("{:+.0} dB", v),
            Unit::Percent => format!("{:.0} %", v * 100.0),
        }
    }

    /// The number to start typing from.
    fn bare(self, v: f64) -> String {
        match self {
            Unit::Db => format!("{v:.1}").trim_end_matches(".0").to_string(),
            Unit::Percent => format!("{:.0}", v * 100.0),
        }
    }

    fn parse(self, text: &str) -> Option<f64> {
        match self {
            Unit::Db => super::typed_number(text, &["db"]),
            Unit::Percent => super::typed_number(text, &["%"]).map(|v| v / 100.0),
        }
    }
}

/// A setting with a slider, and its value that you can click to type one. The slider holds at
/// 0 dB, or at 50 % and 100 %.
fn scale_row(
    title: &str,
    subtitle: &str,
    min: f64,
    max: f64,
    value: f64,
    unit: Unit,
    on: impl Fn(f64) + 'static,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).subtitle(subtitle).build();
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, (max - min) / 200.0);
    scale.set_value(value);
    scale.set_draw_value(false);
    scale.set_width_request(220);
    scale.set_valign(gtk::Align::Center);
    super::notches(
        &scale,
        match unit {
            Unit::Db => &[0.0],
            Unit::Percent => &[0.5, 1.0],
        },
    );
    super::wheel_slider(
        &scale,
        super::grid(match unit {
            Unit::Db => 1.0,
            Unit::Percent => 0.05,
        }),
    );
    let shown = super::value_field(
        {
            let scale = scale.clone();
            move || unit.bare(scale.value())
        },
        {
            let scale = scale.clone();
            move |f, text| {
                if let Some(v) = unit.parse(text) {
                    scale.set_value(v.clamp(min, max));
                }
                // Also puts the old value back after something that is not a number.
                f.set_text(&unit.show(scale.value()));
            }
        },
    );
    shown.add_css_class("numeric");
    shown.set_width_chars(7);
    shown.set_max_width_chars(7);
    shown.set_valign(gtk::Align::Center);
    shown.set_text(&unit.show(value));
    scale.connect_value_changed({
        let shown = shown.clone();
        move |s| {
            shown.set_text(&unit.show(s.value()));
            on(s.value());
        }
    });
    row.add_suffix(&scale);
    row.add_suffix(&shown);
    row
}

/// Device names in full. The list shows them whole (it grows up to the window), the row cuts
/// them only when they really do not fit. The default of a combo row cuts both at 20 letters,
/// and names like "Audient iD4 Headphones" lose their end.
fn full_names(row: &adw::ComboRow) {
    let item_of = |o: &glib::Object| o.downcast_ref::<gtk::ListItem>().cloned();
    let text_of = |item: &gtk::ListItem| item.item().and_downcast::<gtk::StringObject>().map(|s| s.string());

    let current = gtk::SignalListItemFactory::new();
    current.connect_setup(move |_, o| {
        let Some(item) = item_of(o) else { return };
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_max_width_chars(44);
        l.set_width_chars(1);
        item.set_child(Some(&l));
    });
    current.connect_bind(move |_, o| {
        let Some(item) = item_of(o) else { return };
        if let (Some(l), Some(text)) = (item.child().and_downcast::<gtk::Label>(), text_of(&item)) {
            l.set_label(&text);
        }
    });
    row.set_factory(Some(&current));

    // In the list: the whole name and a check at the chosen one, like the default.
    let list = gtk::SignalListItemFactory::new();
    list.connect_setup(move |_, o| {
        let Some(item) = item_of(o) else { return };
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        l.set_hexpand(true);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_max_width_chars(64);
        b.append(&l);
        b.append(&gtk::Image::from_icon_name("object-select-symbolic"));
        item.set_child(Some(&b));
    });
    let check = {
        let row = row.downgrade();
        move |item: &gtk::ListItem| {
            let (Some(row), Some(b)) = (row.upgrade(), item.child()) else { return };
            let chosen = row.selected_item().is_some() && row.selected_item() == item.item();
            if let Some(icon) = b.last_child() {
                icon.set_opacity(if chosen { 1.0 } else { 0.0 });
            }
        }
    };
    // The items in the list, to move the check when the choice changes.
    let shown: Rc<std::cell::RefCell<Vec<glib::WeakRef<gtk::ListItem>>>> = Rc::default();
    row.connect_selected_item_notify({
        let (shown, check) = (shown.clone(), check.clone());
        move |_| {
            shown.borrow_mut().retain(|w| w.upgrade().is_some());
            for item in shown.borrow().iter().filter_map(|w| w.upgrade()) {
                check(&item);
            }
        }
    });
    list.connect_bind(move |_, o| {
        let Some(item) = item_of(o) else { return };
        if let (Some(l), Some(text)) =
            (item.child().and_then(|b| b.first_child()).and_downcast::<gtk::Label>(), text_of(&item))
        {
            l.set_label(&text);
        }
        check(&item);
        let mut list = shown.borrow_mut();
        if !list.iter().any(|w| w.upgrade().as_ref() == Some(&item)) {
            list.push(item.downgrade());
        }
    });
    row.set_list_factory(Some(&list));
}

/// A combo row for devices, with "System default" on top. Keeps itself up to date.
fn device_row(st: &St, title: &str, inputs: bool) -> adw::ComboRow {
    let row = adw::ComboRow::builder().title(title).build();
    full_names(&row);
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

/// Shown while EasyEffects runs. Says where its microphone effects end up.
fn easyeffects_row(st: &St) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(t("Use EasyEffects")).active(st.config.borrow().easyeffects).build();
    let sync = {
        let st = st.clone();
        let row = row.clone();
        move || {
            let u = st.easyeffects.get();
            row.set_visible(u != EasyEffects::Missing);
            row.set_subtitle(match u {
                EasyEffects::After => t("EasyEffects works on the Voicenux Microphone, so its effects come after the voice. In Discord and other apps, pick “Easy Effects Source”."),
                EasyEffects::Before => t("Your EasyEffects microphone effects, like noise reduction, come before the voice. In Discord and other apps, still pick “Voicenux Microphone”, not “Easy Effects Source”."),
                _ if !row.is_active() => t("Run your EasyEffects microphone effects, like noise reduction, before the voice. Pick your real microphone above, not EasyEffects."),
                _ => t("Not in use: EasyEffects works on another microphone. Pick the same one above as in EasyEffects."),
            });
        }
    };
    sync();
    row.connect_active_notify({
        let st = st.clone();
        move |r| st.update(|c| c.easyeffects = r.is_active())
    });
    st.subscribe(move |c| {
        if matches!(c, Change::Devices | Change::Settings) {
            sync();
        }
    });
    row
}

/// A section of the settings: a Lucide icon, a title and its rows in a card.
struct Group {
    widget: gtk::Box,
    head: gtk::Box,
    list: gtk::ListBox,
}

impl Group {
    fn new(pic: &str, title: &str, description: &str) -> Self {
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        head.add_css_class("vn-settings-head");
        // The icon sits on the line of the title, the description goes under both.
        let img = line_icon(pic, 22);
        img.set_valign(gtk::Align::Start);
        img.set_margin_top(1);
        head.append(&img);
        let texts = gtk::Box::new(gtk::Orientation::Vertical, 2);
        texts.set_valign(gtk::Align::Start);
        texts.set_hexpand(true);
        let t = label(title, &["vn-settings-title"]);
        t.set_xalign(0.0);
        texts.append(&t);
        if !description.is_empty() {
            let d = label(description, &["dim-label"]);
            d.set_xalign(0.0);
            d.set_wrap(true);
            texts.append(&d);
        }
        head.append(&texts);
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");
        list.add_css_class("vn-settings-list");
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 12);
        widget.append(&head);
        widget.append(&list);
        Self { widget, head, list }
    }

    fn add(&self, row: &impl IsA<gtk::Widget>) {
        self.list.append(row);
    }

    fn set_header_suffix(&self, w: &impl IsA<gtk::Widget>) {
        w.set_valign(gtk::Align::Center);
        self.head.append(w);
    }
}

/// Settings, and Advanced as a page behind it.
pub struct SettingsPage {
    pub widget: adw::NavigationView,
    pub advanced_search: gtk::SearchEntry,
}

pub fn build(st: &St, window: &adw::ApplicationWindow) -> SettingsPage {
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(t("Settings"), "")));
    let page = gtk::Box::new(gtk::Orientation::Vertical, 30);
    page.set_margin_top(18);
    page.set_margin_bottom(36);
    page.set_margin_start(24);
    page.set_margin_end(24);
    let c = st.config.borrow().clone();

    // Microphone.
    let mic = Group::new("mic", t("Microphone"), t("The real microphone you talk into."));
    mic.add(&device_row(st, t("Microphone"), true));
    mic.add(&scale_row(
        t("Microphone volume"),
        t("Raise it if your voice is too quiet."),
        -12.0,
        24.0,
        c.input_gain_db as f64,
        Unit::Db,
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
    let gate_level = scale_row(
        t("Gate level"),
        t("Sounds below this level are muted."),
        -80.0,
        -20.0,
        c.gate_db as f64,
        Unit::Db,
        {
            let st = st.clone();
            move |v| st.update(|c| c.gate_db = v as f32)
        },
    );
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
    mic.add(&easyeffects_row(st));
    page.append(&mic.widget);

    // Virtual microphone.
    let virt = Group::new(
        "mic-vocal",
        t("Voicenux Microphone"),
        t("Apps hear your new voice through this microphone. Pick it in Discord, OBS or your game."),
    );
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
    virt.add(&scale_row(t("Volume"), t("How loud apps hear you."), -24.0, 12.0, c.mic_volume_db as f64, Unit::Db, {
        let st = st.clone();
        move |v| st.update(|c| c.mic_volume_db = v as f32)
    }));
    let discord = adw::ActionRow::builder()
        .title(t("Sounds in Discord"))
        .subtitle(t("If your friends do not hear the soundboard"))
        .activatable(true)
        .build();
    discord.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    discord.connect_activated({
        let st = st.clone();
        move |r| super::soundboard::discord_help(&st, r)
    });
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
    virt.add(&discord);
    page.append(&virt.widget);

    // Hear myself.
    let hear =
        Group::new("headphones", t("Hear myself"), t("Where you hear your own changed voice and the soundboard."));
    hear.add(&device_row(st, t("Headphones"), false));
    hear.add(&scale_row(
        t("Volume"),
        t("Only for you, apps do not hear this."),
        0.0,
        1.5,
        c.monitor_volume as f64,
        Unit::Percent,
        {
            let st = st.clone();
            move |v| st.update(|c| c.monitor_volume = v as f32)
        },
    ));
    page.append(&hear.widget);

    // Shortcuts.
    let keys = Group::new(
        "keyboard",
        t("Shortcuts"),
        t("Work everywhere, even in games. Your desktop asks once to confirm the keys."),
    );
    let setup = gtk::Button::with_label(t("Set up shortcuts"));
    setup.add_css_class("suggested-action");
    setup.set_valign(gtk::Align::Center);
    keys.set_header_suffix(&setup);
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
    let in_app = adw::ActionRow::builder()
        .title(t("Keys in the app"))
        .subtitle(t("Ctrl+F searches, Ctrl+T turns the voice changer on or off, and more."))
        .activatable(true)
        .build();
    let f1 = label("F1", &["vn-key"]);
    f1.set_valign(gtk::Align::Center);
    in_app.add_suffix(&f1);
    in_app.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    in_app.connect_activated({
        let window = window.clone();
        move |_| super::help::show(&window)
    });
    keys.add(&in_app);
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
    page.append(&keys.widget);
    page.append(&api_group(st).widget);

    // Performance.
    let perf = Group::new("zap", t("Performance"), "");
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
    page.append(&perf.widget);
    page.append(&ai_group(st).widget);

    // App.
    let app = Group::new("settings", t("App"), "");
    let autostart = adw::SwitchRow::builder()
        .title(t("Start with the computer"))
        .subtitle(t("Starts in the background, without a window."))
        .active(c.autostart)
        .build();
    autostart.connect_active_notify({
        let st = st.clone();
        move |r| {
            if r.is_active() != st.config.borrow().autostart {
                crate::app::set_autostart(&st, r.is_active());
            }
        }
    });
    // The welcome dialog asks too.
    st.subscribe({
        let (weak, autostart) = (Rc::downgrade(st), autostart.clone());
        move |c| {
            if let (Change::Settings, Some(st)) = (c, weak.upgrade()) {
                autostart.set_active(st.config.borrow().autostart);
            }
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
    let mono = adw::SwitchRow::builder()
        .title(t("Simple tray icon"))
        .subtitle(t("A plain icon in the color of your panel, instead of the blue logo."))
        .active(c.tray_mono)
        .sensitive(st.tray.get())
        .build();
    mono.connect_active_notify({
        let st = st.clone();
        move |r| st.update(|c| c.tray_mono = r.is_active())
    });
    app.add(&mono);
    // Appearance as buttons side by side, all choices in sight.
    let look = adw::ActionRow::builder().title(t("Appearance")).build();
    let looks = adw::ToggleGroup::new();
    looks.add_css_class("vn-segmented");
    looks.set_valign(gtk::Align::Center);
    // Lucide like the rest of the settings: the theme's own sun is a plain dot in Breeze.
    for (name, icon, text) in
        [("system", "monitor", t("System")), ("light", "sun", t("Light")), ("dark", "moon", t("Dark"))]
    {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.append(&line_icon(icon, 16));
        content.append(&gtk::Label::new(Some(text)));
        looks.add(adw::Toggle::builder().name(name).child(&content).tooltip(text).build());
    }
    looks.set_active_name(Some(&c.appearance));
    looks.connect_active_name_notify({
        let st = st.clone();
        move |g| {
            if let Some(name) = g.active_name() {
                st.update(|c| c.appearance = name.to_string());
            }
        }
    });
    look.add_suffix(&looks);
    app.add(&look);
    let tips = adw::ActionRow::builder()
        .title(t("Show the tips again"))
        .subtitle(t("They show when you open a page"))
        .activatable(true)
        .build();
    tips.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    tips.connect_activated({
        let st = st.clone();
        move |_| {
            st.update(|c| c.tours.clear());
            st.notify(crate::state::Change::Notice(t("The tips show again on each page").to_string()));
        }
    });
    let welcome = adw::ActionRow::builder().title(t("Show the welcome again")).activatable(true).build();
    welcome.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    welcome.connect_activated({
        let st = st.clone();
        let window = window.clone();
        move |_| super::onboarding::show(&st, &window)
    });
    app.add(&welcome);
    app.add(&tips);
    let about = adw::ActionRow::builder().title(t("About Voicenux")).activatable(true).build();
    about.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    about.connect_activated({
        let window = window.clone();
        move |_| about_dialog().present(Some(&window))
    });
    app.add(&about);
    page.append(&app.widget);

    // The rest, for tinkerers, on a page of its own.
    let nav = adw::NavigationView::new();
    let more = Group::new("sliders-horizontal", t("Advanced"), "");
    let open = adw::ActionRow::builder()
        .title(t("Advanced settings"))
        .subtitle(t("For tinkerers: everything else you can set, and experiments."))
        .activatable(true)
        .build();
    open.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    open.connect_activated({
        let nav = nav.downgrade();
        move |_| {
            if let Some(nav) = nav.upgrade() {
                nav.push_by_tag("advanced");
            }
        }
    });
    more.add(&open);
    page.append(&more.widget);

    super::follow_settings(&st.config.borrow());
    st.subscribe({
        let st = Rc::downgrade(st);
        move |c| match c {
            Change::Users => sync_users(),
            Change::Settings => {
                sync_keys();
                if let Some(st) = st.upgrade() {
                    super::follow_settings(&st.config.borrow());
                }
            }
            _ => {}
        }
    });

    super::popup::menus_in(&page);
    let clamp = adw::Clamp::builder().maximum_size(760).tightening_threshold(560).child(&page).build();
    let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).build();
    smooth_scroll(&scroll);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&scroll));
    nav.add(&adw::NavigationPage::builder().title(t("Settings")).tag("settings").child(&view).build());
    let advanced = super::advanced::page(st);
    nav.add(&advanced.widget);
    SettingsPage { widget: nav, advanced_search: advanced.search }
}

/// "5.8 GB", with a comma in German.
fn gigabytes(bytes: u64) -> String {
    let text = format!("{:.1} GB", bytes as f64 / 1e9);
    if crate::i18n::language() == "de" { text.replace('.', ",") } else { text }
}

/// AI voices: on the processor or not, what the engine takes on disk, and a button to remove it.
fn ai_group(st: &St) -> Group {
    let group =
        Group::new("sparkles", t("AI voices"), t("Character voices made by AI, like SpongeBob or Darth Vader."));
    let cpu = adw::SwitchRow::builder()
        .title(t("Run on the processor"))
        .subtitle(t(
            "For computers without a graphics card from NVIDIA, or from AMD since the RX 6000. Takes much of the processor and adds about half a second of delay.",
        ))
        .active(st.config.borrow().ai_cpu)
        .build();
    cpu.connect_active_notify({
        let st = st.clone();
        move |r| {
            let on = r.is_active();
            crate::ai::set_cpu(on);
            st.update(|c| c.ai_cpu = on);
            // The voices turn usable or gray, and the engine may need another build.
            st.notify(Change::Voices);
        }
    });
    group.add(&cpu);
    let row = adw::ActionRow::builder().title(t("AI engine")).build();
    let remove = gtk::Button::with_label(t("Remove"));
    remove.add_css_class("destructive-action");
    remove.set_valign(gtk::Align::Center);
    row.add_suffix(&remove);
    group.add(&row);
    let refresh: Rc<dyn Fn()> = Rc::new({
        let (row, remove) = (row.clone(), remove.clone());
        move || {
            // Anything there counts for removing, also a build for the other setting.
            let there = crate::ai::dir().join("ready").exists();
            remove.set_visible(there);
            if !crate::ai::usable() {
                row.set_subtitle(t(
                    "This computer has no graphics card the AI voices can use. Turn on Run on the processor above.",
                ));
                return;
            }
            if !there {
                row.set_subtitle(t("Not installed. It comes with the first AI voice you pick."));
                return;
            }
            row.set_subtitle(t("Installed"));
            // Thousands of files: counted off the main thread.
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(crate::ai::install::size());
            });
            let row = row.clone();
            gtk::glib::spawn_future_local(async move {
                if let Ok(size) = rx.recv().await {
                    row.set_subtitle(&fill(t("Installed, it takes {} on disk."), &[&gigabytes(size)]));
                }
            });
        }
    });
    refresh();
    remove.connect_clicked({
        let (st, refresh) = (st.clone(), refresh.clone());
        move |b| {
            let dialog = adw::AlertDialog::new(
                Some(t("Remove the AI engine?")),
                Some(t("All AI voices go with it. They download again when you pick one.")),
            );
            dialog.add_response("cancel", t("Cancel"));
            dialog.add_response("remove", t("Remove"));
            dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            let (st, refresh) = (st.clone(), refresh.clone());
            dialog.connect_response(None, move |_, r| {
                if r != "remove" {
                    return;
                }
                // The voice in use goes back to a normal one, so nothing falls silent.
                let current = st.current();
                if crate::ai::model_of(&current.block_values()).is_some() {
                    st.set_voice("baby");
                }
                if let Err(e) = crate::ai::install::remove() {
                    log::warn!("remove AI: {e}");
                }
                refresh();
                st.notify(Change::Voices);
            });
            dialog.present(Some(b));
        }
    });
    st.subscribe(move |c| {
        if matches!(c, Change::Voices) {
            refresh();
        }
    });
    group
}

/// Other apps on this computer, like a Stream Deck plugin, may control Voicenux. See `src/api.rs`.
fn api_group(st: &St) -> Group {
    let group = Group::new(
        "plug",
        t("Other apps"),
        t("Stream Deck plugins, for example for OpenDeck, and scripts can use your voices and sounds."),
    );
    let on = adw::SwitchRow::builder()
        .title(t("Let other apps control Voicenux"))
        .subtitle(t("Only apps on this computer. Web pages can not use it."))
        .active(st.config.borrow().api)
        .build();
    on.connect_active_notify({
        let st = st.clone();
        move |r| st.update(|c| c.api = r.is_active())
    });
    group.add(&on);
    let ask = adw::SwitchRow::builder()
        .title(t("Only apps you allow"))
        .subtitle(t("Voicenux asks the first time an app wants to control it."))
        .active(st.config.borrow().api_ask)
        .build();
    ask.connect_active_notify({
        let st = st.clone();
        move |r| st.update(|c| c.api_ask = r.is_active())
    });
    group.add(&ask);
    let allowed = adw::ExpanderRow::builder().title(t("Allowed apps")).build();
    group.add(&allowed);
    for row in [ask.upcast_ref::<gtk::Widget>(), allowed.upcast_ref()] {
        on.bind_property("active", row, "sensitive").sync_create().build();
    }
    let rows: Rc<std::cell::RefCell<Vec<adw::ActionRow>>> = Rc::default();
    let sync_allowed = {
        let (st, allowed) = (st.clone(), allowed.clone());
        move || {
            let apps = st.config.borrow().api_apps.clone();
            allowed.set_visible(st.config.borrow().api_ask);
            allowed.set_subtitle(&match apps.len() {
                0 => t("None yet").to_string(),
                n => fill(t("Apps: {}"), &[&n.to_string()]),
            });
            allowed.set_enable_expansion(!apps.is_empty());
            for r in rows.borrow_mut().drain(..) {
                allowed.remove(&r);
            }
            for app in apps {
                let row = adw::ActionRow::builder().title(&app.name).subtitle(&app.key).subtitle_lines(2).build();
                let remove = gtk::Button::from_icon_name("user-trash-symbolic");
                remove.add_css_class("flat");
                remove.set_valign(gtk::Align::Center);
                remove.set_tooltip_text(Some(t("Take back")));
                remove.connect_clicked({
                    let st = st.clone();
                    move |_| st.update(|c| c.api_apps.retain(|a| a.key != app.key))
                });
                row.add_suffix(&remove);
                allowed.add_row(&row);
                rows.borrow_mut().push(row);
            }
        }
    };
    sync_allowed();
    let address = format!("ws://127.0.0.1:{0}   http://127.0.0.1:{0}/api", crate::api::PORT);
    let status = adw::ActionRow::builder().subtitle_selectable(true).build();
    status.add_css_class("property");
    let apps = label("", &["dim-label"]);
    apps.set_valign(gtk::Align::Center);
    status.add_suffix(&apps);
    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.set_valign(gtk::Align::Center);
    copy.add_css_class("flat");
    copy.set_tooltip_text(Some(t("Copy")));
    copy.connect_clicked({
        let (st, address) = (st.clone(), address.clone());
        move |b| {
            b.clipboard().set_text(address.split_whitespace().next().unwrap_or(""));
            st.notify(Change::Notice(t("Copied").to_string()));
        }
    });
    status.add_suffix(&copy);
    group.add(&status);
    let docs = adw::ActionRow::builder()
        .title(t("For plugin makers"))
        .subtitle(t("All actions and events, on GitHub."))
        .activatable(true)
        .build();
    docs.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    docs.connect_activated(|r| {
        let window = r.root().and_downcast::<gtk::Window>();
        gtk::UriLauncher::new("https://github.com/voicenux/voicenux/blob/main/API.md").launch(
            window.as_ref(),
            None::<&gtk::gio::Cancellable>,
            |_| {},
        );
    });
    group.add(&docs);
    let sync = {
        let st = st.clone();
        move || {
            let s = st.api.borrow().clone();
            status.set_visible(s.running || s.error.is_some());
            copy.set_visible(s.running);
            match &s.error {
                Some(e) => {
                    status.set_title(t("Could not start"));
                    status.set_subtitle(&fill(
                        t("Port {} is taken, maybe by another Voicenux. {}"),
                        &[&crate::api::PORT.to_string(), e],
                    ));
                    apps.set_label("");
                }
                None => {
                    status.set_title(t("Address"));
                    status.set_subtitle(&address);
                    apps.set_label(&match s.clients {
                        0 => t("No app connected").to_string(),
                        n => fill(t("Apps connected: {}"), &[&n.to_string()]),
                    });
                }
            }
        }
    };
    sync();
    // The list only changes with the settings, and is built again then.
    let shown = std::cell::RefCell::new(st.config.borrow().api_apps.clone());
    st.subscribe({
        let st = Rc::downgrade(st);
        move |c| match c {
            Change::Api => sync(),
            Change::Settings => {
                let Some(st) = st.upgrade() else { return };
                let now = st.config.borrow().api_apps.clone();
                if *shown.borrow() != now {
                    shown.replace(now);
                    sync_allowed();
                } else {
                    allowed.set_visible(st.config.borrow().api_ask);
                }
            }
            _ => {}
        }
    });
    group
}

/// Asks whether an app may control Voicenux, see `api::answer`. You have to pick: closing the
/// window leaves the question for next time.
pub fn ask_app(st: &St, window: &adw::ApplicationWindow, peer: &crate::api::Peer) {
    let dialog = adw::AlertDialog::new(
        None,
        Some(t(
            "The app could change your voice and play sounds into your microphone. You can take it back in Settings, under Other apps.",
        )),
    );
    if peer.name.is_empty() {
        dialog.set_heading(Some(t("Allow an unknown app to control Voicenux?")));
    } else {
        // The name of the app stands out in the color of links.
        let color = if adw::StyleManager::default().is_dark() { "#7fb0ff" } else { "#1d63d8" };
        let name = format!("<span foreground=\"{color}\">{}</span>", glib::markup_escape_text(&peer.name));
        dialog.set_heading_use_markup(true);
        dialog.set_heading(Some(&fill(&glib::markup_escape_text(t("Allow {} to control Voicenux?")), &[&name])));
    }
    if !peer.key.is_empty() {
        let extra = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let path = label(&peer.key, &["dim-label", "caption"]);
        path.set_wrap(true);
        path.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        extra.append(&path);
        if std::path::Path::new(&peer.file).exists() {
            // Small and quiet, the answer is what matters.
            let open = gtk::Button::builder()
                .child(&adw::ButtonContent::builder().icon_name("folder-open-symbolic").label(t("Open folder")).build())
                .halign(gtk::Align::Center)
                .css_classes(["flat", "caption", "dim-label", "vn-open-folder"])
                .build();
            open.connect_clicked({
                let (window, file) = (window.clone(), peer.file.clone());
                move |_| {
                    // The folder in the default file manager. Asking org.freedesktop.FileManager1 to
                    // show the file is slow, and can start another one, like GNOME Files on KDE.
                    let Some(folder) = gtk::gio::File::for_path(&file).parent() else { return };
                    gtk::FileLauncher::new(Some(&folder)).launch(Some(&window), None::<&gtk::gio::Cancellable>, |_| {});
                }
            });
            extra.append(&open);
            // The path can be copied. Then the button takes the focus, else the path would get it,
            // and with it all its text would be selected.
            path.set_selectable(true);
            dialog.set_focus(Some(&open));
        }
        dialog.set_extra_child(Some(&extra));
    }
    dialog.add_response("deny", t("Do not allow"));
    dialog.add_response("allow", t("Allow"));
    dialog.set_response_appearance("allow", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("allow"));
    dialog.set_can_close(false);
    let answered = Rc::new(std::cell::Cell::new(false));
    dialog.connect_response(None, {
        let (st, peer, answered) = (st.clone(), peer.clone(), answered.clone());
        move |_, response| {
            answered.set(true);
            crate::api::answer(&st, &peer, Some(response == "allow"));
        }
    });
    dialog.connect_closed({
        let (st, peer) = (st.clone(), peer.clone());
        move |_| {
            if !answered.get() {
                crate::api::answer(&st, &peer, None);
            }
        }
    });
    dialog.present(Some(window));
}

/// About Voicenux: the logo, the version, links and who to thank.
pub fn about_dialog() -> adw::Dialog {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(4);
    body.set_margin_bottom(26);
    body.set_margin_start(28);
    body.set_margin_end(28);
    let logo = super::logo(104);
    logo.add_css_class("vn-about-logo");
    body.append(&logo);
    body.append(&label("Voicenux", &["vn-hero-title"]));
    let version = label(env!("CARGO_PKG_VERSION"), &["vn-version", "numeric"]);
    version.set_halign(gtk::Align::Center);
    body.append(&version);
    let about = label(t("Change your voice in real time, in any app. Made for Linux and PipeWire."), &["dim-label"]);
    about.set_wrap(true);
    about.set_justify(gtk::Justification::Center);
    about.set_max_width_chars(40);
    about.set_margin_top(4);
    body.append(&about);
    body.append(&label(&fill(t("Made by {}"), &["Felitendo"]), &["caption", "dim-label"]));

    let links = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    links.set_halign(gtk::Align::Center);
    links.set_margin_top(10);
    for (pic, text, url) in [
        ("globe", t("Website"), "https://github.com/voicenux/voicenux"),
        ("bug", t("Report a bug"), "https://github.com/voicenux/voicenux/issues"),
        ("coffee", t("Buy me a coffee"), "https://ko-fi.com/felitendo"),
    ] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&line_icon(pic, 18));
        row.append(&label(text, &[]));
        let b = gtk::Button::builder().child(&row).css_classes(["pill"]).tooltip_text(url).build();
        b.connect_clicked(move |b| {
            let window = b.root().and_downcast::<gtk::Window>();
            gtk::UriLauncher::new(url).launch(window.as_ref(), None::<&gtk::gio::Cancellable>, |_| {});
        });
        links.append(&b);
    }
    body.append(&links);

    let credits = gtk::Box::new(gtk::Orientation::Vertical, 6);
    credits.add_css_class("vn-keys-card");
    credits.set_margin_top(14);
    for (title, names) in [
        (t("Pictures"), vec!["Fluent Emoji by Microsoft (MIT)", "Lucide (ISC)"]),
        (t("Thanks to"), vec!["Freeverb by Jezar at Dreampoint", "RNNoise by Xiph.Org", "Symphonia", "gtk-rs"]),
    ] {
        let head = label(title, &["heading"]);
        head.set_xalign(0.0);
        credits.append(&head);
        let list = label(&names.join(" · "), &["dim-label"]);
        list.set_xalign(0.0);
        list.set_wrap(true);
        credits.append(&list);
    }
    let license = label(&fill(t("Free software under the {} license."), &["BSD-3-Clause"]), &["caption", "dim-label"]);
    license.set_xalign(0.0);
    license.set_margin_top(4);
    credits.append(&license);
    body.append(&credits);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::builder().show_title(false).build());
    view.set_content(Some(&body));
    adw::Dialog::builder().content_width(520).child(&view).build()
}
