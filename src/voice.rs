//! Voices: the built-in ones from `data/voices.toml` and the ones made in Voicelab.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dsp::blocks::{self, Kind};
use crate::dsp::chain::BlockValues;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Category {
    pub id: &'static str,
    pub label: &'static str,
    pub color: &'static str,
}

pub const CATEGORIES: &[Category] = &[
    Category { id: "characters", label: "Characters", color: "#ff8a5c" },
    Category { id: "horror", label: "Horror", color: "#f0445a" },
    Category { id: "scifi", label: "Sci-fi", color: "#22c3e6" },
    Category { id: "places", label: "Places", color: "#3ccf8e" },
    Category { id: "devices", label: "Devices", color: "#ffbf3c" },
    Category { id: "music", label: "Music", color: "#f45cc4" },
    Category { id: "funny", label: "Funny", color: "#a67cff" },
    Category { id: "ai", label: "AI", color: "#4f7bff" },
    Category { id: "custom", label: "My voices", color: "#7c8cff" },
];

pub fn category(id: &str) -> &'static Category {
    CATEGORIES.iter().find(|c| c.id == id).unwrap_or(&CATEGORIES[CATEGORIES.len() - 1])
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlockDef {
    #[serde(rename = "effect")]
    pub kind: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub params: BTreeMap<String, toml::Value>,
}

fn yes() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

impl BlockDef {
    pub fn new(kind: &str) -> Self {
        Self { kind: kind.to_string(), enabled: true, params: BTreeMap::new() }
    }

    /// Parameter values in the order the effect expects. Unknown or missing values
    /// fall back to the defaults.
    pub fn values(&self) -> Vec<f32> {
        let Some(spec) = blocks::find(&self.kind) else { return Vec::new() };
        spec.params
            .iter()
            .map(|p| {
                let v = self.params.get(p.key).and_then(|v| match (v, p.kind) {
                    (toml::Value::String(s), Kind::Choice(list)) => {
                        list.iter().position(|c| c.eq_ignore_ascii_case(s)).map(|i| i as f32)
                    }
                    (toml::Value::Boolean(b), _) => Some(if *b { 1.0 } else { 0.0 }),
                    (toml::Value::Integer(i), _) => Some(*i as f32),
                    (toml::Value::Float(f), _) => Some(*f as f32),
                    _ => None,
                });
                p.clamp(v.unwrap_or(p.default))
            })
            .collect()
    }

    /// Stores a value the way a person would write it in the file.
    pub fn set(&mut self, key: &str, v: f32) {
        let Some(spec) = blocks::find(&self.kind) else { return };
        let Some(p) = spec.params.iter().find(|p| p.key == key) else { return };
        let v = p.clamp(v);
        let value = match p.kind {
            Kind::Choice(list) => toml::Value::String(list[v as usize].to_string()),
            Kind::Toggle => toml::Value::Boolean(v >= 0.5),
            Kind::Int => toml::Value::Integer(v as i64),
            Kind::Float => {
                // Keep files readable: 0.35 instead of 0.3499999940395355.
                let rounded = (v as f64 * 1000.0).round() / 1000.0;
                toml::Value::Float(rounded)
            }
        };
        self.params.insert(key.to_string(), value);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default = "custom")]
    pub category: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub blocks: Vec<BlockDef>,
    #[serde(skip)]
    pub builtin: bool,
}

fn custom() -> String {
    "custom".into()
}

impl Voice {
    pub fn block_values(&self) -> Vec<BlockValues> {
        self.blocks
            .iter()
            .filter(|b| blocks::find(&b.kind).is_some())
            .map(|b| BlockValues { kind: b.kind.clone(), enabled: b.enabled, values: b.values() })
            .collect()
    }

    /// True when a background block is part of the voice.
    pub fn has_background(&self) -> bool {
        self.blocks.iter().any(|b| b.kind == "ambience" && b.enabled)
    }
}

#[derive(Deserialize)]
struct VoiceFile {
    voice: Vec<Voice>,
}

pub fn builtin() -> Vec<Voice> {
    let file: VoiceFile = toml::from_str(include_str!("../data/voices.toml")).expect("data/voices.toml is broken");
    file.voice
        .into_iter()
        .map(|mut v| {
            v.builtin = true;
            v
        })
        .collect()
}

pub fn custom_dir() -> PathBuf {
    crate::config::config_dir().join("voices")
}

pub fn load_custom(dir: &Path) -> Vec<Voice> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut voices: Vec<Voice> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            match toml::from_str::<Voice>(&text) {
                Ok(mut v) => {
                    v.category = "custom".into();
                    Some(v)
                }
                Err(err) => {
                    log::warn!("skipping {}: {err}", e.path().display());
                    None
                }
            }
        })
        .collect();
    voices.sort_by_key(|a| a.name.to_lowercase());
    voices
}

pub fn save_custom(dir: &Path, voice: &Voice) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let text = toml::to_string_pretty(voice)?;
    std::fs::write(dir.join(format!("{}.toml", voice.id)), text)?;
    Ok(())
}

pub fn delete_custom(dir: &Path, id: &str) {
    let _ = std::fs::remove_file(dir.join(format!("{id}.toml")));
}

/// An id for a new custom voice that no other voice has.
pub fn new_id(name: &str, taken: &[&str]) -> String {
    let base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let base = if base.is_empty() { "voice".to_string() } else { format!("my-{base}") };
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id.as_str()) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// The order of the voice page: by name, in your own order, or favorites first in the order you
/// starred them, then your own voices, then the built-in ones. Voices that came after you set
/// your order keep their usual place behind it. `name` is the name people read.
pub fn shown_order(voices: &[Voice], favorites: &[String], order: &[String], by_name: bool, name: impl Fn(&Voice) -> String) -> Vec<String> {
    if by_name {
        let mut ids: Vec<(String, String)> = voices.iter().map(|v| (name(v).to_lowercase(), v.id.clone())).collect();
        ids.sort();
        return ids.into_iter().map(|(_, id)| id).collect();
    }
    let mut usual: Vec<String> = favorites.iter().filter(|f| voices.iter().any(|v| &v.id == *f)).cloned().collect();
    let mut rest: Vec<&Voice> = voices.iter().filter(|v| !favorites.contains(&v.id)).collect();
    rest.sort_by_key(|v| v.builtin);
    usual.extend(rest.into_iter().map(|v| v.id.clone()));
    if order.is_empty() {
        return usual;
    }
    let mut ids: Vec<String> = order.iter().filter(|id| usual.contains(id)).cloned().collect();
    let known: std::collections::HashSet<String> = ids.iter().cloned().collect();
    ids.extend(usual.into_iter().filter(|id| !known.contains(id)));
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::chain::Chain;
    use crate::dsp::{BuildCtx, Ctx};

    #[test]
    fn the_voice_page_keeps_your_order() {
        let mut voices: Vec<Voice> = builtin().into_iter().take(3).collect();
        let ids: Vec<String> = voices.iter().map(|v| v.id.clone()).collect();
        let mut own = voices[0].clone();
        own.id = "mine".into();
        own.builtin = false;
        voices.push(own);
        let name = |v: &Voice| v.id.clone();
        // Favorites first, then your own voices, then the built-in ones.
        let favs = vec![ids[2].clone()];
        assert_eq!(shown_order(&voices, &favs, &[], false, name), [ids[2].as_str(), "mine", &ids[0], &ids[1]]);
        // Your order, with a voice it does not know yet behind, and gone ones left out.
        let order = vec![ids[1].clone(), "gone".into(), ids[0].clone(), "mine".into()];
        assert_eq!(shown_order(&voices, &favs, &order, false, name), [ids[1].as_str(), &ids[0], "mine", &ids[2]]);
        // By name.
        let mut sorted = ids.clone();
        sorted.push("mine".into());
        sorted.sort();
        assert_eq!(shown_order(&voices, &favs, &order, true, name), sorted);
    }

    #[test]
    fn builtin_voices_are_valid() {
        let voices = builtin();
        assert!(voices.len() >= 60);
        let icons = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons")).unwrap();
        let icons: Vec<String> =
            icons.flatten().map(|e| e.path().file_stem().unwrap().to_string_lossy().to_string()).collect();
        let mut ids = std::collections::HashSet::new();
        for v in &voices {
            assert!(ids.insert(v.id.clone()), "duplicate id {}", v.id);
            assert!(icons.contains(&v.icon), "{} has no icon {}", v.id, v.icon);
            assert!(CATEGORIES.iter().any(|c| c.id == v.category), "{}: bad category", v.id);
            assert!(!v.blocks.is_empty(), "{} has no blocks", v.id);
            for b in &v.blocks {
                let spec = blocks::find(&b.kind).unwrap_or_else(|| panic!("{}: no effect {}", v.id, b.kind));
                for (k, val) in &b.params {
                    let p = spec.params.iter().find(|p| p.key == k);
                    let p = p.unwrap_or_else(|| panic!("{}: {} has no setting {k}", v.id, b.kind));
                    if let (toml::Value::String(s), Kind::Choice(list)) = (val, p.kind) {
                        assert!(list.iter().any(|c| c.eq_ignore_ascii_case(s)), "{}: bad choice {s}", v.id);
                    }
                }
            }
        }
        for c in CATEGORIES.iter().filter(|c| c.id != "custom") {
            assert!(voices.iter().any(|v| v.category == c.id), "empty category {}", c.id);
        }
        for spec in blocks::BLOCKS {
            assert!(icons.iter().any(|i| i == spec.icon), "effect {} has no icon {}", spec.kind, spec.icon);
        }
    }

    /// A different voice than the one chains are leveled with: lower, other vowels.
    fn talk(len: usize) -> Vec<f32> {
        let rate = 48000.0;
        let mut bp = crate::dsp::biquad::Biquad::new(crate::dsp::biquad::FilterType::Bandpass, 500.0, 1.2, 0.0, rate);
        let mut ph = 0.0f32;
        (0..len)
            .map(|i| {
                let t = i as f32 / rate;
                let f0 = 105.0 + 15.0 * (t * 0.9).sin();
                ph = (ph + f0 / rate).fract();
                let saw = ph * 2.0 - 1.0;
                let syll = (0.5 + 0.5 * (t * std::f32::consts::TAU * 2.3).sin()).powi(3);
                bp.process(saw) * syll * 0.8
            })
            .collect()
    }

    #[test]
    fn every_voice_keeps_a_sane_level() {
        let input = talk(48000 * 4);
        let in_rms = crate::dsp::util::rms(&input);
        let ctx = BuildCtx::new(48000.0, false);
        for v in builtin() {
            let mut chain = Chain::build_leveled(&v.block_values(), &ctx);
            let mut x = input.clone();
            for block in x.chunks_mut(512) {
                chain.process(block, &Ctx { background: false });
            }
            assert!(x.iter().all(|s| s.is_finite()), "{} made NaN", v.id);
            let out = crate::dsp::util::rms(&x[48000..]);
            let db = crate::dsp::util::gain_to_db(out / in_rms);
            assert!((-5.0..5.0).contains(&db), "{} level {db:.1} dB", v.id);
        }
    }

    #[test]
    #[ignore]
    fn print_voice_levels() {
        let input = talk(48000 * 4);
        let in_rms = crate::dsp::util::rms(&input);
        let ctx = BuildCtx::new(48000.0, false);
        for v in builtin() {
            let mut chain = Chain::build_leveled(&v.block_values(), &ctx);
            let mut x = input.clone();
            for block in x.chunks_mut(512) {
                chain.process(block, &Ctx { background: false });
            }
            let out = crate::dsp::util::rms(&x[48000..]);
            let pk = crate::dsp::util::peak(&x[48000..]);
            println!("{:12} {:+6.1} dB  peak {:.2}", v.id, crate::dsp::util::gain_to_db(out / in_rms), pk);
        }
    }

    /// Plays a recording through every voice into WAV files, to listen to them:
    /// `VOICENUX_RENDER=speech.ogg:out/ cargo test --release render_voices -- --ignored`.
    /// Voices with a background get a second file with it.
    #[test]
    #[ignore]
    fn render_voices() {
        let Ok(arg) = std::env::var("VOICENUX_RENDER") else { return };
        let (input, dir) = arg.split_once(':').unwrap();
        let data = crate::soundboard::decode(std::path::Path::new(input)).unwrap();
        let input: Vec<f32> = data.frames.iter().map(|f| (f[0] + f[1]) * 0.5).collect();
        let ctx = BuildCtx::new(48000.0, false);
        for v in builtin().into_iter().filter(|v| !v.id.starts_with("ai-")) {
            for background in [false, true] {
                if background && !v.has_background() {
                    continue;
                }
                let mut chain = Chain::build_leveled(&v.block_values(), &ctx);
                let mut x = input.clone();
                for block in x.chunks_mut(512) {
                    chain.process(block, &Ctx { background });
                }
                let mut wav = Vec::new();
                let len = x.len() as u32 * 2;
                for part in [&b"RIFF"[..], &(36 + len).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes(), &1u16.to_le_bytes()] {
                    wav.extend_from_slice(part);
                }
                for part in [&1u16.to_le_bytes()[..], &48000u32.to_le_bytes(), &96000u32.to_le_bytes(), &2u16.to_le_bytes()] {
                    wav.extend_from_slice(part);
                }
                for part in [&16u16.to_le_bytes()[..], b"data", &len.to_le_bytes()] {
                    wav.extend_from_slice(part);
                }
                for s in x {
                    wav.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
                }
                let name = if background { format!("{}-bg", v.id) } else { v.id.clone() };
                std::fs::write(format!("{dir}/{name}.wav"), wav).unwrap();
            }
        }
    }

    #[test]
    fn set_and_read_back() {
        let mut b = BlockDef::new("filter");
        b.set("type", 4.0);
        b.set("freq", 1234.5678);
        assert_eq!(b.params["type"], toml::Value::String("Low shelf".into()));
        assert!((b.values()[1] - 1234.568).abs() < 0.01);
        let v = Voice {
            id: "x".into(),
            name: "X".into(),
            icon: "robot".into(),
            category: "custom".into(),
            description: String::new(),
            blocks: vec![b],
            builtin: false,
        };
        let text = toml::to_string_pretty(&v).unwrap();
        let back: Voice = toml::from_str(&text).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn ids_do_not_clash() {
        assert_eq!(new_id("Cool Voice!", &[]), "my-cool-voice");
        assert_eq!(new_id("Cool Voice", &["my-cool-voice"]), "my-cool-voice-2");
    }
}
