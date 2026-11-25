//! AI voices: RVC models that turn a live voice into a character. A Python helper runs them
//! (`data/ai/helper.py`), installed with one click into a folder of its own (`install`). The
//! engine sends it the voice and gets the character back (`bridge`). The effect in the chain is
//! `dsp::ai`.
//!
//! The models come from Hugging Face, pinned to a commit. Only fictional characters: no real
//! people as themselves. More can be found there (`hub`): those voices name their model
//! themselves, see [`Source`].

pub mod bridge;
pub mod hub;
pub mod install;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

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
    /// Found on Hugging Face, not one of [`MODELS`]: `pth` may be a zip with the model inside.
    pub found: bool,
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
    Model { id, name, repo, commit, pitch, pth: "model.pth", index, mb, found: false }
}

/// The setting "Character's pitch" of the AI effect: where your typical pitch moves. The usual
/// one is the character's own, or yours for a voice found online: its pitch is unknown.
pub const HEIGHTS: &[&str] = &["Usual", "Deep", "Medium", "High"];

/// The pitch in Hz of each of [`HEIGHTS`], 0 for the usual one.
pub fn height_hz(i: usize) -> f32 {
    [0.0, 110.0, 160.0, 230.0].get(i).copied().unwrap_or(0.0)
}

/// Where a voice found on Hugging Face gets its model. The voice keeps it in its AI effect, as
/// `model = { repo = "...", commit = "...", file = "..." }`, so a voice file someone gives you
/// brings its model along.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub repo: String,
    pub commit: String,
    /// The model in the repo: a .pth, or a .zip with one inside.
    pub file: String,
    /// The index next to a .pth. A zip brings its own.
    pub index: Option<String>,
    /// About how big the download is, in MB.
    pub mb: u32,
    /// What the voice was called where it was found.
    pub name: String,
}

impl Source {
    /// Reads one from a voice. Only what can be downloaded from Hugging Face, so a made up voice
    /// file can not point anywhere else.
    pub fn from_value(v: &toml::Value) -> Option<Self> {
        let t = v.as_table()?;
        let text = |key: &str| t.get(key).and_then(|v| v.as_str()).map(str::to_string);
        let s = Self {
            repo: text("repo")?,
            commit: text("commit")?.to_lowercase(),
            file: text("file")?,
            index: text("index"),
            mb: t.get("mb").and_then(|v| v.as_integer()).unwrap_or(0).clamp(0, 10_000) as u32,
            name: text("name").unwrap_or_default().chars().take(60).collect(),
        };
        let repo_ok = s.repo.len() <= 96
            && s.repo.split('/').count() == 2
            && s.repo.split('/').all(|p| {
                !p.is_empty()
                    && !p.starts_with('.')
                    && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            });
        let path_ok = |p: &str, ends: &[&str]| {
            let lower = p.to_lowercase();
            p.len() <= 300
                && !p.starts_with('/')
                && p.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
                && !p.chars().any(char::is_control)
                && ends.iter().any(|e| lower.ends_with(e))
        };
        let ok = repo_ok
            && s.commit.len() == 40
            && s.commit.chars().all(|c| c.is_ascii_hexdigit())
            && path_ok(&s.file, &[".pth", ".zip"])
            && s.index.as_deref().is_none_or(|i| path_ok(i, &[".index"]));
        ok.then_some(s)
    }

    pub fn to_value(&self) -> toml::Value {
        let mut t = toml::Table::new();
        t.insert("name".into(), self.name.clone().into());
        t.insert("repo".into(), self.repo.clone().into());
        t.insert("commit".into(), self.commit.clone().into());
        t.insert("file".into(), self.file.clone().into());
        if let Some(i) = &self.index {
            t.insert("index".into(), i.clone().into());
        }
        t.insert("mb".into(), toml::Value::Integer(self.mb as i64));
        toml::Value::Table(t)
    }

    /// The same model, whatever it is called.
    fn same(&self, m: &Model) -> bool {
        m.found
            && m.repo == self.repo
            && m.commit == self.commit
            && m.pth == self.file
            && m.index == self.index.as_deref()
    }

    /// A name for its files: the same for the same model, and never one of [`MODELS`].
    fn id(&self) -> String {
        // FNV-1a: the same on every computer and in every Rust.
        let key = format!("{}\n{}\n{}\n{}", self.repo, self.commit, self.file, self.index.as_deref().unwrap_or(""));
        let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
        format!("hf_{hash:016x}")
    }
}

/// The models found online that voices use, after [`MODELS`] in their numbering. They only
/// come and never go while Voicenux runs, so a number always means the same model.
static FOUND: Mutex<Vec<&'static Model>> = Mutex::new(Vec::new());

fn found() -> std::sync::MutexGuard<'static, Vec<&'static Model>> {
    FOUND.lock().unwrap_or_else(|e| e.into_inner())
}

/// The model with number `i`: one of [`MODELS`], then the ones found online.
pub fn model(i: usize) -> Option<&'static Model> {
    MODELS.get(i).or_else(|| found().get(i - MODELS.len()).copied())
}

/// All models, in their numbering, for the choice in Voicelab.
pub fn models() -> Vec<&'static Model> {
    MODELS.iter().chain(found().iter().copied()).collect()
}

/// The number of a model as a voice names it: one of [`MODELS`] by its name, or a [`Source`].
pub fn number_of(v: &toml::Value) -> Option<usize> {
    match v {
        toml::Value::String(s) => MODELS.iter().position(|m| m.name.eq_ignore_ascii_case(s) || m.id == s),
        toml::Value::Integer(i) => usize::try_from(*i).ok().filter(|i| *i < MODELS.len()),
        toml::Value::Float(f) if f.is_finite() && *f >= 0.0 => Some(*f as usize).filter(|i| *i < MODELS.len()),
        toml::Value::Table(_) => Some(add(&Source::from_value(v)?)),
        _ => None,
    }
}

/// How a voice names the model with number `i`.
pub fn value_of(i: usize) -> toml::Value {
    match model(i) {
        Some(m) if m.found => m.source().to_value(),
        Some(m) => m.name.into(),
        None => MODELS[0].name.into(),
    }
}

/// The number of a model found online. Known ones keep theirs.
pub fn add(source: &Source) -> usize {
    let mut found = found();
    if let Some(i) = found.iter().position(|m| source.same(m)) {
        return MODELS.len() + i;
    }
    let name = if source.name.trim().is_empty() { source.repo.clone() } else { source.name.clone() };
    // Lives as long as Voicenux, like the built-in ones. Only a few each run.
    let m: &'static Model = Box::leak(Box::new(Model {
        id: source.id().leak(),
        name: name.leak(),
        repo: source.repo.clone().leak(),
        commit: source.commit.clone().leak(),
        pitch: 0.0,
        pth: source.file.clone().leak(),
        index: source.index.clone().map(|i| &*i.leak()),
        mb: source.mb.max(1),
        found: true,
    }));
    found.push(m);
    MODELS.len() + found.len() - 1
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

    /// The files carry the commit, so a voice that moves to another model downloads again. The
    /// id of a found one has it already.
    fn file(&self, ext: &str) -> PathBuf {
        let name =
            if self.found { format!("{}.{ext}", self.id) } else { format!("{}-{}.{ext}", self.id, &self.commit[..8]) };
        dir().join("voices").join(name)
    }

    /// The model itself.
    pub fn path(&self) -> PathBuf {
        self.file("pth")
    }

    /// Speech features from the character's recordings, if the model uses them. They make it
    /// sound more like the character. A found one has them if its download brought them.
    pub fn index(&self) -> Option<PathBuf> {
        if self.found {
            return Some(self.file("index")).filter(|p| p.is_file());
        }
        self.index.map(|_| self.file("index"))
    }

    pub fn downloaded(&self) -> bool {
        self.path().is_file() && self.index().is_none_or(|p| p.is_file())
    }

    fn url(&self, file: &str) -> String {
        hub::file_url(self.repo, self.commit, file)
    }

    /// Where a found one comes from.
    pub fn source(&self) -> Source {
        Source {
            repo: self.repo.into(),
            commit: self.commit.into(),
            file: self.pth.into(),
            index: self.index.map(str::to_string),
            mb: self.mb,
            name: self.name.into(),
        }
    }
}

/// The model a voice uses, if it is an AI voice: its first AI block.
pub fn model_of(blocks: &[crate::dsp::chain::BlockValues]) -> Option<&'static Model> {
    blocks.iter().find(|b| b.kind == "ai" && b.enabled).and_then(|b| model(b.values.first().copied()? as usize))
}

/// Whether a voice can play right now: the engine and its model are there.
pub fn ready(model: &Model) -> bool {
    install::installed() && model.downloaded()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_are_pinned() {
        for (i, m) in MODELS.iter().enumerate() {
            assert!(!m.found && !m.id.starts_with("hf_"), "{}", m.id);
            assert!(MODELS[..i].iter().all(|o| o.id != m.id), "{} twice", m.id);
            assert!(m.commit.len() == 40 && m.commit.chars().all(|c| c.is_ascii_hexdigit()), "{}", m.id);
            assert!(m.url(m.pth).starts_with("https://huggingface.co/"));
        }
    }

    fn source() -> Source {
        Source {
            repo: "someone/Mario_RVC".into(),
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            file: "Mario V2.zip".into(),
            index: None,
            mb: 60,
            name: "Mario".into(),
        }
    }

    #[test]
    fn found_models_keep_their_number() {
        let s = source();
        let i = number_of(&s.to_value()).unwrap();
        assert!(i >= MODELS.len());
        assert_eq!(number_of(&s.to_value()), Some(i));
        let m = model(i).unwrap();
        assert!(m.found && m.id.starts_with("hf_"));
        assert_eq!(m.source(), s);
        assert_eq!(value_of(i), s.to_value());
        // Another file is another model.
        let other = Source { file: "Luigi.zip".into(), ..source() };
        assert_ne!(number_of(&other.to_value()), Some(i));
        assert_eq!(number_of(&"Darth Vader".into()), Some(4));
        assert_eq!(value_of(4), toml::Value::from("Darth Vader"));
        assert_eq!(number_of(&toml::Value::Float(-1.0)), None);
        assert_eq!(number_of(&toml::Value::Float(f64::NAN)), None);
    }

    #[test]
    fn found_models_come_only_from_hugging_face() {
        let ok = |s: Source| Source::from_value(&s.to_value()).is_some();
        assert!(ok(source()));
        assert!(ok(Source {
            file: "weights/Mario.pth".into(),
            index: Some("logs/added_IVF.index".into()),
            ..source()
        }));
        assert!(!ok(Source { repo: "../etc".into(), ..source() }));
        assert!(!ok(Source { repo: "a/b/c".into(), ..source() }));
        assert!(!ok(Source { commit: "main".into(), ..source() }));
        assert!(!ok(Source { file: "../../x.zip".into(), ..source() }));
        assert!(!ok(Source { file: "/x.zip".into(), ..source() }));
        assert!(!ok(Source { file: "model.exe".into(), ..source() }));
        assert!(!ok(Source { index: Some("a/../b.index".into()), ..source() }));
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
