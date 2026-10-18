//! The main window: sidebar, pages and the bottom bar.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

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
    voice_search: gtk::SearchEntry,
    sound_search: gtk::SearchEntry,
    add_sounds: Rc<dyn Fn()>,
    pub find_online: Rc<dyn Fn(&str)>,
    st: St,
}

/// The pages: id, Lucide icon, title and the color of the icon's tile.
pub const PAGES: &[(&str, &str, &str)] = &[
    ("voices", "mic", "Voices"),
    ("soundboard", "volume-2", "Soundboard"),
    ("voicelab", "flask-conical", "Voicelab"),
    ("settings", "settings", "Settings"),
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
        mark_page(&self.nav, name);
    }

    pub fn toast(&self, text: &str) {
        let toast = adw::Toast::new(text);
        toast.set_timeout(2);
        self.toast.add_toast(toast);
    }

    /// Keys that work on every page. Menus and dialogs keep their own keys.
    fn key(&self, event: Option<gdk::Event>, key: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        if self.window.visible_dialog().is_some() || event.and_then(|e| e.surface()) != self.window.surface() {
            return glib::Propagation::Proceed;
        }
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let st = &self.st;
        if key == gdk::Key::F1 && !ctrl {
            super::help::show(&self.window);
            return glib::Propagation::Stop;
        }
        if !ctrl || mods.contains(gdk::ModifierType::ALT_MASK) {
            return glib::Propagation::Proceed;
        }
        let page = self.stack.visible_child_name().unwrap_or_default();
        match key.to_lower() {
            gdk::Key::f => {
                if page == "soundboard" {
                    self.sound_search.grab_focus();
                } else {
                    self.show_page("voices");
                    self.voice_search.grab_focus();
                }
            }
            gdk::Key::_1 | gdk::Key::KP_1 => self.show_page("voices"),
            gdk::Key::_2 | gdk::Key::KP_2 => self.show_page("soundboard"),
            gdk::Key::_3 | gdk::Key::KP_3 => self.show_page("voicelab"),
            gdk::Key::_4 | gdk::Key::KP_4 | gdk::Key::comma => self.show_page("settings"),
            gdk::Key::t => {
                let on = !st.config.borrow().voice_on;
                st.set_voice_on(on);
                self.toast(if st.config.borrow().voice_on { t("Voice changer on") } else { t("Voice changer off") });
            }
            gdk::Key::h => {
                let on = !st.config.borrow().hear_myself;
                st.set_hear_myself(on);
                self.toast(if st.config.borrow().hear_myself { t("Hear myself on") } else { t("Hear myself off") });
            }
            gdk::Key::b => {
                let on = !st.config.borrow().background;
                st.set_background(on);
                self.toast(if st.config.borrow().background { t("Background on") } else { t("Background off") });
            }
            gdk::Key::m => {
                st.set_muted(!st.muted.get());
                self.toast(if st.muted.get() { t("Microphone muted") } else { t("Microphone on") });
            }
            gdk::Key::r => {
                st.random_voice();
                self.toast(&crate::i18n::fill(t("Voice: {}"), &[&t_dyn(&st.current().name)]));
            }
            gdk::Key::o => {
                self.show_page("soundboard");
                (self.add_sounds)();
            }
            gdk::Key::n => {
                self.show_page("voicelab");
                self.lab.new_voice();
            }
            gdk::Key::question | gdk::Key::slash => super::help::show(&self.window),
            // Brings back the sound removed last, while its note is up.
            gdk::Key::z if page == "soundboard" => {
                if !st.undo_last_remove() {
                    return glib::Propagation::Proceed;
                }
            }
            gdk::Key::w => self.window.close(),
            gdk::Key::q => {
                if let Some(app) = self.window.application() {
                    self.window.close();
                    app.quit();
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
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
        .maximized(st.config.borrow().maximized)
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
    let sounds_page = soundboard::build(st);
    stack.add_named(&voices_page.widget, Some("voices"));
    stack.add_named(&sounds_page.widget, Some("soundboard"));
    stack.add_named(&lab.widget, Some("voicelab"));
    stack.add_named(&settings::build(st, &window), Some("settings"));

    // Sidebar.
    let nav = gtk::ListBox::new();
    nav.add_css_class("navigation-sidebar");
    nav.add_css_class("vn-nav");
    for (i, (_, pic, title)) in PAGES.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.append(&super::line_icon(pic, 20));
        row.append(&gtk::Label::new(Some(t_dyn(title).as_str())));
        nav.append(&row);
        if let Some(r) = nav.row_at_index(i as i32) {
            r.set_tooltip_text(Some(&format!("{} ({}+{})", t_dyn(title), t("Ctrl"), i + 1)));
        }
    }
    // Only a click or Enter on a page goes there. A selection follows the focus, and libadwaita
    // moves the focus on when a breakpoint changes, like on maximize: that stepped through the
    // pages. So the list selects nothing and marks the page itself.
    nav.set_selection_mode(gtk::SelectionMode::None);
    nav.connect_row_activated({
        let stack = stack.clone();
        move |nav, row| {
            let name = PAGES[row.index() as usize].0;
            // A click on the page you are on only makes its icon jump.
            if stack.visible_child_name().as_deref() == Some(name) {
                jump(row);
            }
            stack.set_visible_child_name(name);
            mark_page(nav, name);
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
    side.append(&now_card(st, {
        let win_cell = win_cell.clone();
        let show = voices_page.show_current.clone();
        move || {
            if let Some(w) = win_cell.get() {
                w.show_page("voices");
                show();
            }
        }
    }));

    let side_view = adw::ToolbarView::new();
    side_view.add_top_bar(&side_header);
    side_view.set_content(Some(&side));
    side_view.add_css_class("vn-sidebar");

    let banner = adw::Banner::new("");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("vn-content");
    content.append(&banner);
    stack.set_vexpand(true);
    content.append(&stack);
    // Under the pages, not the sidebar: so its switches line up with the titles and pages above.
    let (bar, bar_texts) = bottombar::build(st);
    content.append(&bar);

    let split = adw::OverlaySplitView::new();
    split.set_sidebar(Some(&side_view));
    split.set_content(Some(&content));
    split.set_min_sidebar_width(200.0);
    split.set_max_sidebar_width(220.0);
    split.set_vexpand(true);

    // Room for things that float above the pages, like the autoscroll mark.
    let layer = gtk::Overlay::new();
    layer.set_widget_name(super::tour::LAYER);
    layer.set_child(Some(&split));
    let toast = adw::ToastOverlay::new();
    toast.set_child(Some(&layer));
    window.set_content(Some(&toast));

    // In a narrow window the bottom bar drops its texts, so the switches still fit in the middle.
    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 1060sp").expect("a condition"));
    for w in &bar_texts {
        narrow.add_setter(w, "visible", Some(&false.to_value()));
    }
    for c in &sounds_page.shrink {
        narrow.add_setter(c, "label", Some(&"".to_value()));
    }
    window.add_breakpoint(narrow);

    super::popup::watch(&window);

    let win = Window {
        window: window.clone(),
        toast,
        stack,
        nav,
        lab: lab.clone(),
        voice_search: voices_page.search.clone(),
        sound_search: sounds_page.search.clone(),
        add_sounds: sounds_page.add.clone(),
        find_online: sounds_page.find_online.clone(),
        st: st.clone(),
    };
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let win = win.clone();
        move |c, key, _, mods| win.key(c.current_event(), key, mods)
    });
    window.add_controller(keys);
    let _ = win_cell.set(win.clone());
    // Opens on the page you left it on.
    let last = st.config.borrow().page.clone();
    win.show_page(if PAGES.iter().any(|p| p.0 == last) { &last } else { "voices" });
    history(&win);
    win.stack.connect_visible_child_name_notify({
        let st = st.clone();
        move |s| {
            let name = s.visible_child_name().unwrap_or_default().to_string();
            if st.config.borrow().page != name {
                st.config.borrow_mut().page = name;
                st.save_later();
            }
        }
    });

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
            remember_size(w, &st);
            st.save_now();
            if st.runs_in_tray() {
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
            Change::SoundRemoved { ids, title } => {
                let toast = adw::Toast::builder().title(title).button_label(t("Undo")).timeout(6).build();
                // Back in the other order, so each one finds its old place again.
                toast.connect_button_clicked({
                    let (st, ids) = (win.st.clone(), ids.clone());
                    move |_| {
                        for id in ids.iter().rev() {
                            st.undo_remove(id);
                        }
                    }
                });
                toast.connect_dismissed({
                    let (st, ids) = (win.st.clone(), ids.clone());
                    move |_| {
                        for id in &ids {
                            st.forget_removed(id);
                        }
                    }
                });
                win.toast.add_toast(toast);
            }
            Change::Engine(err) => match err {
                Some(e) => {
                    banner.set_title(&crate::i18n::fill(t("The audio engine stopped: {}. Is PipeWire running?"), &[e]));
                    banner.set_revealed(true);
                }
                None => banner.set_revealed(false),
            },
            Change::AiMissing(id) => {
                win.window.present();
                super::ai::offer(&win.st, &win.window, id);
            }
            _ => {}
        }
    });
    win
}

/// The back and forward buttons of the mouse go through the pages you saw, like in a browser.
fn history(win: &Window) {
    #[derive(Default)]
    struct History {
        back: Vec<String>,
        forward: Vec<String>,
        current: String,
        /// Going back or forward right now, so the page change is not a new step.
        stepping: bool,
    }
    let h: Rc<std::cell::RefCell<History>> = Rc::default();
    h.borrow_mut().current = win.stack.visible_child_name().unwrap_or_default().to_string();
    win.stack.connect_visible_child_name_notify({
        let h = h.clone();
        move |s| {
            let name = s.visible_child_name().unwrap_or_default().to_string();
            let mut h = h.borrow_mut();
            if name == h.current {
                return;
            }
            let old = std::mem::replace(&mut h.current, name);
            if !h.stepping {
                h.back.push(old);
                h.forward.clear();
            }
        }
    });
    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let window = win.window.clone();
    let win = win.clone();
    click.connect_pressed(move |g, _, _, _| {
        let back = match g.current_button() {
            8 => true,
            9 => false,
            _ => {
                g.set_state(gtk::EventSequenceState::Denied);
                return;
            }
        };
        g.set_state(gtk::EventSequenceState::Claimed);
        if win.window.visible_dialog().is_some() {
            return;
        }
        // In a folder, back goes up out of it first.
        if back && win.stack.visible_child().is_some_and(|page| super::board::up_on(&page)) {
            return;
        }
        let next = {
            let h = &mut *h.borrow_mut();
            let (from, to) = if back { (&mut h.back, &mut h.forward) } else { (&mut h.forward, &mut h.back) };
            let next = from.pop();
            if next.is_some() {
                to.push(h.current.clone());
            }
            next
        };
        if let Some(page) = next {
            h.borrow_mut().stepping = true;
            win.show_page(&page);
            h.borrow_mut().stepping = false;
        }
    });
    window.add_controller(click);
}

/// The icon of a page jumps when you pick it, also by a key, and on a click on the page you are
/// on. A new animation name starts it again, the same name would not.
fn jump(row: &gtk::ListBoxRow) {
    let a = !row.has_css_class("vn-pick-a");
    row.remove_css_class(if a { "vn-pick-b" } else { "vn-pick-a" });
    row.add_css_class(if a { "vn-pick-a" } else { "vn-pick-b" });
}

/// Marks the page you are on in the sidebar, for the eye and for screen readers.
fn mark_page(nav: &gtk::ListBox, name: &str) {
    for (i, (id, ..)) in PAGES.iter().enumerate() {
        let Some(row) = nav.row_at_index(i as i32) else { continue };
        let on = *id == name;
        if on && !row.has_css_class("vn-current") {
            jump(&row);
        }
        if on {
            row.add_css_class("vn-current");
        } else {
            row.remove_css_class("vn-current");
        }
        row.update_state(&[gtk::accessible::State::Selected(Some(on))]);
    }
}

/// Keeps the size of the window for the next start: its size when not maximized, and whether it
/// is. Every way to quit calls this, not only closing the window.
pub fn remember_size(w: &impl IsA<gtk::Window>, st: &St) {
    let (width, height) = w.default_size();
    let mut c = st.config.borrow_mut();
    if width > 0 && height > 0 {
        c.width = width;
        c.height = height;
    }
    c.maximized = w.is_maximized();
}

/// The current voice, small, at the bottom of the sidebar. Gone while the voice changer is off.
fn now_card(st: &St, on_click: impl Fn() + 'static) -> gtk::Revealer {
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
    // A click shows the voice on the voice page.
    let button = gtk::Button::builder().child(&card).css_classes(["flat", "vn-now-button"]).build();
    button.set_tooltip_text(Some(t("Show the voice")));
    button.connect_clicked(move |_| on_click());
    let reveal = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .transition_duration(260)
        .child(&button)
        .build();
    let sync = {
        let st = st.clone();
        let card = card.clone();
        let reveal = reveal.clone();
        move || {
            let v = st.current();
            reveal.set_reveal_child(st.config.borrow().voice_on);
            img.set_paintable(Some(&super::texture(&v.icon)));
            name.set_label(&t_dyn(&v.name));
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
    reveal
}
