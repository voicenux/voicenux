//! AI voices: RVC models that turn a live voice into a character. A Python helper runs them
//! (`data/ai/helper.py`), installed with one click into a folder of its own (`install`). The
//! engine sends it the voice and gets the character back (`bridge`). The effect in the chain is
//! `dsp::ai`.
//!
//! The models come from Hugging Face, pinned to a commit. Only fictional characters: no real
//! people as themselves.

pub mod bridge;
pub mod install;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Model {
    pub id: &'static str,
    /// Also its name in the voice list.
    pub name: &'static str,
    repo: &'static str,
    commit: &'static str,
    /// The character's typical pitch in Hz. The helper moves your typical pitch there.
    pub pitch: f32,
    /// Whether to get the index too. Only where it made the voice clearly more like the
    /// character: some are hundreds of MB.
    index: bool,
    /// About how big the download is, in MB.
    pub mb: u32,
}

// Pitches and indexes were picked by comparing the voices with recordings of the characters.
pub static MODELS: &[Model] = &[
    m("spongebob", "SpongeBob", "binant/spongebob_squarepants", "b18b978411f2eef0b3091954adcb8cb8e44c69ed", 230.0, true, 220),
    m("patrick", "Patrick", "binant/Patrick_Star__RVC_v2_", "f2995ee3d2a03af1f4ec37cedb0fa37d20258f78", 150.0, false, 55),
    m("squidward", "Squidward", "binant/squidward_tentacles", "16214a28c9e89576f180d9f8579d17d596e559bf", 140.0, false, 55),
    m("plankton", "Plankton", "binant/plankton_spongebob", "e1506b932883e4faf7fcc492f02ba9b3d423fc24", 170.0, false, 55),
    m("vader", "Darth Vader", "binant/darth_vader-male", "8c10be58c06fc41ec94389ebb939309d2f4f5ee5", 108.0, true, 140),
    m("yoda", "Yoda", "binant/grand_master_yoda", "2b769fae9fc1c9ba3371f0873e5af8b7460a140c", 120.0, false, 55),
    m("optimus", "Optimus Prime", "binant/optimus_prime-male", "85285365c4a2b44e84adfb3b538e516bc333da03", 105.0, false, 55),
    m("glados", "GLaDOS", "binant/glados_portal", "ff48705119ea34456ea243fbbe09c368ecaf0e98", 178.0, false, 55),
    m("rick", "Rick Sanchez", "binant/rick_sanchez", "3744d2e1f1438c4acd81bfe6f2dadf0c6cf198b1", 135.0, false, 55),
    m("homer", "Homer Simpson", "binant/HomerSimpson2333333", "2ce40c1dd16db07c1cffc314a43a5542af93aa3f", 125.0, true, 75),
];

/// The names of [`MODELS`] in the same order, for the choice in Voicelab.
pub const NAMES: &[&str] = &[
    "SpongeBob",
    "Patrick",
    "Squidward",
    "Plankton",
    "Darth Vader",
    "Yoda",
    "Optimus Prime",
    "GLaDOS",
    "Rick Sanchez",
    "Homer Simpson",
];

const fn m(
    id: &'static str,
    name: &'static str,
    repo: &'static str,
    commit: &'static str,
    pitch: f32,
    index: bool,
    mb: u32,
) -> Model {
    Model { id, name, repo, commit, pitch, index, mb }
}

/// The folder of everything AI: Python, PyTorch, the base models and the voices. Several GB, so
/// it lives in the data folder, and Settings can remove it.
pub fn dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("voicenux").join("ai")
}

static CPU: AtomicBool = AtomicBool::new(false);

/// The setting "Run on the processor", for computers without an NVIDIA card.
pub fn set_cpu(on: bool) {
    CPU.store(on, Ordering::Relaxed);
}

/// Runs the AI on the processor instead of an NVIDIA card: slower, with more delay. The setting,
/// or `VOICENUX_AI_CPU=1` for tests.
pub fn on_cpu() -> bool {
    CPU.load(Ordering::Relaxed) || std::env::var_os("VOICENUX_AI_CPU").is_some()
}

/// The AI voices need an NVIDIA card with its driver, unless they run on the processor.
pub fn usable() -> bool {
    on_cpu() || Path::new("/proc/driver/nvidia/version").exists()
}

impl Model {
    /// The files carry the commit, so a voice that moves to another model downloads again.
    fn file(&self, ext: &str) -> PathBuf {
        dir().join("voices").join(format!("{}-{}.{ext}", self.id, &self.commit[..8]))
    }

    /// The model itself.
    pub fn path(&self) -> PathBuf {
        self.file("pth")
    }

    /// Speech features from the character's recordings, if the model uses them. They make it
    /// sound more like the character.
    pub fn index(&self) -> Option<PathBuf> {
        self.index.then(|| self.file("index"))
    }

    pub fn downloaded(&self) -> bool {
        self.path().is_file() && self.index().is_none_or(|p| p.is_file())
    }

    fn url(&self, file: &str) -> String {
        format!("https://huggingface.co/{}/resolve/{}/{file}", self.repo, self.commit)
    }
}

/// The model a voice uses, if it is an AI voice: its first AI block.
pub fn model_of(blocks: &[crate::dsp::chain::BlockValues]) -> Option<&'static Model> {
    blocks.iter().find(|b| b.kind == "ai" && b.enabled).and_then(|b| MODELS.get(b.values.first().copied()? as usize))
}

/// Whether a voice can play right now: the engine and its model are there.
pub fn ready(model: &Model) -> bool {
    install::installed() && model.downloaded()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_models() {
        assert_eq!(NAMES.len(), MODELS.len());
        for (i, m) in MODELS.iter().enumerate() {
            assert_eq!(NAMES[i], m.name);
            assert!(MODELS[..i].iter().all(|o| o.id != m.id), "{} twice", m.id);
            assert!(m.commit.len() == 40 && m.commit.chars().all(|c| c.is_ascii_hexdigit()), "{}", m.id);
            assert!(m.url("model.pth").starts_with("https://huggingface.co/"));
        }
    }
}
