//! Tray icon (StatusNotifierItem), so Voicenux can run without a window.

use gtk::prelude::*;
use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use ksni::{Icon, MenuItem, ToolTip};

use crate::i18n::t;

pub enum Cmd {
    Show,
    Action(&'static str),
    Voice(String),
    Quit,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub voice_on: bool,
    pub hear: bool,
    pub muted: bool,
    pub background: bool,
    pub voice: String,
    /// Favorite voices: id and name.
    pub favorites: Vec<(String, String)>,
    /// The plain one-color icon instead of the logo.
    pub mono: bool,
}

pub struct Tray {
    tx: async_channel::Sender<Cmd>,
    s: Snapshot,
    icon: Vec<Icon>,
    /// Where the plain icon is, for a Voicenux that is not installed.
    theme_path: String,
}

/// The plain icon: the bars of the logo in the text color of the panel. KDE colors it through
/// its `ColorScheme-Text` class, GNOME because the name ends in "-symbolic".
const MONO: &str = "io.github.voicenux.Voicenux-symbolic";
const MONO_SVG: &str = include_str!("../data/app/io.github.voicenux.Voicenux-symbolic.svg");

pub type Handle = ksni::blocking::Handle<Tray>;

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        crate::app::APP_ID.into()
    }

    fn title(&self) -> String {
        "Voicenux".into()
    }

    fn icon_name(&self) -> String {
        if self.s.mono { MONO.into() } else { crate::app::APP_ID.into() }
    }

    fn icon_theme_path(&self) -> String {
        self.theme_path.clone()
    }

    /// Only for trays that cannot find the icon by its name.
    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        let state = if self.s.voice_on { self.s.voice.clone() } else { t("Voice changer off").to_string() };
        ToolTip { title: "Voicenux".into(), description: state, ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send_blocking(Cmd::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let action = |id: &'static str| -> Box<dyn Fn(&mut Self) + Send> {
            Box::new(move |this: &mut Self| {
                let _ = this.tx.send_blocking(Cmd::Action(id));
            })
        };
        let mut voices: Vec<MenuItem<Self>> = self
            .s
            .favorites
            .iter()
            .map(|(id, name)| {
                let id = id.clone();
                StandardItem {
                    label: name.clone(),
                    activate: Box::new(move |this: &mut Self| {
                        let _ = this.tx.send_blocking(Cmd::Voice(id.clone()));
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();
        if voices.is_empty() {
            voices.push(
                StandardItem { label: t("Star voices to see them here").into(), enabled: false, ..Default::default() }
                    .into(),
            );
        }
        vec![
            StandardItem {
                label: t("Open Voicenux").into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send_blocking(Cmd::Show);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: t("Voice changer").into(),
                checked: self.s.voice_on,
                activate: action("voice-toggle"),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: t("Hear myself").into(),
                checked: self.s.hear,
                activate: action("hear-toggle"),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: t("Background").into(),
                checked: self.s.background,
                activate: action("background-toggle"),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: t("Mute microphone").into(),
                checked: self.s.muted,
                activate: action("mute-toggle"),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            SubMenu { label: t("Favorite voices").into(), submenu: voices, ..Default::default() }.into(),
            StandardItem { label: t("Random voice").into(), activate: action("random-voice"), ..Default::default() }
                .into(),
            StandardItem { label: t("Stop all sounds").into(), activate: action("stop-sounds"), ..Default::default() }
                .into(),
            MenuItem::Separator,
            StandardItem {
                label: t("Quit").into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send_blocking(Cmd::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// The logo as ARGB32, the way StatusNotifierItem wants it.
fn logo_pixmap() -> Vec<Icon> {
    let tex = gtk::gdk::Texture::from_resource(&format!("{}/logo.svg", crate::ui::RESOURCE));
    let mut d = gtk::gdk::TextureDownloader::new(&tex);
    d.set_format(gtk::gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = d.download_bytes();
    let (w, h) = (tex.width() as usize, tex.height() as usize);
    let mut data = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let i = y * stride + x * 4;
            data.extend_from_slice(&[bytes[i + 3], bytes[i], bytes[i + 1], bytes[i + 2]]);
        }
    }
    vec![Icon { width: w as i32, height: h as i32, data }]
}

/// Puts the plain icon in a folder of the runtime dir, laid out like an icon theme, and returns
/// the folder. Installed, the icon theme has it anyway.
fn write_theme() -> String {
    let root = gtk::glib::user_runtime_dir().join("voicenux").join("icons");
    let dir = root.join("hicolor").join("scalable").join("apps");
    let written = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(format!("{MONO}.svg")), MONO_SVG));
    match written {
        Ok(()) => root.to_string_lossy().into_owned(),
        Err(e) => {
            log::warn!("tray icon: {e}");
            String::new()
        }
    }
}

pub fn spawn(tx: async_channel::Sender<Cmd>, s: Snapshot) -> Option<Handle> {
    let tray = Tray { tx, s, icon: logo_pixmap(), theme_path: write_theme() };
    match tray.spawn() {
        Ok(h) => Some(h),
        Err(e) => {
            log::info!("no tray: {e}");
            None
        }
    }
}

pub fn update(handle: &Handle, s: Snapshot) {
    handle.update(move |t: &mut Tray| t.s = s);
}
