//! The soundboard page: big pads that play sounds into your microphone.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use super::visuals::progress_ring;
use super::{animate_in, popup, clear, icon, icon_picker, label, smooth_scroll, value_field};
use crate::i18n::{fill, t, t_dyn};
use crate::soundboard::{PlayMode, Sound};
use crate::state::{Change, St};

struct Pad {
    child: gtk::FlowBoxChild,
    pad: gtk::Box,
    progress: Rc<Cell<f32>>,
    ring: gtk::DrawingArea,
    /// The shortcut of the sound, as a key cap.
    keys: gtk::Label,
}

pub struct SoundboardPage {
    pub widget: adw::ToolbarView,
    pub search: gtk::SearchEntry,
    /// Opens the file chooser to add sounds.
    pub add: Rc<dyn Fn()>,
    /// Header buttons that drop their text in a narrow window.
    pub shrink: Vec<adw::ButtonContent>,
    /// The text of the play mode button, which goes first when the header gets full.
    pub mode_label: gtk::Label,
    /// Opens the MyInstants search with some words.
    pub find_online: Rc<dyn Fn(&str)>,
}

pub fn build(st: &St) -> SoundboardPage {
    let header = adw::HeaderBar::new();
    // The tools sit on the left, in line with the page, and the rest of the bar is for dragging.
    header.set_show_title(false);
    let (search_box, search) = super::search_field(t("Search sounds"));
    search_box.set_margin_start(12);
    header.pack_start(&search_box);
    let content = |icon: &str, text: &str| adw::ButtonContent::builder().icon_name(icon).label(text).build();
    let (find_content, stop_content) =
        (content("system-search-symbolic", t("Find sounds")), content("media-playback-stop-symbolic", t("Stop all")));
    // MyInstants has all the sounds people want, so finding them is the main thing here. Own
    // files are in the More menu, and can be dropped on the page.
    let find = gtk::ToggleButton::new();
    find.set_child(Some(&find_content));
    find.add_css_class("suggested-action");
    find.set_tooltip_text(Some(t("Find sounds on MyInstants")));
    header.pack_start(&find);
    let add: Rc<dyn Fn()> = Rc::new({
        let (st, header) = (st.clone(), header.downgrade());
        move || {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(t("Audio files")));
            filter.add_mime_type("audio/*");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder().title(t("Add sounds")).filters(&filters).modal(true).build();
            let st = st.clone();
            let parent = header.upgrade().and_then(|h| h.root()).and_downcast::<gtk::Window>();
            dialog.open_multiple(parent.as_ref(), None::<&gio::Cancellable>, move |res| {
                if let Ok(files) = res {
                    let paths: Vec<_> = files.iter::<gio::File>().flatten().filter_map(|f| f.path()).collect();
                    import(&st, &paths);
                }
            });
        }
    });
    let stop = gtk::Button::new();
    stop.set_child(Some(&stop_content));
    stop.set_tooltip_text(Some(t("Stop all sounds (Esc)")));
    header.pack_end(&stop);
    header.pack_end(&more_button(st, add.clone()));
    header.pack_end(&volume_button(st));
    let (mode, mode_label) = mode_button(st);
    header.pack_end(&mode);
    stop.connect_clicked({
        let st = st.clone();
        move |_| st.stop_sounds()
    });

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
        .margin_top(16)
        .margin_bottom(24)
        .activate_on_single_click(true)
        .build();
    grid.add_css_class("vn-pads");
    let scroll =
        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&grid).build();
    smooth_scroll(&scroll);

    let empty = adw::StatusPage::builder()
        .paintable(&super::texture("speaker-high-volume"))
        .title(t("No sounds yet"))
        .description(t("Add audio files, or drop them here. MP3, OGG, FLAC and WAV work."))
        .build();
    let restore = gtk::Button::with_label(t("Bring back the starter sounds"));
    restore.add_css_class("pill");
    restore.set_halign(gtk::Align::Center);
    empty.set_child(Some(&restore));
    restore.connect_clicked({
        let st = st.clone();
        move |_| st.restore_starter_sounds()
    });

    // Nothing matches the search: maybe MyInstants has it.
    let none = adw::StatusPage::builder()
        .paintable(&super::load_picture("magnifying-glass-tilted-left"))
        .title(t("No sounds found"))
        .build();
    let ask_online = gtk::Button::new();
    ask_online.add_css_class("vn-gradient-button");
    ask_online.set_halign(gtk::Align::Center);
    none.set_child(Some(&ask_online));

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.add_named(&scroll, Some("grid"));
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&none, Some("none"));

    let hint = label(
        t("Sounds play into the Voicenux Microphone. Drop audio files anywhere on this page to add them."),
        &["dim-label", "caption"],
    );
    hint.set_wrap(true);
    hint.set_margin_bottom(10);
    hint.set_margin_start(20);
    hint.set_margin_end(20);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.set_hexpand(true);
    body.append(&stack);
    body.append(&hint);

    // Finding sounds online slides in on the right, so new sounds show up in the grid beside it.
    let (finder, online) = super::find_sounds::panel(st);
    let reveal = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideLeft)
        .transition_duration(260)
        .child(&finder)
        .build();
    find.connect_toggled({
        let st = st.clone();
        let reveal = reveal.clone();
        let online = online.clone();
        move |b| {
            reveal.set_reveal_child(b.is_active());
            if b.is_active() {
                online.grab_focus();
            } else {
                st.stop_preview();
            }
        }
    });
    // Escape empties the search, a second one closes the panel.
    online.connect_stop_search({
        let find = find.clone();
        move |e| {
            if e.text().is_empty() {
                find.set_active(false);
            } else {
                e.set_text("");
            }
        }
    });
    ask_online.connect_clicked({
        let (find, online, search) = (find.clone(), online.clone(), search.clone());
        move |_| {
            online.set_text(search.text().trim());
            if find.is_active() {
                online.grab_focus();
            } else {
                find.set_active(true);
            }
        }
    });
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.append(&body);
    content.append(&reveal);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&content));

    let pads: Rc<RefCell<HashMap<String, Pad>>> = Rc::default();

    // Shows the pads whose name has all words of the search.
    let refilter = {
        let (st, pads, stack, search, none, ask_online) =
            (st.clone(), pads.clone(), stack.clone(), search.clone(), none.clone(), ask_online.clone());
        Rc::new(move || {
            let query = search.text().trim().to_string();
            let words: Vec<String> = super::fold(&query).split_whitespace().map(String::from).collect();
            let lib = st.library.borrow();
            let mut shown = 0;
            for s in &lib.sound {
                let Some(p) = pads.borrow().get(&s.id).map(|p| p.child.clone()) else { continue };
                let name = super::fold(&format!("{} {}", t_dyn(&s.name), s.name));
                let visible = words.iter().all(|w| name.contains(w.as_str()));
                p.set_visible(visible);
                shown += visible as usize;
            }
            none.set_description(Some(&fill(t("Nothing on your soundboard is called “{}”."), &[&query])));
            ask_online.set_label(&fill(t("Search MyInstants for “{}”"), &[&query]));
            stack.set_visible_child_name(if lib.sound.is_empty() {
                "empty"
            } else if shown == 0 {
                "none"
            } else {
                "grid"
            });
        })
    };

    let rebuild = {
        let (st, grid, pads, refilter) = (st.clone(), grid.clone(), pads.clone(), refilter.clone());
        Rc::new(move |animate: bool| {
            // Keeps the focus on the same sound, or the one that took its place.
            let focused = grid.focus_child().and_then(|c| {
                let pads = pads.borrow();
                let id = pads.iter().find(|(_, p)| p.child == c).map(|(id, _)| id.clone())?;
                Some((id, c.downcast_ref::<gtk::FlowBoxChild>()?.index()))
            });
            clear(&grid);
            pads.borrow_mut().clear();
            let sounds = st.library.borrow().sound.clone();
            for (i, s) in sounds.iter().enumerate() {
                let p = pad(&st, s);
                grid.append(&p.child);
                if animate {
                    animate_in(&p.child, i);
                }
                pads.borrow_mut().insert(s.id.clone(), p);
            }
            refilter();
            if let Some((id, index)) = focused {
                let child = pads.borrow().get(&id).map(|p| p.child.clone()).or_else(|| grid.child_at_index(index.min(sounds.len() as i32 - 1)));
                if let Some(c) = child {
                    c.grab_focus();
                }
            }
        })
    };
    rebuild(false);
    search.connect_search_changed({
        let refilter = refilter.clone();
        move |_| refilter()
    });

    // Delete removes the focused sound, Alt with the arrows moves it. Only when the pad itself has
    // the focus: keys in its menu, like Delete in the name field, belong to the menu.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let (st, grid, pads) = (st.clone(), grid.clone(), pads.clone());
        move |_, key, _, mods| {
            let Some(child) = grid.focus_child().filter(|c| c.has_focus()) else { return glib::Propagation::Proceed };
            let Some(id) = pads.borrow().iter().find(|(_, p)| p.child == child).map(|(id, _)| id.clone()) else {
                return glib::Propagation::Proceed;
            };
            let alt = mods.contains(gdk::ModifierType::ALT_MASK);
            match key {
                gdk::Key::Delete | gdk::Key::KP_Delete => st.remove_sound(&id),
                gdk::Key::Left | gdk::Key::KP_Left if alt => st.shift_sound(&id, false),
                gdk::Key::Right | gdk::Key::KP_Right if alt => st.shift_sound(&id, true),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });
    grid.add_controller(keys);

    grid.connect_child_activated({
        let st = st.clone();
        let pads = pads.clone();
        move |_, child| {
            let id = pads.borrow().iter().find(|(_, p)| &p.child == child).map(|(id, _)| id.clone());
            if let Some(id) = id {
                st.play_sound(&id);
            }
        }
    });

    // Progress rings, once per frame while the page is on screen.
    view.add_tick_callback({
        let st = st.clone();
        let pads = pads.clone();
        move |w, _| {
            if !w.is_mapped() {
                return glib::ControlFlow::Continue;
            }
            let playing = st.engine.meters.playing();
            let mut now: HashMap<String, f32> = HashMap::new();
            for (rt, p) in playing {
                if let Some(id) = st.sound_id_of(rt) {
                    let e = now.entry(id).or_insert(0.0);
                    *e = e.max(p.max(0.001));
                }
            }
            for (id, p) in pads.borrow().iter() {
                let v = now.get(id).copied().unwrap_or(0.0);
                if v > 0.0 {
                    p.pad.add_css_class("playing");
                } else {
                    p.pad.remove_css_class("playing");
                }
                if (p.progress.get() - v).abs() > 1e-4 {
                    p.progress.set(v);
                    p.ring.queue_draw();
                }
            }
            glib::ControlFlow::Continue
        }
    });

    let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    drop.connect_drop({
        let st = st.clone();
        let body = body.clone();
        move |_, value, _, _| {
            body.remove_css_class("vn-drop");
            let Ok(list) = value.get::<gdk::FileList>() else { return false };
            let paths: Vec<_> = list.files().iter().filter_map(|f| f.path()).collect();
            import(&st, &paths);
            true
        }
    });
    drop.connect_enter({
        let body = body.clone();
        move |_, _, _| {
            body.add_css_class("vn-drop");
            gdk::DragAction::COPY
        }
    });
    drop.connect_leave({
        let body = body.clone();
        move |_| body.remove_css_class("vn-drop")
    });
    view.add_controller(drop);

    // Escape stops all sounds, when nothing else wants it.
    let escape = gtk::EventControllerKey::new();
    escape.connect_key_pressed({
        let st = st.clone();
        move |_, key, _, mods| {
            if key == gdk::Key::Escape && mods.is_empty() {
                st.stop_sounds();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    view.add_controller(escape);
    // Typing anywhere searches your sounds.
    search.set_key_capture_widget(Some(&view));

    st.subscribe({
        let (st, pads) = (st.clone(), pads.clone());
        move |c| match c {
            Change::Sounds => rebuild(false),
            Change::Settings => {
                for (id, p) in pads.borrow().iter() {
                    show_keys(&st, &p.keys, id);
                }
            }
            _ => {}
        }
    });
    let find_online: Rc<dyn Fn(&str)> = Rc::new({
        let (find, online) = (find.clone(), online.clone());
        move |text: &str| {
            find.set_active(true);
            online.set_text(text);
        }
    });
    SoundboardPage { widget: view, search, add, shrink: vec![find_content, stop_content], mode_label, find_online }
}

/// What a click on a sound does, for all sounds at once. Sounds with a mode of their own keep it.
fn mode_button(st: &St) -> (gtk::MenuButton, gtk::Label) {
    // An icon and a label of its own, not a ButtonContent: a narrow window hides the label, and
    // a new mode only changes its text.
    let (icon, text) = (gtk::Image::new(), gtk::Label::new(None));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&icon);
    content.append(&text);
    let b = gtk::MenuButton::new();
    b.set_child(Some(&content));
    b.set_always_show_arrow(true);
    b.set_tooltip_text(Some(t("What a click on a sound does")));
    let items = PlayMode::BOARD
        .iter()
        .map(|&m| {
            let st = st.clone();
            popup::Item::new(m.icon(), t_dyn(m.label()), move || st.set_sound_mode(m))
        })
        .collect();
    b.set_popover(Some(&popup::list(items)));
    let sync = {
        let (st, icon, text) = (st.clone(), icon.clone(), text.clone());
        move || {
            let m = st.config.borrow().sound_mode;
            icon.set_icon_name(Some(m.icon()));
            text.set_label(&t_dyn(m.label()));
        }
    };
    sync();
    st.subscribe(move |c| {
        if matches!(c, Change::Settings) {
            sync();
        }
    });
    (b, text)
}

/// More: add own files, sort the sounds, or bring back the starter sounds.
fn more_button(st: &St, add: Rc<dyn Fn()>) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    b.set_icon_name("view-more-symbolic");
    b.set_tooltip_text(Some(t("More")));
    let (st1, st2) = (st.clone(), st.clone());
    b.set_popover(Some(&popup::list(vec![
        popup::Item::new("list-add-symbolic", t("Add audio files"), move || add()),
        popup::Item::new("view-sort-ascending-symbolic", t("Sort by name"), move || st1.sort_sounds()),
        popup::Item::new("edit-undo-symbolic", t("Bring back the starter sounds"), move || st2.restore_starter_sounds()),
    ])));
    b
}

fn import(st: &St, paths: &[std::path::PathBuf]) {
    let failed = st.import_sounds(paths);
    if !failed.is_empty() {
        st.notify(Change::Notice(fill(t("Could not add {}"), &[&failed.join(", ")])));
    }
}

fn volume_button(st: &St) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    b.set_icon_name("audio-volume-high-symbolic");
    b.set_tooltip_text(Some(t("Soundboard volume")));
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 10);
    bx.set_margin_top(12);
    bx.set_margin_bottom(12);
    bx.set_margin_start(12);
    bx.set_margin_end(12);
    bx.set_width_request(260);
    let title = label(t("Soundboard volume"), &["heading"]);
    title.set_xalign(0.0);
    let scale = volume_slider(st.config.borrow().sound_volume as f64, 1.0, {
        let st = st.clone();
        move |v| st.update(|c| c.sound_volume = v as f32)
    });
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let l = label(t("Hear sounds myself"), &[]);
    l.set_hexpand(true);
    l.set_xalign(0.0);
    let sw = gtk::Switch::new();
    sw.set_active(st.config.borrow().hear_sounds);
    sw.connect_active_notify({
        let st = st.clone();
        move |s| st.update(|c| c.hear_sounds = s.is_active())
    });
    row.append(&l);
    row.append(&sw);
    bx.append(&title);
    bx.append(&scale);
    bx.append(&row);
    let p = gtk::Popover::new();
    popup::menu(&p);
    p.set_child(Some(&bx));
    b.set_popover(Some(&p));
    b
}

/// A volume slider with a speaker that shows the level, and the value in percent. Click the value
/// to type one. It holds at 50 %, 100 % and 150 % on the way, so those are easy to hit.
fn volume_slider(value: f64, max: f64, on_change: impl Fn(f64) + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("vn-volume");
    let speaker = gtk::Image::new();
    speaker.add_css_class("vn-volume-icon");
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, max, 0.05);
    scale.set_hexpand(true);
    scale.set_value(value);
    super::notches(&scale, &[0.5, 1.0, 1.5]);
    let text = |v: f64| format!("{:.0} %", v * 100.0);
    let field = value_field(
        {
            let scale = scale.clone();
            move || format!("{:.0}", scale.value() * 100.0)
        },
        {
            let scale = scale.clone();
            move |f, typed_text| {
                if let Some(n) = super::typed_number(typed_text, &["%"]) {
                    scale.set_value((n / 100.0).clamp(0.0, max));
                }
                // Also puts the old value back after something that is not a number.
                f.set_text(&text(scale.value()));
            }
        },
    );
    field.add_css_class("vn-volume-value");
    field.set_width_chars(5);
    field.set_max_width_chars(5);
    let show = {
        let (speaker, field) = (speaker.clone(), field.clone());
        move |v: f64| {
            speaker.set_icon_name(Some(match v {
                v if v < 0.005 => "audio-volume-muted-symbolic",
                v if v < 0.4 => "audio-volume-low-symbolic",
                v if v < 0.8 => "audio-volume-medium-symbolic",
                _ => "audio-volume-high-symbolic",
            }));
            field.set_text(&text(v));
        }
    };
    show(value);
    scale.connect_value_changed(move |s| {
        let v = s.value();
        show(v);
        on_change(v);
    });
    row.append(&speaker);
    row.append(&scale);
    row.append(&field);
    row
}

fn pad(st: &St, s: &Sound) -> Pad {
    let pad = gtk::Box::new(gtk::Orientation::Vertical, 8);
    pad.add_css_class("sound-pad");
    pad.set_size_request(136, 140);

    let progress = Rc::new(Cell::new(0.0f32));
    let ring = progress_ring(progress.clone());
    let pic = gtk::Overlay::new();
    pic.set_child(Some(&ring));
    let img = icon(&s.icon, 58);
    img.add_css_class("vn-pad-icon");
    img.set_halign(gtk::Align::Center);
    img.set_valign(gtk::Align::Center);
    pic.add_overlay(&img);
    pic.set_halign(gtk::Align::Center);
    pad.append(&pic);
    let name = label(&t_dyn(&s.name), &["vn-pad-name"]);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_max_width_chars(14);
    pad.append(&name);
    let keys = label("", &["vn-key", "vn-pad-keys"]);
    keys.set_halign(gtk::Align::Center);
    keys.set_ellipsize(gtk::pango::EllipsizeMode::End);
    keys.set_max_width_chars(16);
    show_keys(st, &keys, &s.id);
    pad.append(&keys);

    // Little marks for how it plays and whether it is cut.
    let badges = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    badges.set_halign(gtk::Align::Start);
    badges.set_valign(gtk::Align::Start);
    badges.set_margin_top(8);
    badges.set_margin_start(8);
    // Only a mode of its own shows: the soundboard button says how the others play.
    let own = s.mode.filter(|m| *m != st.config.borrow().sound_mode);
    let marks = [
        (own.map(PlayMode::icon), own.map(|m| t_dyn(m.label())).unwrap_or_default()),
        (s.is_trimmed().then_some("edit-cut-symbolic"), t("Trimmed").to_string()),
    ];
    for (name, tip) in marks {
        if let Some(name) = name {
            let img = gtk::Image::from_icon_name(name);
            img.add_css_class("vn-pad-badge");
            img.set_tooltip_text(Some(&tip));
            badges.append(&img);
        }
    }

    let menu = gtk::Button::from_icon_name("view-more-symbolic");
    menu.add_css_class("flat");
    menu.add_css_class("vn-pad-menu");
    menu.set_halign(gtk::Align::End);
    menu.set_valign(gtk::Align::Start);
    menu.set_margin_top(6);
    menu.set_margin_end(6);
    menu.set_tooltip_text(Some(t("Edit sound")));

    let overlay = gtk::Overlay::new();
    overlay.add_css_class("vn-lift");
    overlay.set_child(Some(&pad));
    overlay.add_overlay(&menu);
    overlay.add_overlay(&badges);
    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&overlay));

    // The editor hangs on the grid cell, which never lifts or squishes, so it stays still.
    let editor = editor(st, s);
    editor.set_parent(&child);
    popup::keep_clicks(&editor);
    child.connect_destroy({
        let editor = editor.clone();
        move |_| editor.unparent()
    });
    menu.connect_clicked({
        let (editor, child) = (editor.clone(), child.clone());
        move |b| {
            if let Some(r) = b.compute_bounds(&child) {
                let (x, y, w, h) = (r.x() as i32, r.y() as i32, r.width() as i32, r.height() as i32);
                editor.set_pointing_to(Some(&gdk::Rectangle::new(x, y, w, h)));
            }
            editor.popup();
        }
    });
    popup::context(&child, {
        let editor = editor.clone();
        move |x, y| {
            editor.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            editor.popup();
        }
    });
    reorder(st, &child, &pad, &s.id);
    Pad { child, pad, progress, ring, keys }
}

/// Shows the shortcut of a sound on its pad, or nothing.
fn show_keys(st: &St, keys: &gtk::Label, id: &str) {
    let shown = super::keys::shown(st, &format!("sound:{id}"));
    keys.set_label(shown.as_deref().unwrap_or(""));
    keys.set_visible(shown.is_some());
}

/// Drag a pad onto another one to move it there. A glowing bar shows on which side it lands.
fn reorder(st: &St, child: &gtk::FlowBoxChild, pad: &gtk::Box, id: &str) {
    let drag = gtk::DragSource::new();
    drag.set_actions(gdk::DragAction::MOVE);
    // Where the pad was grabbed, so the picture under the pointer hangs from the same spot.
    let grabbed = Rc::new(Cell::new((0.0, 0.0)));
    drag.connect_prepare({
        let (grabbed, id) = (grabbed.clone(), id.to_string());
        move |_, x, y| {
            grabbed.set((x, y));
            Some(gdk::ContentProvider::for_value(&id.to_value()))
        }
    });
    drag.connect_drag_begin({
        let (child, pad) = (child.downgrade(), pad.downgrade());
        move |src, _| {
            if let (Some(child), Some(pad)) = (child.upgrade(), pad.upgrade()) {
                let (x, y) = grabbed.get();
                src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&pad))), x as i32, y as i32);
                child.add_css_class("vn-dragging");
            }
        }
    });
    drag.connect_drag_end({
        let child = child.downgrade();
        move |_, _, _| {
            if let Some(c) = child.upgrade() {
                c.remove_css_class("vn-dragging");
            }
        }
    });
    child.add_controller(drag);

    let side = |c: &gtk::FlowBoxChild, x: f64| {
        let after = x > c.width() as f64 / 2.0;
        c.remove_css_class(if after { "vn-drop-before" } else { "vn-drop-after" });
        c.add_css_class(if after { "vn-drop-after" } else { "vn-drop-before" });
        after
    };
    let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
    drop.connect_motion({
        let child = child.downgrade();
        move |_, x, _| {
            if let Some(c) = child.upgrade() {
                side(&c, x);
            }
            gdk::DragAction::MOVE
        }
    });
    drop.connect_leave({
        let child = child.downgrade();
        move |_| {
            if let Some(c) = child.upgrade() {
                c.remove_css_class("vn-drop-before");
                c.remove_css_class("vn-drop-after");
            }
        }
    });
    drop.connect_drop({
        let (st, child, target) = (st.clone(), child.downgrade(), id.to_string());
        move |_, value, x, _| {
            let (Some(c), Ok(from)) = (child.upgrade(), value.get::<String>()) else { return false };
            let after = side(&c, x);
            c.remove_css_class("vn-drop-before");
            c.remove_css_class("vn-drop-after");
            // After the drop is done: moving rebuilds the grid, with this pad in it.
            glib::idle_add_local_once({
                let (st, target) = (st.clone(), target.clone());
                move || st.move_sound(&from, &target, after)
            });
            true
        }
    });
    child.add_controller(drop);
}

/// Popover to rename a sound, change its picture, volume and how it plays.
fn editor(st: &St, s: &Sound) -> gtk::Popover {
    let sound = Rc::new(RefCell::new(s.clone()));
    let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
    b.set_margin_top(12);
    b.set_margin_bottom(12);
    b.set_margin_start(12);
    b.set_margin_end(12);
    b.set_width_request(280);

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::new();
    entry.set_text(&t_dyn(&s.name));
    entry.set_hexpand(true);
    entry.set_valign(gtk::Align::Center);
    let save = {
        let st = st.clone();
        let sound = sound.clone();
        move || st.update_sound(sound.borrow().clone())
    };
    let save = Rc::new(save);
    top.append(&icon_picker(&s.icon, 32, {
        let sound = sound.clone();
        let save = save.clone();
        move |name| {
            sound.borrow_mut().icon = name.to_string();
            save();
        }
    }));
    top.append(&entry);
    entry.connect_activate({
        let sound = sound.clone();
        let save = save.clone();
        move |e| {
            sound.borrow_mut().name = e.text().to_string();
            save();
        }
    });
    b.append(&top);

    let vol_label = label(t("Volume"), &["caption-heading"]);
    vol_label.set_xalign(0.0);
    b.append(&vol_label);
    b.append(&volume_slider(s.volume as f64, 2.0, {
        let st = st.clone();
        let sound = sound.clone();
        move |v| {
            sound.borrow_mut().volume = v as f32;
            st.set_sound_volume(&sound.borrow().id, v as f32);
        }
    }));

    let mode_label = label(t("When pressed"), &["caption-heading"]);
    mode_label.set_xalign(0.0);
    b.append(&mode_label);
    // First "like the soundboard", then the modes.
    let mut names: Vec<String> = vec![t("Like the soundboard").into()];
    names.extend(PlayMode::ALL.iter().map(|m| t_dyn(m.label())));
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mode = gtk::DropDown::from_strings(&names);
    let own = s.mode.and_then(|m| PlayMode::ALL.iter().position(|a| *a == m));
    mode.set_selected(own.map_or(0, |i| i as u32 + 1));
    popup::menus_in(&mode);
    mode.connect_selected_notify({
        let sound = sound.clone();
        let save = save.clone();
        move |d| {
            sound.borrow_mut().mode = (d.selected() as usize).checked_sub(1).map(|i| PlayMode::ALL[i]);
            save();
        }
    });
    b.append(&mode);

    let trim = gtk::Button::new();
    trim.add_css_class("pill");
    trim.set_child(Some(&adw::ButtonContent::builder().icon_name("edit-cut-symbolic").label(t("Trim sound")).build()));
    b.append(&trim);

    b.append(&super::keys::button(st, &format!("sound:{}", s.id), &t_dyn(&s.name)));

    let remove = gtk::Button::with_label(t("Remove"));
    remove.add_css_class("destructive-action");
    remove.set_margin_top(4);
    remove.connect_clicked({
        let st = st.clone();
        let id = s.id.clone();
        move |_| st.remove_sound(&id)
    });
    b.append(&remove);

    let p = gtk::Popover::new();
    popup::menu(&p);
    p.set_child(Some(&b));
    trim.connect_clicked({
        let (st, p, id) = (st.clone(), p.downgrade(), s.id.clone());
        move |b| {
            let Some(p) = p.upgrade() else { return };
            p.popdown();
            // On the window: the grid is made again when the cut is saved.
            if let Some(window) = b.root() {
                super::trim::open(&st, &id, &window);
            }
        }
    });
    p.connect_closed({
        let sound = sound.clone();
        let entry = entry.clone();
        let save = save.clone();
        move |_| {
            let name = entry.text().to_string();
            let changed = sound.borrow().name != name;
            if changed && !name.trim().is_empty() {
                sound.borrow_mut().name = name;
                save();
            }
        }
    });
    p
}
