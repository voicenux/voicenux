//! The list of effect blocks a voice is made of, with their parameters.
//! The order of `params` is the index each effect's `set_param` expects.

use super::{BuildCtx, Effect};
use super::{ambience, autotune, dynamics, modulation, noise, pitch, space, spectral, tone, vocoder};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unit {
    None,
    Semitones,
    Cents,
    Hz,
    Ms,
    Seconds,
    Db,
    Percent,
    Note,
    PerSecond,
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Float,
    Int,
    Toggle,
    Choice(&'static [&'static str]),
}

#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: Unit,
    pub kind: Kind,
    /// Shown on the voice page. The rest only in Voicelab.
    pub main: bool,
    /// Slider moves on a log scale (frequencies, times).
    pub log: bool,
}

const fn p(key: &'static str, label: &'static str, min: f32, max: f32, default: f32, unit: Unit) -> ParamSpec {
    ParamSpec { key, label, min, max, default, unit, kind: Kind::Float, main: false, log: false }
}

const fn pct(key: &'static str, label: &'static str, default: f32) -> ParamSpec {
    p(key, label, 0.0, 1.0, default, Unit::Percent)
}

const fn choice(key: &'static str, label: &'static str, list: &'static [&'static str], default: f32) -> ParamSpec {
    ParamSpec { kind: Kind::Choice(list), ..p(key, label, 0.0, (list.len() - 1) as f32, default, Unit::None) }
}

const fn toggle(key: &'static str, label: &'static str, default: bool) -> ParamSpec {
    ParamSpec { kind: Kind::Toggle, ..p(key, label, 0.0, 1.0, if default { 1.0 } else { 0.0 }, Unit::None) }
}

impl ParamSpec {
    const fn main(mut self) -> Self {
        self.main = true;
        self
    }

    const fn log(mut self) -> Self {
        self.log = true;
        self
    }

    const fn int(mut self) -> Self {
        self.kind = Kind::Int;
        self
    }

    pub fn clamp(&self, v: f32) -> f32 {
        let v = v.clamp(self.min, self.max);
        match self.kind {
            Kind::Float => v,
            _ => v.round(),
        }
    }

    #[cfg(test)]
    pub fn choices(&self) -> Option<&'static [&'static str]> {
        match self.kind {
            Kind::Choice(c) => Some(c),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Voice,
    Space,
    Motion,
    Tone,
    Texture,
}

impl Group {
    pub const ALL: [Group; 5] = [Group::Voice, Group::Space, Group::Motion, Group::Tone, Group::Texture];

    pub fn label(self) -> &'static str {
        match self {
            Group::Voice => "Voice",
            Group::Space => "Space",
            Group::Motion => "Motion",
            Group::Tone => "Tone",
            Group::Texture => "Sounds",
        }
    }
}

pub struct BlockSpec {
    pub kind: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    /// Picture from `data/icons`.
    pub icon: &'static str,
    pub group: Group,
    pub params: &'static [ParamSpec],
    pub build: fn(&[f32], &BuildCtx) -> Box<dyn Effect>,
}

impl BlockSpec {
    #[cfg(test)]
    pub fn defaults(&self) -> Vec<f32> {
        self.params.iter().map(|p| p.default).collect()
    }

    pub fn param_index(&self, key: &str) -> Option<usize> {
        self.params.iter().position(|p| p.key == key)
    }
}

pub fn find(kind: &str) -> Option<&'static BlockSpec> {
    BLOCKS.iter().find(|b| b.kind == kind)
}

macro_rules! boxed {
    ($t:path) => {
        |p, c| Box::new($t(p, c))
    };
}

pub static BLOCKS: &[BlockSpec] = &[
    BlockSpec {
        kind: "pitch",
        name: "Pitch",
        about: "Higher or lower voice. Formant changes the size of the throat.",
        icon: "control-knobs",
        group: Group::Voice,
        params: &[
            p("semitones", "Pitch", -24.0, 24.0, 0.0, Unit::Semitones).main(),
            p("formant", "Formant", -12.0, 12.0, 0.0, Unit::Semitones).main(),
            pct("mix", "Mix", 1.0),
        ],
        build: boxed!(pitch::Pitch::new),
    },
    BlockSpec {
        kind: "harmony",
        name: "Harmony",
        about: "Extra voices on top of yours: choirs, demons and duets.",
        icon: "angel",
        group: Group::Voice,
        params: &[
            p("voice_a", "First voice", -24.0, 24.0, 12.0, Unit::Semitones).main(),
            p("voice_b", "Second voice", -24.0, 24.0, 0.0, Unit::Semitones),
            p("voice_c", "Third voice", -24.0, 24.0, 0.0, Unit::Semitones),
            pct("level", "Extra voices", 0.7).main(),
            pct("dry", "Your voice", 1.0),
            pct("follow", "Formant follows", 0.0),
            p("detune", "Detune", 0.0, 50.0, 0.0, Unit::Cents),
        ],
        build: boxed!(pitch::Harmony::new),
    },
    BlockSpec {
        kind: "robot",
        name: "Robot",
        about: "Turns your voice into a flat, buzzing machine voice.",
        icon: "robot",
        group: Group::Voice,
        params: &[p("pitch", "Robot pitch", 40.0, 400.0, 110.0, Unit::Hz).main().log(), pct("mix", "Mix", 1.0)],
        build: boxed!(spectral::Spectral::robot),
    },
    BlockSpec {
        kind: "whisper",
        name: "Whisper",
        about: "Takes the tone out of your voice so only breath is left.",
        icon: "shushing",
        group: Group::Voice,
        params: &[pct("mix", "Whisper", 1.0).main()],
        build: boxed!(spectral::Spectral::whisper),
    },
    BlockSpec {
        kind: "autotune",
        name: "Auto-Tune",
        about: "Pulls every note to the nearest note of a scale.",
        icon: "microphone",
        group: Group::Voice,
        params: &[
            choice("key", "Key", autotune::KEYS, 0.0),
            choice("scale", "Scale", autotune::SCALES, 1.0),
            pct("speed", "Hardness", 0.9).main(),
            p("shift", "Transpose", -12.0, 12.0, 0.0, Unit::Semitones),
            p("formant", "Formant", -12.0, 12.0, 0.0, Unit::Semitones),
            pct("mix", "Mix", 1.0),
        ],
        build: boxed!(autotune::AutoTune::new),
    },
    BlockSpec {
        kind: "vocoder",
        name: "Vocoder",
        about: "Your voice plays a synthesizer.",
        icon: "keyboard",
        group: Group::Voice,
        params: &[
            choice("carrier", "Synth plays", vocoder::CARRIERS, 0.0),
            p("note", "Note", 24.0, 72.0, 48.0, Unit::Note).int(),
            choice("chord", "Chord", vocoder::CHORDS, 0.0),
            choice("progression", "Chord changes", vocoder::PROGRESSIONS, 0.0),
            p("chord_time", "Chord length", 0.5, 8.0, 2.0, Unit::Seconds),
            pct("clarity", "Clarity", 0.15),
            pct("mix", "Mix", 1.0).main(),
        ],
        build: boxed!(vocoder::Vocoder::new),
    },
    BlockSpec {
        kind: "echo",
        name: "Echo",
        about: "Repeats what you say.",
        icon: "mountain",
        group: Group::Space,
        params: &[
            p("time", "Echo time", 20.0, 2000.0, 350.0, Unit::Ms).main().log(),
            pct("feedback", "Repeats", 0.4).main(),
            pct("mix", "Echo volume", 0.4),
            p("tone", "Brightness", 500.0, 16000.0, 5000.0, Unit::Hz).log(),
        ],
        build: boxed!(space::Echo::new),
    },
    BlockSpec {
        kind: "reverb",
        name: "Reverb",
        about: "Puts you in a room, a hall or a cave.",
        icon: "church",
        group: Group::Space,
        params: &[
            pct("size", "Room size", 0.6).main(),
            pct("damping", "Damping", 0.4),
            pct("mix", "Reverb", 0.35).main(),
            p("predelay", "Pre-delay", 0.0, 200.0, 20.0, Unit::Ms),
        ],
        build: boxed!(space::Reverb::new),
    },
    BlockSpec {
        kind: "chorus",
        name: "Chorus",
        about: "Makes one voice sound like a few.",
        icon: "bunny-ears",
        group: Group::Motion,
        params: &[
            p("rate", "Speed", 0.05, 5.0, 0.8, Unit::Hz).log(),
            p("depth", "Depth", 0.0, 10.0, 3.0, Unit::Ms),
            p("delay", "Delay", 5.0, 30.0, 14.0, Unit::Ms),
            pct("mix", "Chorus", 0.5),
        ],
        build: boxed!(modulation::ModDelay::chorus),
    },
    BlockSpec {
        kind: "flanger",
        name: "Flanger",
        about: "A sweeping jet sound.",
        icon: "rocket",
        group: Group::Motion,
        params: &[
            p("rate", "Speed", 0.05, 5.0, 0.25, Unit::Hz).log(),
            p("depth", "Depth", 0.0, 8.0, 2.0, Unit::Ms),
            p("feedback", "Feedback", -0.95, 0.95, 0.6, Unit::Percent),
            pct("mix", "Flanger", 0.5),
        ],
        build: boxed!(modulation::ModDelay::flanger),
    },
    BlockSpec {
        kind: "phaser",
        name: "Phaser",
        about: "A swirling, spacey sweep.",
        icon: "cyclone",
        group: Group::Motion,
        params: &[
            p("rate", "Speed", 0.05, 5.0, 0.4, Unit::Hz).log(),
            pct("depth", "Depth", 0.8),
            p("feedback", "Feedback", -0.9, 0.9, 0.5, Unit::Percent),
            pct("mix", "Phaser", 0.5),
        ],
        build: boxed!(modulation::Phaser::new),
    },
    BlockSpec {
        kind: "vibrato",
        name: "Vibrato",
        about: "Makes the pitch wobble or shake.",
        icon: "woozy",
        group: Group::Motion,
        params: &[
            p("rate", "Wobble speed", 0.1, 15.0, 5.5, Unit::Hz).main().log(),
            p("depth", "Wobble", 0.0, 200.0, 30.0, Unit::Cents).main(),
            pct("random", "Shakiness", 0.0),
        ],
        build: boxed!(modulation::Vibrato::new),
    },
    BlockSpec {
        kind: "tremolo",
        name: "Tremolo",
        about: "Makes the volume pulse, like talking into a fan.",
        icon: "high-voltage",
        group: Group::Motion,
        params: &[
            p("rate", "Pulse speed", 0.1, 40.0, 6.0, Unit::Hz).main().log(),
            pct("depth", "Pulse", 0.6).main(),
            toggle("square", "Hard pulses", false),
        ],
        build: boxed!(modulation::Tremolo::new),
    },
    BlockSpec {
        kind: "ringmod",
        name: "Ring modulator",
        about: "A metal, alien sound.",
        icon: "alien",
        group: Group::Tone,
        params: &[
            p("freq", "Ring tone", 1.0, 2000.0, 60.0, Unit::Hz).main().log(),
            pct("wobble", "Ring wobble", 0.0),
            pct("mix", "Metal", 1.0).main(),
        ],
        build: boxed!(modulation::RingMod::new),
    },
    BlockSpec {
        kind: "distortion",
        name: "Distortion",
        about: "Crunch, grit and growl.",
        icon: "fire",
        group: Group::Tone,
        params: &[
            p("drive", "Grit", 0.0, 40.0, 12.0, Unit::Db).main(),
            choice("type", "Type", tone::DRIVE_TYPES, 0.0),
            p("tone", "Brightness", 500.0, 16000.0, 6000.0, Unit::Hz).log(),
            pct("mix", "Mix", 1.0),
            p("output", "Volume", -24.0, 12.0, 0.0, Unit::Db),
        ],
        build: boxed!(tone::Distortion::new),
    },
    BlockSpec {
        kind: "bitcrush",
        name: "Bit crusher",
        about: "Old video game and cheap digital sound.",
        icon: "video-game",
        group: Group::Tone,
        params: &[
            p("bits", "Bits", 2.0, 16.0, 8.0, Unit::None).main().int(),
            p("rate", "Sample rate", 1000.0, 48000.0, 8000.0, Unit::Hz).log(),
            pct("mix", "Mix", 1.0),
        ],
        build: boxed!(tone::Bitcrush::new),
    },
    BlockSpec {
        kind: "filter",
        name: "Filter",
        about: "Cuts or boosts some frequencies.",
        icon: "control-knobs",
        group: Group::Tone,
        params: &[
            choice("type", "Type", tone::FILTER_TYPES, 0.0),
            p("freq", "Frequency", 20.0, 20000.0, 1000.0, Unit::Hz).log(),
            p("q", "Resonance", 0.1, 20.0, 0.707, Unit::None).log(),
            p("gain", "Boost", -24.0, 24.0, 0.0, Unit::Db),
        ],
        build: boxed!(tone::Filter::new),
    },
    BlockSpec {
        kind: "band",
        name: "Band limit",
        about: "Only the middle of your voice gets through, like on a phone.",
        icon: "telephone",
        group: Group::Tone,
        params: &[
            p("low", "Low cut", 20.0, 2000.0, 300.0, Unit::Hz).log(),
            p("high", "High cut", 1000.0, 20000.0, 3400.0, Unit::Hz).log(),
            p("boost", "Mid boost", -12.0, 18.0, 0.0, Unit::Db),
        ],
        build: boxed!(tone::Band::new),
    },
    BlockSpec {
        kind: "compressor",
        name: "Compressor",
        about: "Evens out loud and quiet parts.",
        icon: "level",
        group: Group::Tone,
        params: &[
            p("threshold", "Threshold", -60.0, 0.0, -20.0, Unit::Db),
            p("ratio", "Ratio", 1.0, 20.0, 4.0, Unit::None).log(),
            p("attack", "Attack", 0.1, 100.0, 5.0, Unit::Ms).log(),
            p("release", "Release", 10.0, 1000.0, 120.0, Unit::Ms).log(),
            p("makeup", "Gain", 0.0, 24.0, 4.0, Unit::Db),
        ],
        build: boxed!(dynamics::Compressor::new),
    },
    BlockSpec {
        kind: "gate",
        name: "Noise gate",
        about: "Silences everything below a level.",
        icon: "shushing",
        group: Group::Tone,
        params: &[
            p("threshold", "Threshold", -80.0, -10.0, -50.0, Unit::Db),
            p("release", "Release", 10.0, 1000.0, 150.0, Unit::Ms).log(),
        ],
        build: boxed!(dynamics::Gate::new),
    },
    BlockSpec {
        kind: "gain",
        name: "Volume",
        about: "Louder or quieter.",
        icon: "loudspeaker",
        group: Group::Tone,
        params: &[p("gain", "Volume", -24.0, 24.0, 0.0, Unit::Db)],
        build: boxed!(dynamics::Gain::new),
    },
    BlockSpec {
        kind: "noise",
        name: "Noise",
        about: "Static, hiss, crackle or hum on your voice.",
        icon: "radio",
        group: Group::Texture,
        params: &[
            choice("type", "Type", noise::NOISE_TYPES, 0.0),
            p("level", "Noise", -60.0, 0.0, -32.0, Unit::Db),
            pct("follow", "Only while talking", 0.0),
        ],
        build: boxed!(noise::Noise::new),
    },
    BlockSpec {
        kind: "ambience",
        name: "Background",
        about: "A sound behind your voice. Plays while Background is on.",
        icon: "storm",
        group: Group::Texture,
        params: &[
            choice("type", "Sound", ambience::AMBIENCES, 0.0),
            p("level", "Background", -40.0, 0.0, -16.0, Unit::Db).main(),
        ],
        build: boxed!(ambience::Ambience::new),
    },
    BlockSpec {
        kind: "beeps",
        name: "Radio beeps",
        about: "A beep or squelch when you start or stop talking.",
        icon: "antenna",
        group: Group::Texture,
        params: &[
            choice("type", "Sound", noise::BEEP_TYPES, 0.0),
            p("level", "Volume", -40.0, 0.0, -12.0, Unit::Db),
            toggle("start", "When you start", true),
            toggle("end", "When you stop", true),
        ],
        build: boxed!(noise::Beeps::new),
    },
    BlockSpec {
        kind: "stutter",
        name: "Glitch",
        about: "Repeats bits of what you say, like a bad connection.",
        icon: "television",
        group: Group::Texture,
        params: &[
            p("chance", "Glitches", 0.0, 5.0, 0.8, Unit::PerSecond).main(),
            p("length", "Length", 10.0, 300.0, 80.0, Unit::Ms).log(),
            p("repeats", "Repeats", 1.0, 8.0, 3.0, Unit::None).int(),
            pct("mix", "Mix", 1.0),
        ],
        build: boxed!(noise::Stutter::new),
    },
];

pub fn note_name(midi: f32) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let m = midi.round() as i32;
    format!("{}{}", NAMES[m.rem_euclid(12) as usize], m.div_euclid(12) - 1)
}

/// Text for a parameter value, like "350 ms" or "+3 st".
pub fn format_value(spec: &ParamSpec, v: f32) -> String {
    match spec.kind {
        Kind::Choice(list) => return list.get(v as usize).copied().unwrap_or("").to_string(),
        Kind::Toggle => return if v >= 0.5 { "On" } else { "Off" }.to_string(),
        _ => {}
    }
    match spec.unit {
        Unit::Semitones => {
            if v.abs() < 0.05 {
                "0".into()
            } else {
                format!("{:+.1}", v).trim_end_matches(".0").to_string()
            }
        }
        Unit::Cents => format!("{v:.0} ct"),
        Unit::Hz if v >= 1000.0 => format!("{:.1} kHz", v / 1000.0),
        Unit::Hz if v < 10.0 => format!("{v:.2} Hz"),
        Unit::Hz => format!("{v:.0} Hz"),
        Unit::Ms if v >= 1000.0 => format!("{:.2} s", v / 1000.0),
        Unit::Ms => format!("{v:.0} ms"),
        Unit::Seconds => format!("{v:.1} s"),
        Unit::Db => format!("{v:.0} dB"),
        Unit::Percent => format!("{:.0} %", v * 100.0),
        Unit::Note => note_name(v),
        Unit::PerSecond => format!("{v:.1}/s"),
        Unit::None if matches!(spec.kind, Kind::Int) => format!("{v:.0}"),
        Unit::None => format!("{v:.2}"),
    }
}

/// Reads a typed value back, like "350 ms", "1,2 kHz", "-3" or "C#4". A plain number counts in
/// the unit that is shown. None when the text is not a value.
pub fn parse_value(spec: &ParamSpec, text: &str) -> Option<f32> {
    let t = text.trim().replace(',', ".").to_lowercase();
    let t = t.as_str();
    let num = |s: &str| s.trim().trim_start_matches('+').parse::<f32>().ok().filter(|v| v.is_finite());
    let v = match spec.unit {
        Unit::Percent => num(t.trim_end_matches('%'))? / 100.0,
        Unit::Hz => match t.strip_suffix("khz") {
            Some(k) => num(k)? * 1000.0,
            None => num(t.trim_end_matches("hz"))?,
        },
        Unit::Ms => match (t.strip_suffix("ms"), t.strip_suffix('s')) {
            (Some(ms), _) => num(ms)?,
            (None, Some(s)) => num(s)? * 1000.0,
            _ => num(t)?,
        },
        Unit::Seconds => num(t.trim_end_matches('s'))?,
        Unit::Db => num(t.trim_end_matches("db"))?,
        Unit::Cents => num(t.trim_end_matches("ct"))?,
        Unit::Semitones => num(t.trim_end_matches("st"))?,
        Unit::PerSecond => num(t.trim_end_matches("/s"))?,
        Unit::Note => num(t).or_else(|| parse_note(t))?,
        Unit::None => num(t)?,
    };
    Some(spec.clamp(v))
}

/// "c#4", "bb3" or "a4" as a MIDI note.
fn parse_note(t: &str) -> Option<f32> {
    let mut chars = t.chars();
    let base = match chars.next()? {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (sharp, rest) = match (rest.strip_prefix('#'), rest.strip_prefix('b').filter(|r| !r.is_empty())) {
        (Some(r), _) => (1, r),
        (None, Some(r)) => (-1, r),
        _ => (0, rest),
    };
    let octave: i32 = rest.trim().parse().ok()?;
    Some(((octave + 1) * 12 + base + sharp) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::Ctx;

    #[test]
    fn typed_values_read_back() {
        for b in BLOCKS {
            for p in b.params.iter().filter(|p| matches!(p.kind, Kind::Float | Kind::Int)) {
                for v in [p.min, p.default, p.max] {
                    let shown = format_value(p, v);
                    let back = parse_value(p, &shown).unwrap_or_else(|| panic!("{}.{}: {shown}", b.kind, p.key));
                    assert_eq!(format_value(p, back), shown, "{}.{}", b.kind, p.key);
                }
            }
        }
        let find_param = |kind: &str, key: &str| *find(kind).unwrap().params.iter().find(|p| p.key == key).unwrap();
        let pitch = find_param("pitch", "semitones");
        assert_eq!(parse_value(&pitch, "-3"), Some(-3.0));
        assert_eq!(parse_value(&pitch, "+40"), Some(24.0));
        assert_eq!(parse_value(&pitch, "loud"), None);
        let mix = find_param("pitch", "mix");
        assert_eq!(parse_value(&mix, "37,5 %"), Some(0.375));
        let ring = find_param("ringmod", "freq");
        assert_eq!(parse_value(&ring, "1,5 kHz"), Some(1500.0));
        assert_eq!(parse_value(&ring, "90"), Some(90.0));
        assert_eq!(parse_note("c#4"), Some(61.0));
        assert_eq!(parse_note("bb3"), Some(58.0));
        assert_eq!(parse_note("a4"), Some(69.0));
    }

    #[test]
    fn every_block_builds_and_runs() {
        let ctx = BuildCtx::new(48000.0, false);
        for b in BLOCKS {
            let mut e = (b.build)(&b.defaults(), &ctx);
            let mut x: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
            for _ in 0..50 {
                e.process(&mut x, &Ctx::default());
            }
            assert!(x.iter().all(|v| v.is_finite()), "{} made NaN", b.kind);
            for (i, p) in b.params.iter().enumerate() {
                e.set_param(i, p.max);
                e.set_param(i, p.min);
                e.set_param(i, p.default);
                assert!(p.default >= p.min && p.default <= p.max, "{}.{}", b.kind, p.key);
            }
        }
    }

    #[test]
    fn kinds_are_unique() {
        for (i, a) in BLOCKS.iter().enumerate() {
            for b in &BLOCKS[i + 1..] {
                assert_ne!(a.kind, b.kind);
            }
        }
    }

    #[test]
    fn formats() {
        let s = &find("pitch").unwrap().params[0];
        assert_eq!(format_value(s, 7.0), "+7");
        assert_eq!(format_value(s, -3.5), "-3.5");
        assert_eq!(note_name(60.0), "C4");
    }
}
