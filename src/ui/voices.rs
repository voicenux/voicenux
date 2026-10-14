//! The voice page: a grid of voice cards and the selected voice on the right.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::visuals::Orb;
use super::{animate_in, cat_class, clear, icon, label, smooth_scroll};
use crate::dsp::blocks::{self, Kind};
use crate::i18n::{t, t_dyn};
use crate::state::{Change, St};
use crate::voice::{CATEGORIES, Voice, category};

struct Card {
    child: gtk::FlowBoxChild,
    card: gtk::Box,
    star: gtk::Button,
}

pub struct VoicesPage {
    pub widget: adw::ToolbarView,
    pub search: gtk::SearchEntry,
}

/// Called with a voice id when someone wants to edit a voice in Voicelab.
pub type OpenLab = Rc<dyn Fn(&str)>;

pub fn build(st: &St, open_lab: OpenLab) -> VoicesPage {
    let header = adw::HeaderBar::new();
    let search = gtk::SearchEntry::builder().placeholder_text(t("Search voices")).width_request(320).build();
    header.set_title_widget(Some(&search));
    let random = gtk::Button::from_icon_name("media-playlist-shuffle-symbolic");
    random.set_tooltip_text(Some(t("Random voice")));
    header.pack_end(&random);
    random.connect_clicked({
        let st = st.clone();
        move |_| st.random_voice()
    });

    let filter = Rc::new(RefCell::new((String::from("all"), String::new())));

    // Category chips.
    let chips = adw::WrapBox::builder().child_spacing(8).line_spacing(8).build();
    chips.set_margin_start(20);
    chips.set_margin_end(20);
    chips.set_margin_top(6);
    chips.set_margin_bottom(12);

    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(16)
        .column_spacing(18)
        .row_spacing(18)
        .valign(gtk::Align::Start)
        .margin_start(20)
        .margin_end(20)
        .margin_top(14)
        .margin_bottom(24)
        .activate_on_single_click(true)
        .build();
    grid.add_css_class("vn-grid");
    let grid_scroll =
        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&grid).build();
    smooth_scroll(&grid_scroll);

    let empty = adw::StatusPage::builder()
        .icon_name("edit-find-symbolic")
        .title(t("No voices found"))
        .description(t("Try another word or category."))
        .build();
    let grid_stack = gtk::Stack::new();
    grid_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    grid_stack.add_named(&grid_scroll, Some("grid"));
    grid_stack.add_named(&empty, Some("empty"));

    let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
    left.set_hexpand(true);
    left.append(&chips);
    left.append(&grid_stack);

    let detail = Detail::new(st, open_lab.clone());

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&left);
    body.append(&detail.widget);

    let widget = adw::ToolbarView::new();
    widget.add_top_bar(&header);
    widget.set_content(Some(&body));

    let cards: Rc<RefCell<HashMap<String, Card>>> = Rc::default();
    let order: Rc<RefCell<Vec<String>>> = Rc::default();

    let refilter = {
        let grid = grid.clone();
        let grid_stack = grid_stack.clone();
        let cards = cards.clone();
        let order = order.clone();
        let filter = filter.clone();
        let st = st.clone();
        Rc::new(move |animate: bool| {
            let (cat, query) = filter.borrow().clone();
            let query = query.to_lowercase();
            let mut shown = 0;
            for id in order.borrow().iter() {
                let Some(c) = cards.borrow().get(id).map(|c| c.child.clone()) else { continue };
                let Some(v) = st.voices.borrow().iter().find(|v| &v.id == id).cloned() else { continue };
                let in_cat = match cat.as_str() {
                    "all" => true,
                    "favorites" => st.is_favorite(id),
                    c => v.category == c,
                };
                let name = t_dyn(&v.name).to_lowercase();
                let matches = query.is_empty()
                    || name.contains(&query)
                    || v.name.to_lowercase().contains(&query)
                    || t_dyn(&v.description).to_lowercase().contains(&query)
                    || t_dyn(category(&v.category).label).to_lowercase().contains(&query);
                let visible = in_cat && matches;
                c.set_visible(visible);
                if visible {
                    if animate {
                        animate_in(&c, shown);
                    }
                    shown += 1;
                }
            }
            grid_stack.set_visible_child_name(if shown == 0 { "empty" } else { "grid" });
            let _ = &grid;
        })
    };

    let rebuild_chips = {
        let chips = chips.clone();
        let filter = filter.clone();
        let refilter = refilter.clone();
        let st = st.clone();
        Rc::new(move || {
            clear(&chips);
            let has_custom = st.voices.borrow().iter().any(|v| !v.builtin);
            let mut list: Vec<(&str, String)> = vec![("all", t("All").into()), ("favorites", t("Favorites").into())];
            if has_custom {
                list.push(("custom", t("My voices").into()));
            }
            for c in CATEGORIES.iter().filter(|c| c.id != "custom") {
                list.push((c.id, t_dyn(c.label)));
            }
            let current = filter.borrow().0.clone();
            let mut first: Option<gtk::ToggleButton> = None;
            for (id, name) in list {
                let b = gtk::ToggleButton::new();
                let inner = gtk::Box::new(gtk::Orientation::Horizontal, 7);
                let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                dot.add_css_class("vn-chip-dot");
                dot.set_valign(gtk::Align::Center);
                if id == "favorites" {
                    let s = gtk::Image::from_icon_name("starred-symbolic");
                    s.set_pixel_size(13);
                    inner.append(&s);
                } else {
                    inner.append(&dot);
                }
                inner.append(&gtk::Label::new(Some(&name)));
                b.set_child(Some(&inner));
                b.add_css_class("vn-chip");
                b.add_css_class(&cat_class(id));
                b.set_active(id == current);
                match &first {
                    Some(f) => b.set_group(Some(f)),
                    None => first = Some(b.clone()),
                }
                let filter = filter.clone();
                let refilter = refilter.clone();
                let id = id.to_string();
                b.connect_toggled(move |b| {
                    if b.is_active() {
                        filter.borrow_mut().0 = id.clone();
                        refilter(true);
                    }
                });
                chips.append(&b);
                let _ = &chips;
            }
            if !st.voices.borrow().iter().any(|v| !v.builtin) && current == "custom" {
                filter.borrow_mut().0 = "all".into();
            }
        })
    };

    let update_active = {
        let cards = cards.clone();
        let st = st.clone();
        Rc::new(move || {
            let current = st.config.borrow().voice.clone();
            let on = st.config.borrow().voice_on;
            for (id, c) in cards.borrow().iter() {
                let active = *id == current;
                if active {
                    c.card.add_css_class("active");
                } else {
                    c.card.remove_css_class("active");
                }
                if active && !on {
                    c.card.add_css_class("off");
                } else {
                    c.card.remove_css_class("off");
                }
                let fav = st.is_favorite(id);
                c.star.set_icon_name(if fav { "starred-symbolic" } else { "non-starred-symbolic" });
                if fav {
                    c.star.add_css_class("on");
                } else {
                    c.star.remove_css_class("on");
                }
            }
        })
    };

    let rebuild_cards = {
        let grid = grid.clone();
        let cards = cards.clone();
        let order = order.clone();
        let st = st.clone();
        let refilter = refilter.clone();
        let update_active = update_active.clone();
        Rc::new(move || {
            clear(&grid);
            cards.borrow_mut().clear();
            order.borrow_mut().clear();
            let voices = st.voices.borrow().clone();
            for v in voices.iter() {
                let c = card(&st, v);
                grid.append(&c.child);
                order.borrow_mut().push(v.id.clone());
                cards.borrow_mut().insert(v.id.clone(), c);
            }
            update_active();
            refilter(true);
        })
    };

    grid.connect_child_activated({
        let st = st.clone();
        let cards = cards.clone();
        move |_, child| {
            let id = cards.borrow().iter().find(|(_, c)| &c.child == child).map(|(id, _)| id.clone());
            if let Some(id) = id {
                st.set_voice(&id);
            }
        }
    });

    search.connect_search_changed({
        let filter = filter.clone();
        let refilter = refilter.clone();
        move |s| {
            filter.borrow_mut().1 = s.text().to_string();
            refilter(false);
        }
    });

    rebuild_chips();
    rebuild_cards();
    detail.show(&st.current(), false);

    st.subscribe({
        let rebuild_cards = rebuild_cards.clone();
        let rebuild_chips = rebuild_chips.clone();
        let update_active = update_active.clone();
        let refilter = refilter.clone();
        let filter = filter.clone();
        let cards = cards.clone();
        let st = st.clone();
        let detail = detail.clone();
        move |change| match change {
            Change::Voices => {
                rebuild_chips();
                rebuild_cards();
                detail.show(&st.current(), false);
            }
            Change::VoiceEdited(id) => {
                if let Some(v) = st.voice(id) {
                    if let Some(c) = cards.borrow().get(id) {
                        refresh_card(c, &v);
                    }
                    if *id == st.config.borrow().voice {
                        detail.edited(&v);
                    }
                }
            }
            Change::Voice => {
                update_active();
                detail.show(&st.current(), true);
                let id = st.config.borrow().voice.clone();
                if let Some(c) = cards.borrow().get(&id)
                    && c.child.is_visible()
                {
                    c.child.grab_focus();
                }
            }
            Change::Toggles => {
                update_active();
                detail.toggles();
            }
            Change::Favorites => {
                update_active();
                detail.toggles();
                if filter.borrow().0 == "favorites" {
                    refilter(false);
                }
            }
            _ => {}
        }
    });

    VoicesPage { widget, search }
}

fn refresh_card(c: &Card, v: &Voice) {
    // Custom voices can change name and picture in Voicelab.
    let mut child = c.card.first_child();
    while let Some(w) = child {
        if let Some(img) = w.downcast_ref::<gtk::Image>() {
            img.set_paintable(Some(&super::texture(&v.icon)));
        }
        if let Some(l) = w.downcast_ref::<gtk::Label>()
            && l.has_css_class("voice-name")
        {
            l.set_label(&t_dyn(&v.name));
        }
        child = w.next_sibling();
    }
}

fn card(st: &St, v: &Voice) -> Card {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("voice-card");
    card.add_css_class(&cat_class(&v.category));
    card.set_size_request(124, 146);

    let img = icon(&v.icon, 72);
    img.add_css_class("voice-icon");
    img.set_margin_top(8);
    card.append(&img);
    let name = label(&t_dyn(&v.name), &["voice-name"]);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_max_width_chars(14);
    card.append(&name);

    let star = gtk::Button::from_icon_name("non-starred-symbolic");
    star.add_css_class("flat");
    star.add_css_class("vn-star");
    star.set_halign(gtk::Align::End);
    star.set_valign(gtk::Align::Start);
    star.set_margin_top(6);
    star.set_margin_end(6);
    star.set_tooltip_text(Some(t("Favorite")));
    star.connect_clicked({
        let st = st.clone();
        let id = v.id.clone();
        move |_| st.toggle_favorite(&id)
    });

    // The overlay lifts on hover, so the star moves along with the card.
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("vn-lift");
    overlay.set_child(Some(&card));
    overlay.add_overlay(&star);

    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&overlay));
    child.set_tooltip_text(Some(&t_dyn(&v.description)));
    Card { child, card, star }
}

/// The selected voice: orb, name, and its main settings.
#[derive(Clone)]
struct Detail {
    widget: gtk::ScrolledWindow,
    inner: gtk::Box,
    orb: Rc<Orb>,
    cat: gtk::Label,
    name: gtk::Label,
    about: gtk::Label,
    fav: gtk::Button,
    fav_state: gtk::Stack,
    params: gtk::Box,
    key_slot: gtk::Box,
    reset: gtk::Button,
    note: gtk::Label,
    st: St,
    shown: Rc<RefCell<String>>,
    busy: Rc<Cell<bool>>,
}

impl Detail {
    fn new(st: &St, open_lab: OpenLab) -> Self {
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);
        inner.set_margin_top(18);
        inner.set_margin_bottom(24);
        inner.set_margin_start(22);
        inner.set_margin_end(22);

        let orb = Rc::new(Orb::new(st, 224));
        inner.append(&orb.widget);

        let cat = label("", &["vn-detail-cat"]);
        cat.set_margin_top(4);
        let name = label("", &["vn-detail-name"]);
        name.set_wrap(true);
        name.set_justify(gtk::Justification::Center);
        let about = label("", &["dim-label"]);
        about.set_wrap(true);
        about.set_justify(gtk::Justification::Center);
        about.set_max_width_chars(30);
        inner.append(&cat);
        inner.append(&name);
        inner.append(&about);

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::Center);
        buttons.set_margin_top(10);
        // Both labels in a stack: the button keeps the width of the longer one, nothing jumps.
        let fav_state = gtk::Stack::new();
        fav_state.set_transition_type(gtk::StackTransitionType::Crossfade);
        for (name, icon, text) in
            [("off", "non-starred-symbolic", t("Add to favorites")), ("on", "starred-symbolic", t("Favorite"))]
        {
            fav_state.add_named(&adw::ButtonContent::builder().icon_name(icon).label(text).build(), Some(name));
        }
        let fav = gtk::Button::builder().child(&fav_state).build();
        fav.add_css_class("pill");
        fav.add_css_class("image-text-button");
        fav.add_css_class("vn-fav");
        let lab = gtk::Button::new();
        lab.add_css_class("pill");
        lab.set_child(Some(
            &adw::ButtonContent::builder().icon_name("applications-science-symbolic").label(t("Customize")).build(),
        ));
        buttons.append(&fav);
        buttons.append(&lab);
        inner.append(&buttons);
        let key_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        key_slot.set_halign(gtk::Align::Center);
        inner.append(&key_slot);

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        head.set_margin_top(18);
        let title = label(t("Voice settings"), &["heading"]);
        title.set_halign(gtk::Align::Start);
        title.set_hexpand(true);
        let reset = gtk::Button::with_label(t("Reset"));
        reset.add_css_class("flat");
        reset.add_css_class("small");
        head.append(&title);
        head.append(&reset);
        inner.append(&head);

        let params = gtk::Box::new(gtk::Orientation::Vertical, 8);
        params.set_margin_top(4);
        inner.append(&params);

        let note = label("", &["dim-label", "caption"]);
        note.set_wrap(true);
        note.set_margin_top(10);
        note.set_xalign(0.0);
        inner.append(&note);

        let widget = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(320)
            .child(&inner)
            .build();
        widget.add_css_class("vn-detail");
        smooth_scroll(&widget);

        let d = Self {
            widget,
            inner,
            orb,
            cat,
            name,
            about,
            fav: fav.clone(),
            fav_state,
            params,
            key_slot,
            reset: reset.clone(),
            note,
            st: st.clone(),
            shown: Rc::default(),
            busy: Rc::default(),
        };
        fav.connect_clicked({
            let st = st.clone();
            let shown = d.shown.clone();
            move |_| st.toggle_favorite(&shown.borrow())
        });
        lab.connect_clicked({
            let shown = d.shown.clone();
            move |_| open_lab(&shown.borrow())
        });
        reset.connect_clicked({
            let st = st.clone();
            let shown = d.shown.clone();
            let d = d.clone();
            move |_| {
                let id = shown.borrow().clone();
                st.reset_voice(&id);
                if let Some(v) = st.voice(&id) {
                    d.show(&v, false);
                }
            }
        });
        d
    }

    fn show(&self, v: &Voice, animate: bool) {
        *self.shown.borrow_mut() = v.id.clone();
        let c = category(&v.category);
        for cat in CATEGORIES {
            self.inner.remove_css_class(&cat_class(cat.id));
        }
        self.inner.add_css_class(&cat_class(&v.category));
        self.orb.set_voice(&v.icon, c.color, animate);
        self.cat.set_label(&t_dyn(c.label).to_uppercase());
        self.name.set_label(&t_dyn(&v.name));
        let about =
            if v.description.is_empty() { t("Made by you in Voicelab.").to_string() } else { t_dyn(&v.description) };
        self.about.set_label(&about);
        self.params_for(v);
        clear(&self.key_slot);
        self.key_slot.append(&super::keys::button(&self.st, &format!("voice:{}", v.id), &t_dyn(&v.name)));
        self.toggles();
        if animate {
            for w in [self.name.upcast_ref::<gtk::Widget>(), self.about.upcast_ref(), self.params.upcast_ref()] {
                w.remove_css_class("vn-detail-pop");
                let w = w.clone();
                glib::idle_add_local_once(move || w.add_css_class("vn-detail-pop"));
            }
        }
    }

    fn edited(&self, v: &Voice) {
        if self.busy.get() || *self.shown.borrow() != v.id {
            self.reset.set_visible(self.st.is_tweaked(&v.id));
            return;
        }
        self.show(v, false);
    }

    fn toggles(&self) {
        let id = self.shown.borrow().clone();
        let fav = self.st.is_favorite(&id);
        self.fav_state.set_visible_child_name(if fav { "on" } else { "off" });
        if fav {
            self.fav.add_css_class("on");
        } else {
            self.fav.remove_css_class("on");
        }
        let v = self.st.voice(&id);
        let note = match v {
            Some(v) if v.has_background() && !self.st.config.borrow().background => {
                t("This voice has a background sound. Turn on Background below to hear it.")
            }
            _ => "",
        };
        self.note.set_label(note);
        self.note.set_visible(!note.is_empty());
        self.reset.set_visible(self.st.is_tweaked(&id));
    }

    fn params_for(&self, v: &Voice) {
        clear(&self.params);
        let mut any = false;
        for (bi, b) in v.blocks.iter().enumerate() {
            let Some(spec) = blocks::find(&b.kind) else { continue };
            if !b.enabled {
                continue;
            }
            let values = b.values();
            for (pi, p) in spec.params.iter().enumerate() {
                if !p.main {
                    continue;
                }
                any = true;
                let row = param_row(p, values[pi], {
                    let st = self.st.clone();
                    let id = v.id.clone();
                    let key = p.key;
                    let busy = self.busy.clone();
                    let reset = self.reset.clone();
                    move |value| {
                        busy.set(true);
                        st.set_param(&id, bi, key, value);
                        busy.set(false);
                        reset.set_visible(st.is_tweaked(&id));
                    }
                });
                self.params.append(&row);
            }
        }
        if !any {
            self.params.append(&label(t("This voice has nothing to adjust."), &["dim-label"]));
        }
    }
}

/// A labelled slider (or switch, or dropdown) for one parameter.
pub fn param_row(p: &'static blocks::ParamSpec, value: f32, on_change: impl Fn(f32) + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.add_css_class("vn-param");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = label(t_dyn(p.label).as_str(), &["vn-param-label"]);
    name.set_hexpand(true);
    name.set_xalign(0.0);
    top.append(&name);
    row.append(&top);

    match p.kind {
        Kind::Toggle => {
            let sw = gtk::Switch::new();
            sw.set_active(value >= 0.5);
            sw.set_valign(gtk::Align::Center);
            sw.connect_active_notify(move |s| on_change(if s.is_active() { 1.0 } else { 0.0 }));
            top.append(&sw);
            row.set_margin_bottom(0);
        }
        Kind::Choice(list) => {
            let names: Vec<String> = list.iter().map(|c| t_dyn(c)).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let dd = gtk::DropDown::from_strings(&names);
            dd.set_selected(value as u32);
            dd.connect_selected_notify(move |d| on_change(d.selected() as f32));
            top.append(&dd);
        }
        Kind::Float | Kind::Int => {
            let shown = label(&blocks::format_value(p, value), &["vn-param-value"]);
            top.append(&shown);
            // Log sliders run on log(value), the rest on the value itself.
            let (lo, hi, v) = if p.log {
                (p.min.max(1e-3).ln() as f64, p.max.ln() as f64, value.max(1e-3).ln() as f64)
            } else {
                (p.min as f64, p.max as f64, value as f64)
            };
            let step = if matches!(p.kind, Kind::Int) { 1.0 } else { (hi - lo) / 400.0 };
            let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, lo, hi, step);
            scale.set_value(v);
            scale.set_draw_value(false);
            scale.set_hexpand(true);
            if !p.log && p.min < 0.0 && p.max > 0.0 {
                scale.add_mark(0.0, gtk::PositionType::Bottom, None);
            }
            let log = p.log;
            scale.connect_value_changed(move |s| {
                let raw = s.value() as f32;
                let v = p.clamp(if log { raw.exp() } else { raw });
                shown.set_label(&blocks::format_value(p, v));
                on_change(v);
            });
            row.append(&scale);
        }
    }
    row
}
