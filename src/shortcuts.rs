//! Global shortcuts through the XDG desktop portal, so they work on Wayland too.
//! The desktop shows its own dialog to confirm or change the keys.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use ashpd::desktop::{CreateSessionOptions, Session};
use futures_util::StreamExt;
use gtk::glib;

use crate::i18n::{fill, t, t_dyn};
use crate::state::{Change, St};

/// Actions that can have a shortcut, with the keys we suggest.
pub const ACTIONS: &[(&str, &str, &str)] = &[
    ("voice-toggle", "Voice changer on or off", "CTRL+ALT+V"),
    ("hear-toggle", "Hear myself on or off", "CTRL+ALT+H"),
    ("mute-toggle", "Mute microphone", "CTRL+ALT+M"),
    ("background-toggle", "Background sounds on or off", "CTRL+ALT+B"),
    ("random-voice", "Random voice", "CTRL+ALT+R"),
    ("random-favorite", "Random favorite", "CTRL+SHIFT+ALT+R"),
    ("next-voice", "Next voice", "CTRL+ALT+Right"),
    ("previous-voice", "Previous voice", "CTRL+ALT+Left"),
    ("stop-sounds", "Stop all sounds", "CTRL+ALT+S"),
];

#[derive(Default)]
pub struct Shortcuts {
    portal: Option<Rc<(GlobalShortcuts, Session<GlobalShortcuts>)>>,
    /// What the desktop says each shortcut is bound to, like "Ctrl+Alt+V".
    pub bound: HashMap<String, String>,
    pub unavailable: bool,
}

pub type Handle = Rc<RefCell<Shortcuts>>;

fn wanted(st: &St) -> Vec<NewShortcut> {
    let c = st.config.borrow();
    let mut list: Vec<NewShortcut> = ACTIONS
        .iter()
        .map(|(id, desc, keys)| {
            let keys = c.shortcuts.get(*id).map(String::as_str).unwrap_or(keys);
            NewShortcut::new(*id, t_dyn(desc)).preferred_trigger(Some(keys))
        })
        .collect();
    for (id, keys) in c.shortcuts.iter().filter(|(id, _)| id.contains(':')) {
        let (kind, target) = id.split_once(':').unwrap();
        let name = match kind {
            "voice" => {
                st.voices.borrow().iter().find(|v| v.id == target).map(|v| fill(t("Voice: {}"), &[&t_dyn(&v.name)]))
            }
            "sound" => st.library.borrow().find(target).map(|s| fill(t("Sound: {}"), &[&t_dyn(&s.name)])),
            _ => None,
        };
        if let Some(name) = name {
            list.push(NewShortcut::new(id.clone(), name).preferred_trigger(Some(keys.as_str())));
        }
    }
    list
}

/// Connects to the portal and binds the shortcuts, if the user has set them up before.
/// `run` gets the id of a shortcut when it is pressed.
pub fn start(st: &St, handle: &Handle, run: Rc<dyn Fn(&str)>) {
    let st = st.clone();
    let handle = handle.clone();
    glib::spawn_future_local(async move {
        let _ = ashpd::register_host_app(crate::app::APP_ID.parse().unwrap()).await;
        let proxy = match GlobalShortcuts::new().await {
            Ok(p) => p,
            Err(e) => {
                log::info!("no global shortcuts portal: {e}");
                handle.borrow_mut().unavailable = true;
                st.notify(Change::Settings);
                return;
            }
        };
        let session = match proxy.create_session(CreateSessionOptions::default()).await {
            Ok(s) => s,
            Err(e) => {
                log::warn!("global shortcuts session failed: {e}");
                handle.borrow_mut().unavailable = true;
                st.notify(Change::Settings);
                return;
            }
        };
        let mut activated = match proxy.receive_activated().await {
            Ok(s) => s,
            Err(e) => {
                log::warn!("cannot listen for shortcuts: {e}");
                return;
            }
        };
        let portal = Rc::new((proxy, session));
        handle.borrow_mut().portal = Some(portal.clone());
        if st.config.borrow().shortcuts_on {
            bind(&st, &handle).await;
        }
        while let Some(a) = activated.next().await {
            run(a.shortcut_id());
        }
    });
}

async fn bind(st: &St, handle: &Handle) {
    let Some(portal) = handle.borrow().portal.clone() else { return };
    let list = wanted(st);
    let result = async {
        let req = portal.0.bind_shortcuts(&portal.1, &list, None, BindShortcutsOptions::default()).await?;
        req.response()
    }
    .await;
    match result {
        Ok(r) => {
            let mut h = handle.borrow_mut();
            h.bound = r.shortcuts().iter().map(|s| (s.id().to_string(), s.trigger_description().to_string())).collect();
        }
        Err(e) => log::warn!("binding shortcuts failed: {e}"),
    }
    st.notify(Change::Settings);
}

/// Binds again, for example after a voice got a shortcut. The desktop may ask the user.
pub fn rebind(st: &St, handle: &Handle) {
    st.config.borrow_mut().shortcuts_on = true;
    st.save_later();
    let st = st.clone();
    let handle = handle.clone();
    glib::spawn_future_local(async move { bind(&st, &handle).await });
}

/// Opens the desktop's own shortcut settings when it has them, otherwise binds again.
pub fn configure(st: &St, handle: &Handle) {
    let Some(portal) = handle.borrow().portal.clone() else { return };
    if portal.0.version() < 2 || handle.borrow().bound.is_empty() {
        rebind(st, handle);
        return;
    }
    glib::spawn_future_local(async move {
        if let Err(e) = portal.0.configure_shortcuts(&portal.1, None, Default::default()).await {
            log::warn!("cannot open the shortcut settings: {e}");
        }
    });
}

/// Turns a key press into the portal's trigger syntax, like "CTRL+ALT+v".
pub fn trigger(key: gtk::gdk::Key, mods: gtk::gdk::ModifierType) -> Option<String> {
    use gtk::gdk::ModifierType as M;
    let name = key.to_lower().name()?.to_string();
    if ["Control_L", "Control_R", "Alt_L", "Alt_R", "Shift_L", "Shift_R", "Super_L", "Super_R", "Meta_L", "Meta_R"]
        .contains(&name.as_str())
    {
        return None;
    }
    let mut parts = Vec::new();
    if mods.contains(M::CONTROL_MASK) {
        parts.push("CTRL".to_string());
    }
    if mods.contains(M::ALT_MASK) {
        parts.push("ALT".to_string());
    }
    if mods.contains(M::SHIFT_MASK) {
        parts.push("SHIFT".to_string());
    }
    if mods.contains(M::SUPER_MASK) {
        parts.push("LOGO".to_string());
    }
    parts.push(name);
    Some(parts.join("+"))
}

/// "CTRL+ALT+v" to "Ctrl+Alt+V" for display.
pub fn pretty(trigger: &str) -> String {
    trigger
        .split('+')
        .map(|p| match p {
            "CTRL" => "Ctrl".to_string(),
            "ALT" => "Alt".to_string(),
            "SHIFT" => "Shift".to_string(),
            "LOGO" => "Super".to_string(),
            k if k.len() == 1 => k.to_uppercase(),
            k => k.to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_triggers() {
        assert_eq!(pretty("CTRL+ALT+v"), "Ctrl+Alt+V");
        assert_eq!(pretty("LOGO+F5"), "Super+F5");
    }
}
