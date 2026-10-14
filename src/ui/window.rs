//! The main window: sidebar, pages and the bottom bar.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::{bottombar, label, logo, settings, soundboard, voicelab, voices};
use crate::i18n::{t, t_dyn};
use crate::state::{Change, St};

#[derive(Clone)]
pub struct Window {
    pub window: adw::ApplicationWindow,
    pub toast: adw::ToastOverlay,
    pub stack: gtk::Stack,
    nav: gtk::ListBox,
    lab: voicelab::Lab,
    st: St,
}

pub const PAGES: &[(&str, &str, &str)] = &[
    ("voices", "audio-input-microphone-symbolic", "Voices"),
    ("soundboard", "view-app-grid-symbolic", "Soundboard"),
    ("voicelab", "applications-science-symbolic", "Voicelab"),
    ("settings", "preferences-system-symbolic", "Settings"),
];

impl Window {
    pub fn show_page(&self, name: &str) {
        // Voicelab opens on the voice you use, if it is one of yours.
        if name == "voicelab" {
            let v = self.st.current();
            if !v.builtin {
                self.lab.open(&v.id);
            }
        }
        self.stack.set_visible_child_name(name);
        if let Some(i) = PAGES.iter().position(|p| p.0 == name)
            && let Some(row) = self.nav.row_at_index(i as i32)
        {
            self.nav.select_row(Some(&row));
        }
    }

    pub fn toast(&self, text: &str) {
        let toast = adw::Toast::new(text);
        toast.set_timeout(2);
        self.toast.add_toast(toast);
    }
}

pub fn build(app: &adw::Application, st: &St) -> Window {
    let (w, h) = {
        let c = st.config.borrow();
        (c.width, c.height)
    };
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Voicenux")
        .default_width(w)
        .default_height(h)
        .width_request(980)
        .height_request(620)
        .build();

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(220);

    // Pages that need to send the user elsewhere get this.
    let win_cell: Rc<std::cell::OnceCell<Window>> = Rc::default();

    let lab = voicelab::build(st);
    let open_lab: voices::OpenLab = Rc::new({
        let st = st.clone();
        let lab = lab.clone();
        let win_cell = win_cell.clone();
        move |id: &str| {
            let builtin = st.voices.borrow().iter().find(|v| v.id == id).map(|v| v.builtin).unwrap_or(true);
            let target = if builtin { st.duplicate(id).map(|v| v.id) } else { Some(id.to_string()) };
            if let Some(target) = target {
                st.set_voice(&target);
                lab.open(&target);
                if let Some(w) = win_cell.get() {
                    w.show_page("voicelab");
                    if builtin {
                        w.toast(t("Saved a copy in Voicelab. Change it as you like."));
                    }
                }
            }
        }
    });
    let voices_page = voices::build(st, open_lab);
    stack.add_named(&voices_page.widget, Some("voices"));
    stack.add_named(&soundboard::build(st), Some("soundboard"));
    stack.add_named(&lab.widget, Some("voicelab"));
    stack.add_named(&settings::build(st, &window), Some("settings"));

    // Sidebar.
    let nav = gtk::ListBox::new();
    nav.add_css_class("navigation-sidebar");
    nav.add_css_class("vn-nav");
    for (_, icon, title) in PAGES {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.append(&gtk::Image::from_icon_name(icon));
        row.append(&gtk::Label::new(Some(t_dyn(title).as_str())));
        nav.append(&row);
    }
    nav.connect_row_selected({
        let stack = stack.clone();
        move |_, row| {
            if let Some(row) = row {
                stack.set_visible_child_name(PAGES[row.index() as usize].0);
            }
        }
    });

    let brand = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    brand.append(&logo(28));
    brand.append(&label("Voicenux", &["vn-brand"]));
    let side_header = adw::HeaderBar::new();
    side_header.set_title_widget(Some(&brand));

    let side = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side.append(&nav);
    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    side.append(&spacer);
    side.append(&now_card(st));

    let side_view = adw::ToolbarView::new();
    side_view.add_top_bar(&side_header);
    side_view.set_content(Some(&side));
    side_view.add_css_class("vn-sidebar");

    let banner = adw::Banner::new("");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&banner);
    stack.set_vexpand(true);
    content.append(&stack);

    let split = adw::OverlaySplitView::new();
    split.set_sidebar(Some(&side_view));
    split.set_content(Some(&content));
    split.set_min_sidebar_width(200.0);
    split.set_max_sidebar_width(220.0);
    split.set_vexpand(true);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&split);
    root.append(&bottombar::build(st));

    let toast = adw::ToastOverlay::new();
    toast.set_child(Some(&root));
    window.set_content(Some(&toast));

    let win = Window { window: window.clone(), toast, stack, nav, lab: lab.clone(), st: st.clone() };
    let _ = win_cell.set(win.clone());
    win.show_page("voices");

    // Typing anywhere searches voices.
    voices_page.search.set_key_capture_widget(Some(&voices_page.widget));

    window.add_tick_callback({
        let st = st.clone();
        move |_, clock| {
            st.poll_levels(clock.frame_time());
            glib::ControlFlow::Continue
        }
    });

    window.connect_close_request({
        let st = st.clone();
        move |w| {
            let (width, height) = (w.width(), w.height());
            {
                let mut c = st.config.borrow_mut();
                c.width = width;
                c.height = height;
            }
            st.save_now();
            if st.config.borrow().close_to_tray {
                w.set_visible(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });

    st.subscribe({
        let win = win.clone();
        let banner = banner.clone();
        move |c| match c {
            Change::Notice(text) => {
                if win.window.is_visible() {
                    win.toast(text);
                }
            }
            Change::Engine(err) => match err {
                Some(e) => {
                    banner.set_title(&crate::i18n::fill(t("The audio engine stopped: {}. Is PipeWire running?"), &[e]));
                    banner.set_revealed(true);
                }
                None => banner.set_revealed(false),
            },
            _ => {}
        }
    });
    win
}

/// The current voice, small, at the bottom of the sidebar.
fn now_card(st: &St) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    card.add_css_class("vn-now");
    let img = gtk::Image::new();
    img.set_pixel_size(40);
    img.add_css_class("vn-now-icon");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    let small = label(t("Your voice"), &["caption", "dim-label"]);
    small.set_xalign(0.0);
    let name = label("", &["vn-now-title"]);
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&small);
    text.append(&name);
    card.append(&img);
    card.append(&text);
    let sync = {
        let st = st.clone();
        let card = card.clone();
        move || {
            let v = st.current();
            let on = st.config.borrow().voice_on;
            img.set_paintable(Some(&super::texture(if on { &v.icon } else { "microphone" })));
            name.set_label(&if on { t_dyn(&v.name) } else { t("Your real voice").to_string() });
            for c in crate::voice::CATEGORIES {
                card.remove_css_class(&super::cat_class(c.id));
            }
            card.add_css_class(&super::cat_class(&v.category));
        }
    };
    sync();
    st.subscribe(move |c| {
        if matches!(c, Change::Voice | Change::Toggles | Change::VoiceEdited(_) | Change::Voices) {
            sync();
        }
    });
    card
}
