//! The soundboard page: big pads that play sounds into your microphone.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use super::visuals::progress_ring;
use super::{animate_in, clear, icon, icon_picker, label, smooth_scroll};
use crate::i18n::{fill, t, t_dyn};
use crate::soundboard::{PlayMode, Sound};
use crate::state::{Change, St};

struct Pad {
    child: gtk::FlowBoxChild,
    pad: gtk::Box,
    progress: Rc<Cell<f32>>,
    ring: gtk::DrawingArea,
}

pub fn build(st: &St) -> adw::ToolbarView {
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(t("Soundboard"), "")));
    let add = gtk::Button::new();
    add.set_child(Some(&adw::ButtonContent::builder().icon_name("list-add-symbolic").label(t("Add sounds")).build()));
    add.add_css_class("suggested-action");
    header.pack_start(&add);
    let stop = gtk::Button::new();
    stop.set_child(Some(
        &adw::ButtonContent::builder().icon_name("media-playback-stop-symbolic").label(t("Stop all")).build(),
    ));
    header.pack_end(&stop);
    header.pack_end(&volume_button(st));
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
        .icon_name("view-app-grid-symbolic")
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

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.add_named(&scroll, Some("grid"));
    stack.add_named(&empty, Some("empty"));

    let hint = label(
        t("Sounds play into the Voicenux Microphone. Drop audio files anywhere on this page to add them."),
        &["dim-label", "caption"],
    );
    hint.set_wrap(true);
    hint.set_margin_bottom(10);
    hint.set_margin_start(20);
    hint.set_margin_end(20);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&stack);
    body.append(&hint);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&body));

    let pads: Rc<RefCell<HashMap<String, Pad>>> = Rc::default();
    let order: Rc<RefCell<Vec<String>>> = Rc::default();

    let rebuild = {
        let st = st.clone();
        let grid = grid.clone();
        let pads = pads.clone();
        let order = order.clone();
        let stack = stack.clone();
        Rc::new(move |animate: bool| {
            clear(&grid);
            pads.borrow_mut().clear();
            order.borrow_mut().clear();
            let sounds = st.library.borrow().sound.clone();
            for (i, s) in sounds.iter().enumerate() {
                let p = pad(&st, s);
                grid.append(&p.child);
                if animate {
                    animate_in(&p.child, i);
                }
                order.borrow_mut().push(s.id.clone());
                pads.borrow_mut().insert(s.id.clone(), p);
            }
            stack.set_visible_child_name(if sounds.is_empty() { "empty" } else { "grid" });
        })
    };
    rebuild(false);

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

    let window_of = |w: &gtk::Widget| w.root().and_downcast::<gtk::Window>();
    add.connect_clicked({
        let st = st.clone();
        move |b| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(t("Audio files")));
            filter.add_mime_type("audio/*");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder().title(t("Add sounds")).filters(&filters).modal(true).build();
            let st = st.clone();
            let parent = window_of(b.upcast_ref());
            dialog.open_multiple(parent.as_ref(), None::<&gio::Cancellable>, move |res| {
                if let Ok(files) = res {
                    let paths: Vec<_> = files.iter::<gio::File>().flatten().filter_map(|f| f.path()).collect();
                    import(&st, &paths);
                }
            });
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

    st.subscribe(move |c| {
        if matches!(c, Change::Sounds) {
            rebuild(false);
        }
    });
    view
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
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    scale.set_value(st.config.borrow().sound_volume as f64);
    scale.connect_value_changed({
        let st = st.clone();
        move |s| st.update(|c| c.sound_volume = s.value() as f32)
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
    p.set_child(Some(&bx));
    b.set_popover(Some(&p));
    b
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

    let menu = gtk::MenuButton::new();
    menu.set_icon_name("view-more-symbolic");
    menu.add_css_class("flat");
    menu.add_css_class("vn-pad-menu");
    menu.set_halign(gtk::Align::End);
    menu.set_valign(gtk::Align::Start);
    menu.set_margin_top(6);
    menu.set_margin_end(6);
    menu.set_tooltip_text(Some(t("Edit sound")));
    menu.set_popover(Some(&editor(st, s)));

    let overlay = gtk::Overlay::new();
    overlay.add_css_class("vn-lift");
    overlay.set_child(Some(&pad));
    overlay.add_overlay(&menu);
    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&overlay));
    Pad { child, pad, progress, ring }
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
    let vol = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 2.0, 0.01);
    vol.set_value(s.volume as f64);
    vol.add_mark(1.0, gtk::PositionType::Bottom, None);
    vol.connect_value_changed({
        let st = st.clone();
        let sound = sound.clone();
        move |v| {
            sound.borrow_mut().volume = v.value() as f32;
            st.set_sound_volume(&sound.borrow().id, v.value() as f32);
        }
    });
    b.append(&vol);

    let mode_label = label(t("When pressed"), &["caption-heading"]);
    mode_label.set_xalign(0.0);
    b.append(&mode_label);
    let names: Vec<String> = PlayMode::ALL.iter().map(|m| t_dyn(m.label())).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mode = gtk::DropDown::from_strings(&names);
    mode.set_selected(PlayMode::ALL.iter().position(|m| *m == s.mode).unwrap_or(0) as u32);
    mode.connect_selected_notify({
        let sound = sound.clone();
        let save = save.clone();
        move |d| {
            sound.borrow_mut().mode = PlayMode::ALL[d.selected() as usize];
            save();
        }
    });
    b.append(&mode);

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
    p.set_child(Some(&b));
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
