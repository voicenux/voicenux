//! Voicelab: build your own voices from effect blocks.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::voices::param_row;
use super::{bounce, clear, icon, icon_picker, label, smooth_scroll};
use crate::dsp::blocks::{self, BLOCKS, Group};
use crate::i18n::{fill, t, t_dyn};
use crate::state::{Change, St};
use crate::voice::{BlockDef, Voice};

type Opener = Rc<dyn Fn(&str)>;
/// A change to the list of blocks of a voice.
type Restructure = Box<dyn FnOnce(&mut Vec<BlockDef>)>;

thread_local! {
    /// Opens a voice in the editor, for buttons deep inside it.
    static OPEN: RefCell<Option<Opener>> = RefCell::new(None);
}

#[derive(Clone)]
pub struct Lab {
    pub widget: adw::ToolbarView,
    open: Rc<dyn Fn(&str)>,
}

impl Lab {
    pub fn open(&self, id: &str) {
        (self.open)(id)
    }
}

pub fn build(st: &St) -> Lab {
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(t("Voicelab"), t("Build your own voices"))));
    let new = gtk::Button::new();
    new.set_child(Some(&adw::ButtonContent::builder().icon_name("list-add-symbolic").label(t("New voice")).build()));
    new.add_css_class("suggested-action");
    header.pack_start(&new);

    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    list.add_css_class("vn-lab-list");
    let list_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .width_request(230)
        .child(&list)
        .build();
    smooth_scroll(&list_scroll);
    let list_title = label(t("My voices"), &["heading"]);
    list_title.set_xalign(0.0);
    list_title.set_margin_start(18);
    list_title.set_margin_top(14);
    list_title.set_margin_bottom(6);
    let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
    left.append(&list_title);
    list_scroll.set_vexpand(true);
    left.append(&list_scroll);
    left.add_css_class("vn-detail");

    let editor = gtk::Box::new(gtk::Orientation::Vertical, 14);
    editor.set_margin_top(18);
    editor.set_margin_bottom(28);
    editor.set_margin_start(28);
    editor.set_margin_end(28);
    let clamp = adw::Clamp::builder().maximum_size(820).child(&editor).build();
    let editor_scroll =
        gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).hexpand(true).child(&clamp).build();
    smooth_scroll(&editor_scroll);

    let empty = adw::StatusPage::builder()
        .icon_name("applications-science-symbolic")
        .title(t("Make a voice of your own"))
        .description(t("Stack effects like pitch, echo or robot and tune every setting. Start from scratch, or press Customize on any voice."))
        .build();
    let start = gtk::Button::with_label(t("New voice"));
    start.add_css_class("vn-gradient-button");
    start.set_halign(gtk::Align::Center);
    empty.set_child(Some(&start));

    let right = gtk::Stack::new();
    right.set_transition_type(gtk::StackTransitionType::Crossfade);
    right.add_named(&empty, Some("empty"));
    right.add_named(&editor_scroll, Some("editor"));

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&left);
    body.append(&right);

    let widget = adw::ToolbarView::new();
    widget.add_top_bar(&header);
    widget.set_content(Some(&body));

    let current: Rc<RefCell<Option<String>>> = Rc::default();

    let show_editor: Rc<dyn Fn(&str)> = Rc::new({
        let st = st.clone();
        let editor = editor.clone();
        let right = right.clone();
        let current = current.clone();
        move |id: &str| {
            *current.borrow_mut() = Some(id.to_string());
            right.set_visible_child_name("editor");
            fill_editor(&st, &editor, id);
        }
    });

    let rebuild_list = {
        let st = st.clone();
        let list = list.clone();
        let current = current.clone();
        let right = right.clone();
        Rc::new(move || {
            clear(&list);
            let mine: Vec<Voice> = st.voices.borrow().iter().filter(|v| !v.builtin).cloned().collect();
            for v in &mine {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                row.append(&icon(&v.icon, 32));
                let name = label(&v.name, &[]);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                name.set_xalign(0.0);
                row.append(&name);
                let r = gtk::ListBoxRow::new();
                r.set_child(Some(&row));
                r.set_widget_name(&v.id);
                list.append(&r);
                if current.borrow().as_deref() == Some(v.id.as_str()) {
                    list.select_row(Some(&r));
                }
            }
            let has_current = current.borrow().as_ref().is_some_and(|c| mine.iter().any(|v| &v.id == c));
            if !has_current {
                *current.borrow_mut() = None;
                right.set_visible_child_name("empty");
            }
        })
    };
    rebuild_list();

    list.connect_row_activated({
        let show_editor = show_editor.clone();
        let st = st.clone();
        move |_, row| {
            let id = row.widget_name().to_string();
            st.set_voice(&id);
            show_editor(&id);
        }
    });

    let make_new = {
        let st = st.clone();
        let show_editor = show_editor.clone();
        move || {
            let v = st.new_custom();
            st.set_voice(&v.id);
            show_editor(&v.id);
        }
    };
    new.connect_clicked({
        let make_new = make_new.clone();
        move |_| make_new()
    });
    start.connect_clicked(move |_| make_new());

    st.subscribe({
        let rebuild_list = rebuild_list.clone();
        let current = current.clone();
        let st = st.clone();
        let list = list.clone();
        move |c| match c {
            Change::Voices => rebuild_list(),
            Change::VoiceEdited(id) => {
                // Name or picture may have changed: refresh the row only.
                if let Some(v) = st.voices.borrow().iter().find(|v| &v.id == id) {
                    let mut row = list.first_child();
                    while let Some(r) = row {
                        if r.widget_name() == *id
                            && let Some(lr) = r.downcast_ref::<gtk::ListBoxRow>()
                        {
                            let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                            b.append(&icon(&v.icon, 32));
                            let name = label(&v.name, &[]);
                            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                            b.append(&name);
                            lr.set_child(Some(&b));
                        }
                        row = r.next_sibling();
                    }
                }
                let _ = &current;
            }
            _ => {}
        }
    });

    let open: Rc<dyn Fn(&str)> = Rc::new({
        let rebuild_list = rebuild_list.clone();
        let current = current.clone();
        move |id: &str| {
            *current.borrow_mut() = Some(id.to_string());
            rebuild_list();
            show_editor(id);
        }
    });
    OPEN.with(|o| *o.borrow_mut() = Some(open.clone()));
    Lab { widget, open }
}

/// Fills the editor with one voice. Called again after changes to the block list.
fn fill_editor(st: &St, editor: &gtk::Box, id: &str) {
    clear(editor);
    let Some(v) = st.voices.borrow().iter().find(|v| v.id == id).cloned() else { return };

    // Name, picture and actions.
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    top.add_css_class("vn-block");
    let picker = icon_picker(&v.icon, 64, {
        let st = st.clone();
        let id = id.to_string();
        move |name| edit(&st, &id, |v| v.icon = name.to_string())
    });
    picker.add_css_class("flat");
    top.append(&picker);
    let names = gtk::Box::new(gtk::Orientation::Vertical, 6);
    names.set_hexpand(true);
    names.set_valign(gtk::Align::Center);
    let name = gtk::Entry::new();
    name.set_text(&v.name);
    name.add_css_class("title-3");
    name.set_placeholder_text(Some(t("Name of the voice")));
    let save_name = {
        let st = st.clone();
        let id = id.to_string();
        move |e: &gtk::Entry| {
            let text = e.text().trim().to_string();
            if !text.is_empty() {
                edit(&st, &id, |v| v.name = text.clone());
            }
        }
    };
    name.connect_activate(save_name.clone());
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave({
        let name = name.clone();
        move |_| save_name(&name)
    });
    name.add_controller(focus);
    let about = gtk::Entry::new();
    about.set_text(&v.description);
    about.set_placeholder_text(Some(t("Short description (optional)")));
    about.connect_changed({
        let st = st.clone();
        let id = id.to_string();
        move |e| {
            let text = e.text().to_string();
            edit_quiet(&st, &id, |v| v.description = text.clone());
        }
    });
    names.append(&name);
    names.append(&about);
    top.append(&names);
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 6);
    actions.set_valign(gtk::Align::Center);
    let duplicate = gtk::Button::with_label(t("Duplicate"));
    duplicate.connect_clicked({
        let st = st.clone();
        let id = id.to_string();
        move |_| {
            if let Some(copy) = st.duplicate(&id) {
                st.set_voice(&copy.id);
                if let Some(open) = OPEN.with(|o| o.borrow().clone()) {
                    open(&copy.id);
                }
            }
        }
    });
    let delete = gtk::Button::with_label(t("Delete"));
    delete.add_css_class("destructive-action");
    delete.connect_clicked({
        let st = st.clone();
        let id = id.to_string();
        let vname = v.name.clone();
        move |b| {
            let dialog =
                adw::AlertDialog::new(Some(&fill(t("Delete {}?"), &[&vname])), Some(t("The voice is gone for good.")));
            dialog.add_response("cancel", t("Cancel"));
            dialog.add_response("delete", t("Delete"));
            dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            let st = st.clone();
            let id = id.clone();
            dialog.connect_response(None, move |_, r| {
                if r == "delete" {
                    st.delete_custom(&id);
                }
            });
            dialog.present(Some(b));
        }
    });
    actions.append(&duplicate);
    actions.append(&delete);
    top.append(&actions);
    editor.append(&top);

    let hint = label(
        t("Effects run from top to bottom. Turn on Hear myself to hear every change right away."),
        &["dim-label"],
    );
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    editor.append(&hint);

    let count = v.blocks.len();
    for (i, b) in v.blocks.iter().enumerate() {
        if i > 0 {
            let arrow = gtk::Image::from_icon_name("go-down-symbolic");
            arrow.add_css_class("vn-chain-arrow");
            editor.append(&arrow);
        }
        editor.append(&block_card(st, editor, &v.id, i, count, b));
    }

    let add = gtk::MenuButton::new();
    add.set_child(Some(&adw::ButtonContent::builder().icon_name("list-add-symbolic").label(t("Add effect")).build()));
    add.add_css_class("pill");
    add.set_halign(gtk::Align::Center);
    add.set_margin_top(8);
    add.set_popover(Some(&effect_picker(st, editor, &v.id)));
    editor.append(&add);
}

/// Changes a custom voice and saves it.
fn edit(st: &St, id: &str, f: impl FnOnce(&mut Voice)) {
    let Some(mut v) = st.voices.borrow().iter().find(|v| v.id == id).cloned() else { return };
    f(&mut v);
    st.save_custom(v);
}

/// Same, without telling everyone (for typing in the description).
fn edit_quiet(st: &St, id: &str, f: impl FnOnce(&mut Voice)) {
    let mut list = st.voices.borrow_mut();
    let Some(v) = list.iter_mut().find(|v| v.id == id) else { return };
    f(v);
    let v = v.clone();
    drop(list);
    if let Err(e) = crate::voice::save_custom(&crate::voice::custom_dir(), &v) {
        log::error!("could not save voice: {e}");
    }
}

fn block_card(st: &St, editor: &gtk::Box, id: &str, index: usize, count: usize, b: &BlockDef) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
    card.add_css_class("vn-block");
    if !b.enabled {
        card.add_css_class("disabled");
    }
    let Some(spec) = blocks::find(&b.kind) else { return card };

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    head.append(&icon(spec.icon, 36));
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
    titles.set_hexpand(true);
    titles.set_valign(gtk::Align::Center);
    let title = label(&t_dyn(spec.name), &["vn-block-title"]);
    title.set_xalign(0.0);
    let about = label(&t_dyn(spec.about), &["dim-label", "caption"]);
    about.set_xalign(0.0);
    about.set_wrap(true);
    titles.append(&title);
    titles.append(&about);
    head.append(&titles);

    let restructure = {
        let st = st.clone();
        let editor = editor.clone();
        let id = id.to_string();
        move |f: Restructure| {
            edit(&st, &id, |v| f(&mut v.blocks));
            fill_editor(&st, &editor, &id);
        }
    };
    let restructure = Rc::new(restructure);

    let on = gtk::Switch::new();
    on.set_active(b.enabled);
    on.set_valign(gtk::Align::Center);
    on.set_tooltip_text(Some(t("Effect on or off")));
    on.connect_active_notify({
        let restructure = restructure.clone();
        move |s| {
            let active = s.is_active();
            restructure(Box::new(move |blocks| blocks[index].enabled = active));
        }
    });
    let button = |icon: &str, tip: &str, sensitive: bool| {
        let b = gtk::Button::from_icon_name(icon);
        b.add_css_class("flat");
        b.add_css_class("circular");
        b.set_valign(gtk::Align::Center);
        b.set_tooltip_text(Some(tip));
        b.set_sensitive(sensitive);
        b
    };
    let up = button("go-up-symbolic", t("Move up"), index > 0);
    let down = button("go-down-symbolic", t("Move down"), index + 1 < count);
    let remove = button("user-trash-symbolic", t("Remove effect"), true);
    up.connect_clicked({
        let restructure = restructure.clone();
        move |_| restructure(Box::new(move |blocks| blocks.swap(index, index - 1)))
    });
    down.connect_clicked({
        let restructure = restructure.clone();
        move |_| restructure(Box::new(move |blocks| blocks.swap(index, index + 1)))
    });
    remove.connect_clicked({
        let restructure = restructure.clone();
        move |_| {
            restructure(Box::new(move |blocks| {
                blocks.remove(index);
            }))
        }
    });
    head.append(&on);
    head.append(&up);
    head.append(&down);
    head.append(&remove);
    card.append(&head);

    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(1)
        .max_children_per_line(2)
        .homogeneous(true)
        .column_spacing(10)
        .row_spacing(10)
        .build();
    let values = b.values();
    for (pi, p) in spec.params.iter().enumerate() {
        let row = param_row(p, values[pi], {
            let st = st.clone();
            let id = id.to_string();
            let key = p.key;
            move |value| st.set_param(&id, index, key, value)
        });
        grid.append(&row);
    }
    card.append(&grid);
    card
}

fn effect_picker(st: &St, editor: &gtk::Box, id: &str) -> gtk::Popover {
    let popover = gtk::Popover::new();
    bounce(&popover);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    col.set_margin_top(10);
    col.set_margin_bottom(10);
    col.set_margin_start(10);
    col.set_margin_end(10);
    for g in Group::ALL {
        let head = label(&t_dyn(g.label()), &["heading"]);
        head.set_xalign(0.0);
        head.set_margin_top(6);
        col.append(&head);
        let grid = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .min_children_per_line(3)
            .max_children_per_line(3)
            .homogeneous(true)
            .column_spacing(4)
            .row_spacing(4)
            .build();
        for spec in BLOCKS.iter().filter(|b| b.group == g) {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            b.add_css_class("vn-effect-tile");
            let inner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            inner.append(&icon(spec.icon, 28));
            let l = label(&t_dyn(spec.name), &[]);
            l.set_xalign(0.0);
            inner.append(&l);
            b.set_child(Some(&inner));
            b.set_tooltip_text(Some(&t_dyn(spec.about)));
            let st = st.clone();
            let editor = editor.clone();
            let id = id.to_string();
            let popover = popover.clone();
            let kind = spec.kind;
            b.connect_clicked(move |_| {
                popover.popdown();
                edit(&st, &id, |v| v.blocks.push(BlockDef::new(kind)));
                fill_editor(&st, &editor, &id);
            });
            grid.append(&b);
        }
        col.append(&grid);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(420)
        .min_content_width(480)
        .child(&col)
        .build();
    smooth_scroll(&scroll);
    popover.set_child(Some(&scroll));
    popover
}
