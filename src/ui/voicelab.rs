//! Voicelab: build your own voices from effect blocks.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use super::voices::param_row;
use super::{popup, clear, icon, icon_picker, label, smooth_scroll};
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
    pub widget: gtk::Box,
    open: Rc<dyn Fn(&str)>,
    new_voice: Rc<dyn Fn()>,
}

impl Lab {
    pub fn open(&self, id: &str) {
        (self.open)(id)
    }

    /// Makes a new voice and opens it.
    pub fn new_voice(&self) {
        (self.new_voice)()
    }
}

pub fn build(st: &St) -> Lab {
    // The list and the right side each have their own header, so the title sits centered above
    // the editor and the empty page.
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new(t("Voicelab"), t("Build your own voices"))));
    let list_header = adw::HeaderBar::new();
    list_header.set_show_end_title_buttons(false);
    list_header.set_title_widget(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    let new = gtk::Button::new();
    new.set_child(Some(&adw::ButtonContent::builder().icon_name("list-add-symbolic").label(t("New voice")).build()));
    new.add_css_class("suggested-action");
    list_header.pack_start(&new);

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
        .paintable(&super::load_picture("alembic"))
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

    let left_view = adw::ToolbarView::new();
    left_view.add_top_bar(&list_header);
    left_view.set_content(Some(&left));
    let right_view = adw::ToolbarView::new();
    right_view.add_top_bar(&header);
    right_view.set_content(Some(&right));
    right_view.set_hexpand(true);

    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    widget.append(&left_view);
    widget.append(&right_view);

    let current: Rc<RefCell<Option<String>>> = Rc::default();

    // Opening a voice here does not switch to it, so Voicelab never changes your voice in a call
    // by itself. This button does, when the open voice is not the one in use.
    let use_voice = gtk::Button::with_label(t("Use voice"));
    use_voice.add_css_class("suggested-action");
    header.pack_start(&use_voice);
    let sync_use = Rc::new({
        let st = st.clone();
        let current = current.clone();
        let use_voice = use_voice.clone();
        move || {
            let c = st.config.borrow();
            let open = current.borrow();
            use_voice.set_visible(open.as_ref().is_some_and(|id| c.voice != *id || !c.voice_on));
        }
    });
    use_voice.connect_clicked({
        let st = st.clone();
        let current = current.clone();
        move |_| {
            let id = current.borrow().clone();
            if let Some(id) = id {
                st.set_voice(&id);
            }
        }
    });

    let show_editor: Rc<dyn Fn(&str)> = Rc::new({
        let st = st.clone();
        let editor = editor.clone();
        let right = right.clone();
        let current = current.clone();
        let sync_use = sync_use.clone();
        move |id: &str| {
            *current.borrow_mut() = Some(id.to_string());
            right.set_visible_child_name("editor");
            fill_editor(&st, &editor, id);
            sync_use();
        }
    });

    let rebuild_list = {
        let st = st.clone();
        let list = list.clone();
        let current = current.clone();
        let right = right.clone();
        let left_view = left_view.clone();
        let show_editor = show_editor.clone();
        let sync_use = sync_use.clone();
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
                popup::context(&r, {
                    let (st, id, name, weak) = (st.clone(), v.id.clone(), v.name.clone(), r.downgrade());
                    move |x, y| {
                        let Some(r) = weak.upgrade() else { return };
                        let (st1, st2, st3, id1, id2, id3, name) =
                            (st.clone(), st.clone(), st.clone(), id.clone(), id.clone(), id.clone(), name.clone());
                        let parent = r.downgrade();
                        popup::show_list(&r, x, y, vec![
                            popup::Item::new("object-select-symbolic", t("Use voice"), move || st1.set_voice(&id1)),
                            popup::Item::new("edit-copy-symbolic", t("Duplicate"), move || duplicate(&st2, &id2)),
                            popup::Item::new("user-trash-symbolic", t("Delete…"), move || {
                                if let Some(p) = parent.upgrade() {
                                    ask_delete(&st3, &id3, &name, &p);
                                }
                            })
                            .danger(),
                        ]);
                    }
                });
                list.append(&r);
                if current.borrow().as_deref() == Some(v.id.as_str()) {
                    list.select_row(Some(&r));
                }
            }
            // Without voices of your own, the empty page gets the whole width.
            left_view.set_visible(!mine.is_empty());
            let has_current = current.borrow().as_ref().is_some_and(|c| mine.iter().any(|v| &v.id == c));
            if has_current {
                return;
            }
            // Open a voice rather than an empty page: the one in use, or else the first.
            let voice = st.config.borrow().voice.clone();
            match mine.iter().find(|v| v.id == voice).or(mine.first()) {
                Some(v) => {
                    let mut row = list.first_child();
                    while let Some(r) = row {
                        if r.widget_name() == v.id.as_str() {
                            list.select_row(r.downcast_ref::<gtk::ListBoxRow>());
                        }
                        row = r.next_sibling();
                    }
                    show_editor(&v.id);
                }
                None => {
                    *current.borrow_mut() = None;
                    right.set_visible_child_name("empty");
                    sync_use();
                }
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

    let make_new: Rc<dyn Fn()> = Rc::new({
        let st = st.clone();
        let show_editor = show_editor.clone();
        move || {
            let v = st.new_custom();
            st.set_voice(&v.id);
            show_editor(&v.id);
        }
    });
    new.set_tooltip_text(Some(&super::with_key(t("New voice"), "N")));
    new.connect_clicked({
        let make_new = make_new.clone();
        move |_| make_new()
    });
    start.connect_clicked({
        let make_new = make_new.clone();
        move |_| make_new()
    });

    st.subscribe({
        let rebuild_list = rebuild_list.clone();
        let current = current.clone();
        let st = st.clone();
        let list = list.clone();
        move |c| match c {
            Change::Voices => rebuild_list(),
            Change::Voice | Change::Toggles => sync_use(),
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
    Lab { widget, open, new_voice: make_new }
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
    let duplicate_button = gtk::Button::with_label(t("Duplicate"));
    duplicate_button.connect_clicked({
        let st = st.clone();
        let id = id.to_string();
        move |_| duplicate(&st, &id)
    });
    let delete = gtk::Button::with_label(t("Delete"));
    delete.add_css_class("destructive-action");
    delete.connect_clicked({
        let st = st.clone();
        let id = id.to_string();
        let vname = v.name.clone();
        move |b| ask_delete(&st, &id, &vname, b)
    });
    actions.append(&duplicate_button);
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

/// Asks before a voice of your own is deleted.
fn ask_delete(st: &St, id: &str, name: &str, parent: &impl IsA<gtk::Widget>) {
    let dialog = adw::AlertDialog::new(Some(&fill(t("Delete {}?"), &[name])), Some(t("The voice is gone for good.")));
    dialog.add_response("cancel", t("Cancel"));
    dialog.add_response("delete", t("Delete"));
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    let st = st.clone();
    let id = id.to_string();
    dialog.connect_response(None, move |_, r| {
        if r == "delete" {
            st.delete_custom(&id);
        }
    });
    dialog.present(Some(parent));
}

/// Makes a copy of a voice and opens it.
fn duplicate(st: &St, id: &str) {
    if let Some(copy) = st.duplicate(id) {
        st.set_voice(&copy.id);
        if let Some(open) = OPEN.with(|o| o.borrow().clone()) {
            open(&copy.id);
        }
    }
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
    popup::menu(&popover);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    col.set_margin_bottom(10);
    col.set_margin_start(10);
    col.set_margin_end(10);
    // Each group: its title, its tiles, and each tile with the words that find it.
    type Tiles = Vec<(gtk::Button, String)>;
    let mut groups: Vec<(gtk::Label, gtk::FlowBox, Tiles)> = Vec::new();
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
        let mut tiles = Vec::new();
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
            let words = super::fold(&format!("{} {} {} {}", t_dyn(spec.name), spec.name, t_dyn(spec.about), spec.about));
            tiles.push((b, words));
        }
        col.append(&grid);
        groups.push((head, grid, tiles));
    }
    let none = label(t("No effects found"), &["dim-label"]);
    none.set_margin_top(24);
    none.set_visible(false);
    col.append(&none);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(420)
        .min_content_width(480)
        .child(&col)
        .build();
    smooth_scroll(&scroll);

    // Typing filters the effects, Enter adds the first one found.
    let search = gtk::SearchEntry::builder().placeholder_text(t("Search effects")).build();
    search.set_margin_top(10);
    search.set_margin_bottom(6);
    search.set_margin_start(10);
    search.set_margin_end(10);
    let groups = Rc::new(groups);
    search.connect_search_changed({
        let (groups, none) = (groups.clone(), none.clone());
        move |e| {
            let words: Vec<String> = super::fold(&e.text()).split_whitespace().map(String::from).collect();
            let mut found = false;
            for (head, grid, tiles) in groups.iter() {
                let mut any = false;
                for (b, text) in tiles {
                    let shown = words.iter().all(|w| text.contains(w.as_str()));
                    if let Some(cell) = b.parent() {
                        cell.set_visible(shown);
                    }
                    any |= shown;
                }
                head.set_visible(any);
                grid.set_visible(any);
                found |= any;
            }
            none.set_visible(!found);
        }
    });
    search.connect_activate({
        let groups = groups.clone();
        move |_| {
            let first = groups.iter().flat_map(|(_, _, tiles)| tiles).find(|(b, _)| b.parent().is_some_and(|c| c.is_visible()));
            if let Some((b, _)) = first {
                b.emit_clicked();
            }
        }
    });
    popover.connect_map({
        let search = search.clone();
        move |_| {
            search.grab_focus();
        }
    });
    popover.connect_closed({
        let search = search.clone();
        move |_| search.set_text("")
    });
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&search);
    body.append(&scroll);
    popover.set_child(Some(&body));
    popover
}
