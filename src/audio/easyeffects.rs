//! EasyEffects runs its own microphone effects (noise reduction, EQ and more) and offers the
//! result as the source `easyeffects_source`. Voicenux records from there when EasyEffects works
//! on the same microphone, so those effects come before the voice.

/// The source with the EasyEffects microphone effects on it.
pub const SOURCE: &str = "easyeffects_source";

/// What the EasyEffects microphone effects do for the voice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Use {
    /// EasyEffects is not running.
    #[default]
    Missing,
    /// Voicenux records from EasyEffects: its effects run before the voice.
    Before,
    /// EasyEffects works on the Voicenux Microphone: its effects run after the voice.
    After,
    /// EasyEffects works on another microphone, or the switch is off.
    Unused,
}

/// Nodes of the EasyEffects pipelines (`ee_sie_*` for the microphone).
pub fn is_own(name: &str) -> bool {
    name.starts_with("ee_")
}

pub fn is_mic_pipeline(name: &str) -> bool {
    name.starts_with("ee_sie_")
}

fn settings_files() -> [Option<std::path::PathBuf>; 2] {
    [
        dirs::config_dir().map(|d| d.join("easyeffects/db/easyeffectsrc")),
        dirs::home_dir().map(|h| h.join(".var/app/com.github.wwmm.easyeffects/config/easyeffects/db/easyeffectsrc")),
    ]
}

/// EasyEffects has run on this computer once, as a package or a Flatpak.
pub fn installed() -> bool {
    settings_files().iter().flatten().any(|p| p.exists())
}

/// The microphone EasyEffects is set to, from its settings. None when it follows the default.
pub fn configured_input() -> Option<String> {
    let text = settings_files().iter().flatten().find_map(|p| std::fs::read_to_string(p).ok())?;
    parse_input(&text)
}

fn parse_input(text: &str) -> Option<String> {
    let mut section = "";
    let mut use_default = true;
    let mut device = None;
    for line in text.lines().map(str::trim) {
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s;
        } else if section == "StreamInputs" {
            match line.split_once('=') {
                Some(("useDefaultInputDevice", v)) => use_default = v.trim() != "false",
                Some(("inputDevice", v)) if !v.trim().is_empty() => device = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    // EasyEffects takes the default too when no device is set.
    if use_default { None } else { device }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_input_device() {
        let follows = "[StreamInputs]\ninputDevice=alsa_input.mic\n[StreamOutputs]\noutputDevice=x\n";
        assert_eq!(parse_input(follows), None);
        let fixed = "[StreamInputs]\nuseDefaultInputDevice=false\ninputDevice=alsa_input.mic\n";
        assert_eq!(parse_input(fixed), Some("alsa_input.mic".into()));
        let other_section = "[StreamOutputs]\nuseDefaultInputDevice=false\ninputDevice=x\n";
        assert_eq!(parse_input(other_section), None);
    }
}
