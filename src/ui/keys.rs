//! Shortcut buttons for voices and sounds: press keys, the desktop confirms them.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::label;
use crate::i18n::{fill, t};
use crate::shortcuts;
use crate::state::{Change, St};

/// What a shortcut id is bound to: the desktop's name for it, or the keys we asked for.
pub fn shown(st: &St, id: &str) -> Option<String> {
    if let Some(b) = st.keys.borrow().bound.get(id).filter(|s| !s.is_empty()) {
        return Some(b.clone());
    }
    st.config.borrow().shortcuts.get(id).map(|k| shortcuts::pretty(k))
}

/// A button that shows the shortcut of `id` ("voice:robot", "sound:coin") and changes it.
pub fn button(st: &St, id: &str, what: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("pill");
    let sync = {
        let st = st.clone();
        let b = b.clone();
        let id = id.to_string();
        move || {
            let text = shown(&st, &id);
            b.set_child(Some(
                &adw::ButtonContent::builder()
                    .icon_name("input-keyboard-symbolic")
                    .label(text.as_deref().unwrap_or(t("Add shortcut")))
                    .build(),
            ));
        }
    };
    sync();
    b.connect_clicked({
        let st = st.clone();
        let id = id.to_string();
        let what = what.to_string();
        move |b| {
            // In a menu, the menu closes first: the dialog opens on the window, under the menu.
            let Some(window) = b.root() else { return };
            if let Some(menu) = b.ancestor(gtk::Popover::static_type()).and_downcast::<gtk::Popover>() {
                menu.popdown();
            }
            record(&st, &window, &id, &what)
        }
    });
    let weak = b.downgrade();
    st.subscribe(move |c| {
        if matches!(c, Change::Settings) && weak.upgrade().is_some() {
            sync();
        }
    });
    b
}

pub fn record(st: &St, parent: &impl IsA<gtk::Widget>, id: &str, what: &str) {
    if st.keys.borrow().unavailable {
        st.notify(Change::Notice(t("Your desktop has no global shortcuts").to_string()));
        return;
    }
    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_top(10);
    body.set_margin_bottom(22);
    body.set_margin_start(24);
    body.set_margin_end(24);
    let title = label(&fill(t("Shortcut for {}"), &[what]), &["title-3"]);
    title.set_wrap(true);
    let hint = label(t("Press the keys you want, for example Ctrl+Alt+1."), &["dim-label"]);
    hint.set_wrap(true);
    let keys = label(t("Waiting for keys…"), &["vn-hero-title"]);
    keys.set_margin_top(8);
    keys.set_margin_bottom(8);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::Center);
    let clear = gtk::Button::with_label(t("Remove"));
    clear.add_css_class("pill");
    let save = gtk::Button::with_label(t("Save"));
    save.add_css_class("vn-gradient-button");
    save.set_sensitive(false);
    buttons.append(&clear);
    buttons.append(&save);
    body.append(&title);
    body.append(&hint);
    body.append(&keys);
    body.append(&buttons);

    let header = adw::HeaderBar::new();
    header.set_show_title(false);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);
    root.append(&body);
    let dialog = adw::Dialog::builder().content_width(420).child(&root).build();

    let picked: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
    let ctl = gtk::EventControllerKey::new();
    ctl.set_propagation_phase(gtk::PropagationPhase::Capture);
    ctl.connect_key_pressed({
        let picked = picked.clone();
        let keys = keys.clone();
        let save = save.clone();
        move |_, key, _, mods| {
            if key == gtk::gdk::Key::Escape && mods.is_empty() {
                return glib::Propagation::Proceed;
            }
            if let Some(tr) = shortcuts::trigger(key, mods) {
                keys.set_label(&shortcuts::pretty(&tr));
                *picked.borrow_mut() = Some(tr);
                save.set_sensitive(true);
            }
            glib::Propagation::Stop
        }
    });
    dialog.add_controller(ctl);

    let apply = {
        let st = st.clone();
        let id = id.to_string();
        let dialog = dialog.clone();
        move |keys: Option<String>| {
            st.update(|c| match &keys {
                Some(k) => {
                    c.shortcuts.insert(id.clone(), k.clone());
                }
                None => {
                    c.shortcuts.remove(&id);
                }
            });
            shortcuts::rebind(&st, &st.keys);
            dialog.close();
        }
    };
    let apply = Rc::new(apply);
    save.connect_clicked({
        let apply = apply.clone();
        let picked = picked.clone();
        move |_| apply(picked.borrow().clone())
    });
    clear.connect_clicked(move |_| apply(None));
    dialog.present(Some(parent));
}
