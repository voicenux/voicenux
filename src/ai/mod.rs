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
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Model {
    pub id: &'static str,
    /// Also its name in the voice list.
    pub name: &'static str,
    repo: &'static str,
    commit: &'static str,
    /// The character's typical pitch in Hz. The helper moves your typical pitch there.
    pub pitch: f32,
    /// The model's file in the repo, and its index if we get that too. Only where the index made
    /// the voice clearly more like the character: some are hundreds of MB.
    pth: &'static str,
    index: Option<&'static str>,
    /// About how big the download is, in MB.
    pub mb: u32,
}

// Pitches and indexes were picked by comparing the voices with recordings of the characters.
pub static MODELS: &[Model] = &[
    m(
        "spongebob",
        "SpongeBob",
        "binant/spongebob_squarepants",
        "b18b978411f2eef0b3091954adcb8cb8e44c69ed",
        230.0,
        true,
        220,
    ),
    m(
        "patrick",
        "Patrick",
        "binant/Patrick_Star__RVC_v2_",
        "f2995ee3d2a03af1f4ec37cedb0fa37d20258f78",
        150.0,
        false,
        55,
    ),
    m(
        "squidward",
        "Squidward",
        "binant/squidward_tentacles",
        "16214a28c9e89576f180d9f8579d17d596e559bf",
        140.0,
        false,
        55,
    ),
    m(
        "plankton",
        "Plankton",
        "binant/plankton_spongebob",
        "e1506b932883e4faf7fcc492f02ba9b3d423fc24",
        170.0,
        false,
        55,
    ),
    m("vader", "Darth Vader", "binant/darth_vader-male", "8c10be58c06fc41ec94389ebb939309d2f4f5ee5", 108.0, true, 140),
    m("yoda", "Yoda", "binant/grand_master_yoda", "2b769fae9fc1c9ba3371f0873e5af8b7460a140c", 120.0, false, 55),
    m(
        "optimus",
        "Optimus Prime",
        "binant/optimus_prime-male",
        "85285365c4a2b44e84adfb3b538e516bc333da03",
        105.0,
        false,
        55,
    ),
    m("glados", "GLaDOS", "binant/glados_portal", "ff48705119ea34456ea243fbbe09c368ecaf0e98", 178.0, false, 55),
    m("rick", "Rick Sanchez", "binant/rick_sanchez", "3744d2e1f1438c4acd81bfe6f2dadf0c6cf198b1", 135.0, false, 55),
    // Other Homer models only reach 3.4 kHz, like a telephone.
    m(
        "homer",
        "Homer Simpson",
        "Sunwest/Homer_Simpson_300",
        "dceff7b632f95613e8e5beed01af753bb112b8a4",
        125.0,
        true,
        100,
    )
    .files("homer.pth", "added_IVF342_Flat_nprobe_1_homer_v2.index"),
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
    let index = if index { Some("model.index") } else { None };
    Model { id, name, repo, commit, pitch, pth: "model.pth", index, mb }
}

/// The folder of everything AI: Python, PyTorch, the base models and the voices. Several GB, so
/// it lives in the data folder, and Settings can remove it.
pub fn dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("voicenux").join("ai")
}

static CPU: AtomicBool = AtomicBool::new(false);

/// The setting "Run on the processor", for computers without a graphics card for the AI.
pub fn set_cpu(on: bool) {
    CPU.store(on, Ordering::Relaxed);
}

/// Runs the AI on the processor instead of the graphics card: slower, with more delay. The
/// setting, or `VOICENUX_AI_CPU=1` for tests.
pub fn on_cpu() -> bool {
    CPU.load(Ordering::Relaxed) || std::env::var_os("VOICENUX_AI_CPU").is_some()
}

/// The AI voices need a graphics card from [`card`], unless they run on the processor.
pub fn usable() -> bool {
    on_cpu() || card().is_some()
}

/// A graphics card the AI voices run on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Card {
    /// With the NVIDIA driver: PyTorch for CUDA.
    Nvidia,
    /// PyTorch for ROCm, which brings ROCm along: the kernel's amdgpu driver is enough.
    Amd(Amd),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Amd {
    /// Its place among the AMD chips, when there are several. ROCm would else also start the
    /// one in the processor, which it may not run.
    pub only: Option<usize>,
    /// The chip ROCm should take it for, when PyTorch has no code of its own for it.
    pub pass_as: Option<&'static str>,
}

/// The graphics card for the AI, looked for once. NVIDIA first: the voices were tried there.
pub fn card() -> Option<Card> {
    static CARD: OnceLock<Option<Card>> = OnceLock::new();
    *CARD.get_or_init(|| {
        if Path::new("/proc/driver/nvidia/version").exists() {
            return Some(Card::Nvidia);
        }
        let amd = pick_amd(&amd_chips());
        log::info!("AMD graphics card for the AI: {amd:?}");
        amd.map(Card::Amd)
    })
}

/// The AMD chips ROCm sees, in its order, as `gfx_target_version` and number of SIMD units.
fn amd_chips() -> Vec<(u32, u32)> {
    let Ok(dir) = std::fs::read_dir("/sys/class/kfd/kfd/topology/nodes") else { return Vec::new() };
    let mut nodes: Vec<(u32, String)> = dir
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_str()?.parse().ok()?;
            Some((n, std::fs::read_to_string(e.path().join("properties")).ok()?))
        })
        .collect();
    nodes.sort_by_key(|n| n.0);
    nodes
        .iter()
        .filter_map(|(_, props)| {
            let prop =
                |name| props.lines().find_map(|l| l.strip_prefix(name)?.strip_prefix(' ')?.trim().parse::<u32>().ok());
            // The processor itself is a node too, with 0.
            let target = prop("gfx_target_version").filter(|&t| t != 0)?;
            Some((target, prop("simd_count").unwrap_or(0)))
        })
        .collect()
}

/// What PyTorch for ROCm 7.2 does with a chip, by its `gfx_target_version` (110001 is gfx1101):
/// runs it, runs it as a close relative, or not at all.
fn rocm_runs(target: u32) -> Option<Option<&'static str>> {
    let (major, minor, step) = (target / 10000, target / 100 % 100, target % 100);
    // The chips the build has code for. gfx1103 (Radeon 780M) is listed too, but its BLAS
    // libraries are missing.
    const OWN: &[&str] = &[
        "gfx900", "gfx906", "gfx908", "gfx90a", "gfx942", "gfx950", "gfx1030", "gfx1100", "gfx1101", "gfx1102",
        "gfx1150", "gfx1151", "gfx1200", "gfx1201",
    ];
    if OWN.contains(&format!("gfx{major}{minor:x}{step:x}").as_str()) {
        return Some(None);
    }
    match (major, minor) {
        // RX 6700, 6600, 6500 and the Radeon 680M run the code of the RX 6800.
        (10, 3) => Some(Some("10.3.0")),
        // The Radeon 780M runs the code of the RX 7900.
        (11, 0) => Some(Some("11.0.0")),
        _ => None,
    }
}

/// The chip for the AI: one with code of its own first, then the biggest.
fn pick_amd(chips: &[(u32, u32)]) -> Option<Amd> {
    let (index, pass_as, _) = chips
        .iter()
        .enumerate()
        .filter_map(|(i, &(target, simds))| Some((i, rocm_runs(target)?, simds)))
        .max_by_key(|&(i, pass_as, simds)| (pass_as.is_none(), simds, std::cmp::Reverse(i)))?;
    Some(Amd { only: (chips.len() > 1).then_some(index), pass_as })
}

impl Model {
    /// For a repo whose files are not called model.pth and model.index.
    const fn files(mut self, pth: &'static str, index: &'static str) -> Self {
        self.pth = pth;
        self.index = Some(index);
        self
    }

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
        self.index.map(|_| self.file("index"))
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
            assert!(m.url(m.pth).starts_with("https://huggingface.co/"));
        }
    }

    #[test]
    fn picks_the_amd_chip() {
        let amd = |only, pass_as| Some(Amd { only, pass_as });
        assert_eq!(pick_amd(&[]), None);
        // RX 7900 XTX, RX 9070, MI210.
        assert_eq!(pick_amd(&[(110000, 192)]), amd(None, None));
        assert_eq!(pick_amd(&[(120001, 128)]), amd(None, None));
        assert_eq!(pick_amd(&[(90010, 416)]), amd(None, None));
        // RX 6600 and Radeon 780M pass as their big relatives.
        assert_eq!(pick_amd(&[(100302, 56)]), amd(None, Some("10.3.0")));
        assert_eq!(pick_amd(&[(110003, 24)]), amd(None, Some("11.0.0")));
        // RX 5700 and the Vega in older Ryzen: too old.
        assert_eq!(pick_amd(&[(101000, 80)]), None);
        assert_eq!(pick_amd(&[(90012, 16)]), None);
        // The RX 7800 next to the chip in a Ryzen 7000, and the bigger of two that need help.
        assert_eq!(pick_amd(&[(100306, 4), (110001, 120)]), amd(Some(1), None));
        assert_eq!(pick_amd(&[(110001, 120), (100306, 4)]), amd(Some(0), None));
        assert_eq!(pick_amd(&[(100306, 4), (100302, 56)]), amd(Some(1), Some("10.3.0")));
    }
}
