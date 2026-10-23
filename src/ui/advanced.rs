//! Advanced settings, like the flags of a browser: a list of switches to search, for people who
//! want to set everything, and for experiments. Settings opens it from its last row. A new flag is
//! one more line in `FLAGS` and a field in `Config`.

use std::rc::Rc;

use adw::prelude::*;

use super::{fold, label};
use crate::config::Config;
use crate::i18n::{t, t_dyn};
use crate::state::{Change, St};

/// One switch: what it is called and does, and the setting behind it.
pub struct Flag {
    pub title: &'static str,
    pub about: &'static str,
    get: fn(&Config) -> bool,
    set: fn(&mut Config, bool),
}

pub const FLAGS: &[Flag] = &[
    Flag {
        title: "Hear changes while you drag",
        about: "The voice follows its sliders right away. When off, it changes when you let go.",
        get: |c| c.live_sliders,
        set: |c, on| c.live_sliders = on,
    },
    Flag {
        title: "Mouse wheel moves sliders",
        about: "Turn the wheel over a slider to change it. While the page scrolls, it keeps scrolling.",
        get: |c| c.wheel_sliders,
        set: |c, on| c.wheel_sliders = on,
    },
    Flag {
        title: "Random voice button",
        about: "Random voice at the top of the voice page. Its arrow picks out of your favorites. Ctrl+R works without it.",
        get: |c| c.random_voice_button,
        set: |c, on| c.random_voice_button = on,
    },
];

pub struct AdvancedPage {
    pub widget: adw::NavigationPage,
    pub search: gtk::SearchEntry,
}

pub fn page(st: &St) -> AdvancedPage {
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(t("Advanced"), "")));
    let (search_box, search) = super::search_field(t("Search advanced settings"));
    header.pack_start(&search_box);
    let reset = gtk::Button::with_label(t("Reset all"));
    reset.set_tooltip_text(Some(t("Puts every switch here back to how it came")));
    header.pack_start(&reset);

    let intro =
        label(t("For tinkerers who want to set everything. Some are experiments and may change."), &["dim-label"]);
    intro.set_xalign(0.0);
    intro.set_wrap(true);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    list.add_css_class("vn-settings-list");
    let defaults = Config::default();
    let mut rows = Vec::new();
    for flag in FLAGS {
        let row = adw::ActionRow::builder().title(t_dyn(flag.title)).subtitle(t_dyn(flag.about)).build();
        // Changed ones stand out, like in the flags of a browser. The mark goes before the switch.
        let changed = label(t("Changed"), &["vn-flag-changed"]);
        changed.set_valign(gtk::Align::Center);
        row.add_suffix(&changed);
        let switch = gtk::Switch::builder().valign(gtk::Align::Center).build();
        row.add_suffix(&switch);
        row.set_activatable_widget(Some(&switch));
        switch.connect_active_notify({
            let st = st.clone();
            move |s| {
                if (flag.get)(&st.config.borrow()) != s.is_active() {
                    st.update(|c| (flag.set)(c, s.is_active()));
                }
            }
        });
        // The search finds English and translated words alike.
        let words = fold(&format!("{} {} {} {}", flag.title, flag.about, t_dyn(flag.title), t_dyn(flag.about)));
        list.append(&row);
        rows.push((flag, row, switch, changed, words));
    }
    let none = adw::StatusPage::builder()
        .paintable(&super::load_picture("magnifying-glass-tilted-left"))
        .title(t("No settings found"))
        .description(t("Try another word."))
        .vexpand(true)
        .build();
    none.set_visible(false);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_top(18);
    body.set_margin_bottom(36);
    body.set_margin_start(24);
    body.set_margin_end(24);
    body.append(&intro);
    body.append(&list);
    body.append(&none);
    let clamp = adw::Clamp::builder().maximum_size(760).tightening_threshold(560).child(&body).build();
    let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).build();
    super::smooth_scroll(&scroll);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&scroll));
    // Typing anywhere on the page searches.
    search.set_key_capture_widget(Some(&view));

    let rows = Rc::new(rows);
    let sync = {
        let (st, rows, reset) = (st.clone(), rows.clone(), reset.clone());
        move || {
            let c = st.config.borrow().clone();
            let mut any = false;
            for (flag, _, switch, changed, _) in rows.iter() {
                let on = (flag.get)(&c);
                if switch.is_active() != on {
                    switch.set_active(on);
                }
                let differs = on != (flag.get)(&defaults);
                changed.set_visible(differs);
                any |= differs;
            }
            reset.set_sensitive(any);
        }
    };
    sync();
    st.subscribe(move |c| {
        if matches!(c, Change::Settings) {
            sync();
        }
    });
    reset.connect_clicked({
        let st = st.clone();
        move |_| {
            let defaults = Config::default();
            st.update(|c| {
                for flag in FLAGS {
                    (flag.set)(c, (flag.get)(&defaults));
                }
            });
        }
    });
    search.connect_search_changed({
        let (rows, list) = (rows.clone(), list.clone());
        move |s| {
            let words: Vec<String> = fold(&s.text()).split_whitespace().map(String::from).collect();
            let mut found = false;
            for (_, row, _, _, text) in rows.iter() {
                let hit = words.iter().all(|w| text.contains(w.as_str()));
                row.set_visible(hit);
                found |= hit;
            }
            list.set_visible(found);
            none.set_visible(!found);
        }
    });

    let widget = adw::NavigationPage::builder().title(t("Advanced")).tag("advanced").child(&view).build();
    AdvancedPage { widget, search }
}
