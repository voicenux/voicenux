//! Settings. They live in `~/.config/voicenux/config.toml`, but people change them in the
//! app, never in the file.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::voice::BlockDef;

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("voicenux")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Selected voice.
    pub voice: String,
    pub voice_on: bool,
    pub hear_myself: bool,
    pub background: bool,
    /// Node name of the microphone, None for the system default.
    pub input: Option<String>,
    /// Record from EasyEffects when it works on the same microphone.
    pub easyeffects: bool,
    /// Node name of the headphones for "Hear myself", None for the system default.
    pub output: Option<String>,
    pub input_gain_db: f32,
    pub mic_volume_db: f32,
    pub monitor_volume: f32,
    pub noise_suppression: bool,
    pub gate: bool,
    pub gate_db: f32,
    pub low_latency: bool,
    pub hear_sounds: bool,
    pub sound_volume: f32,
    pub favorites: Vec<String>,
    /// Changed settings of built-in voices, by voice id.
    pub tweaks: BTreeMap<String, Vec<BlockDef>>,
    /// Shortcuts by action name, in the portal's trigger syntax ("CTRL+ALT+v").
    pub shortcuts: BTreeMap<String, String>,
    /// The user has set up shortcuts once, so they are bound at every start.
    pub shortcuts_on: bool,
    pub close_to_tray: bool,
    pub autostart: bool,
    /// "system", "light" or "dark".
    pub appearance: String,
    pub onboarded: bool,
    pub default_mic: bool,
    /// The microphone that was the default before Voicenux took over.
    pub previous_default: Option<String>,
    pub width: i32,
    pub height: i32,
    /// The page the window showed last.
    pub page: String,
    /// Other apps on this computer may control Voicenux, see `src/api.rs`.
    pub api: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            voice: "baby".into(),
            voice_on: true,
            hear_myself: false,
            background: true,
            input: None,
            easyeffects: true,
            output: None,
            input_gain_db: 0.0,
            mic_volume_db: 0.0,
            monitor_volume: 0.8,
            noise_suppression: false,
            gate: false,
            gate_db: -50.0,
            low_latency: false,
            hear_sounds: true,
            sound_volume: 0.8,
            favorites: Vec::new(),
            tweaks: BTreeMap::new(),
            shortcuts: BTreeMap::new(),
            shortcuts_on: false,
            close_to_tray: true,
            autostart: false,
            appearance: "system".into(),
            onboarded: false,
            default_mic: false,
            previous_default: None,
            width: 1180,
            height: 760,
            page: "voices".into(),
            api: false,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        config_dir().join("config.toml")
    }

    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                log::warn!("config.toml is broken, using defaults: {e}");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let dir = config_dir();
        let tmp = dir.join("config.toml.tmp");
        let result = std::fs::create_dir_all(&dir)
            .map_err(anyhow::Error::from)
            .and_then(|_| Ok(toml::to_string_pretty(self)?))
            .and_then(|text| Ok(std::fs::write(&tmp, text)?))
            .and_then(|_| Ok(std::fs::rename(&tmp, Self::path())?));
        if let Err(e) = result {
            log::error!("could not save settings: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut c = Config::default();
        c.favorites.push("robot".into());
        c.tweaks.insert("baby".into(), vec![BlockDef::new("pitch")]);
        let text = toml::to_string_pretty(&c).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), c);
    }

    #[test]
    fn old_files_still_load() {
        let c: Config = toml::from_str("voice = \"robot\"\nunknown = 1\n").unwrap();
        assert_eq!(c.voice, "robot");
        assert!(c.voice_on);
    }
}
