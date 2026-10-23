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
    /// Favorite voices: id, name and their picture as PNG.
    pub favorites: Vec<(String, String, Vec<u8>)>,
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
/// Waves for the background sounds, the Lucide icon of the bottom bar, colored the same way.
const BACKGROUND: &str = "io.github.voicenux.Voicenux-background-symbolic";
const BACKGROUND_SVG: &str = include_str!("../data/app/io.github.voicenux.Voicenux-background-symbolic.svg");

pub type Handle = ksni::blocking::Handle<Tray>;

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        crate::app::APP_ID.into()
    }

    fn title(&self) -> String {
        "Voicenux".into()
    }

    /// The logo goes only as a pixmap: for a name, KDE takes its "-symbolic" icon when there is
    /// one, and that is the plain icon.
    fn icon_name(&self) -> String {
        if self.s.mono { MONO.into() } else { String::new() }
    }

    fn icon_theme_path(&self) -> String {
        self.theme_path.clone()
    }

    /// The logo, and the plain icon for trays that cannot find it by its name.
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

    /// Every item has an icon: KDE keeps a column for icons, and with only some of them filled
    /// the texts look out of line. The icons are the ones of the bottom bar.
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let action = |id: &'static str| -> Box<dyn Fn(&mut Self) + Send> {
            Box::new(move |this: &mut Self| {
                let _ = this.tx.send_blocking(Cmd::Action(id));
            })
        };
        let toggle = |label: &str, icon: &str, checked: bool, id: &'static str| -> MenuItem<Self> {
            CheckmarkItem {
                label: label.into(),
                icon_name: icon.into(),
                checked,
                activate: action(id),
                ..Default::default()
            }
            .into()
        };
        let item = |label: &str, icon: &str, id: &'static str| -> MenuItem<Self> {
            StandardItem { label: label.into(), icon_name: icon.into(), activate: action(id), ..Default::default() }
                .into()
        };
        let mut voices: Vec<MenuItem<Self>> = self
            .s
            .favorites
            .iter()
            .map(|(id, name, picture)| {
                let id = id.clone();
                StandardItem {
                    label: name.clone(),
                    icon_data: picture.clone(),
                    activate: Box::new(move |this: &mut Self| {
                        let _ = this.tx.send_blocking(Cmd::Voice(id.clone()));
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect();
        if voices.len() >= 2 {
            voices.insert(0, item(t("Random favorite"), "media-playlist-shuffle-symbolic", "random-favorite"));
            voices.insert(1, MenuItem::Separator);
        }
        if voices.is_empty() {
            voices.push(
                StandardItem { label: t("Star voices to see them here").into(), enabled: false, ..Default::default() }
                    .into(),
            );
        }
        vec![
            StandardItem {
                label: t("Open Voicenux").into(),
                icon_name: if self.s.mono { MONO.into() } else { crate::app::APP_ID.into() },
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send_blocking(Cmd::Show);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            toggle(t("Voice changer"), "audio-input-microphone-symbolic", self.s.voice_on, "voice-toggle"),
            toggle(t("Hear myself"), "audio-headphones-symbolic", self.s.hear, "hear-toggle"),
            toggle(t("Background"), BACKGROUND, self.s.background, "background-toggle"),
            toggle(t("Mute microphone"), "microphone-sensitivity-muted-symbolic", self.s.muted, "mute-toggle"),
            MenuItem::Separator,
            SubMenu {
                label: t("Favorite voices").into(),
                icon_name: "starred-symbolic".into(),
                submenu: voices,
                ..Default::default()
            }
            .into(),
            item(t("Random voice"), "media-playlist-shuffle-symbolic", "random-voice"),
            item(t("Stop all sounds"), "media-playback-stop-symbolic", "stop-sounds"),
            MenuItem::Separator,
            StandardItem {
                label: t("Quit").into(),
                icon_name: "application-exit-symbolic".into(),
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
    let written = std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(dir.join(format!("{MONO}.svg")), MONO_SVG))
        .and_then(|_| std::fs::write(dir.join(format!("{BACKGROUND}.svg")), BACKGROUND_SVG));
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
