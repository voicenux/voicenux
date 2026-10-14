//! Soundboard: the list of sounds, loading audio files and the built-in sounds.

pub mod synth;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Decoded audio, stereo at [`RATE`].
pub struct SoundData {
    pub frames: Vec<[f32; 2]>,
}

pub const RATE: u32 = 48000;

impl SoundData {
    #[cfg(test)]
    pub fn seconds(&self) -> f32 {
        self.frames.len() as f32 / RATE as f32
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayMode {
    /// Press plays from the start, press while playing starts over.
    #[default]
    Restart,
    /// Press plays, press again stops.
    Toggle,
    /// Every press starts another copy on top.
    Overlap,
    /// Plays in a loop until pressed again.
    Loop,
}

impl PlayMode {
    pub const ALL: [PlayMode; 4] = [PlayMode::Restart, PlayMode::Toggle, PlayMode::Overlap, PlayMode::Loop];

    pub fn label(self) -> &'static str {
        match self {
            PlayMode::Restart => "Play from the start",
            PlayMode::Toggle => "Play and stop",
            PlayMode::Overlap => "Play on top",
            PlayMode::Loop => "Loop",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sound {
    pub id: String,
    pub name: String,
    pub icon: String,
    /// File in the soundboard folder, for imported sounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Name of a built-in sound from [`synth`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<String>,
    #[serde(default = "full")]
    pub volume: f32,
    #[serde(default)]
    pub mode: PlayMode,
}

fn full() -> f32 {
    1.0
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub sound: Vec<Sound>,
}

pub fn dir() -> PathBuf {
    crate::config::config_dir().join("sounds")
}

fn library_path() -> PathBuf {
    crate::config::config_dir().join("soundboard.toml")
}

impl Library {
    /// The saved soundboard, or the built-in starter pack on first start.
    pub fn load() -> Self {
        match std::fs::read_to_string(library_path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                log::warn!("soundboard.toml is broken: {e}");
                Self::starter()
            }),
            Err(_) => Self::starter(),
        }
    }

    pub fn starter() -> Self {
        Self {
            sound: synth::BUILTIN
                .iter()
                .map(|b| Sound {
                    id: b.id.to_string(),
                    name: b.name.to_string(),
                    icon: b.icon.to_string(),
                    file: None,
                    builtin: Some(b.id.to_string()),
                    volume: 1.0,
                    mode: PlayMode::Restart,
                })
                .collect(),
        }
    }

    pub fn save(&self) {
        let path = library_path();
        let result = std::fs::create_dir_all(path.parent().unwrap())
            .map_err(anyhow::Error::from)
            .and_then(|_| Ok(toml::to_string_pretty(self)?))
            .and_then(|t| Ok(std::fs::write(&path, t)?));
        if let Err(e) = result {
            log::error!("could not save the soundboard: {e}");
        }
    }

    pub fn find(&self, id: &str) -> Option<&Sound> {
        self.sound.iter().find(|s| s.id == id)
    }

    pub fn new_id(&self) -> String {
        let mut n = self.sound.len() + 1;
        loop {
            let id = format!("sound-{n}");
            if self.find(&id).is_none() {
                return id;
            }
            n += 1;
        }
    }

    /// Copies an audio file into the soundboard folder and adds it.
    pub fn import(&mut self, path: &Path) -> anyhow::Result<Sound> {
        // Decode first, so broken files are refused right away.
        decode(path)?;
        let dir = dir();
        std::fs::create_dir_all(&dir)?;
        let file_name = path.file_name().ok_or_else(|| anyhow::anyhow!("no file name"))?;
        let mut target = dir.join(file_name);
        let mut n = 2;
        while target.exists() {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            target = dir.join(format!("{stem}-{n}{ext}"));
            n += 1;
        }
        std::fs::copy(path, &target)?;
        let name = path.file_stem().unwrap_or_default().to_string_lossy().replace(['_', '-'], " ");
        let sound = Sound {
            id: self.new_id(),
            name: name.trim().to_string(),
            icon: "speaker".into(),
            file: Some(target.file_name().unwrap().to_string_lossy().to_string()),
            builtin: None,
            volume: 1.0,
            mode: PlayMode::Restart,
        };
        self.sound.push(sound.clone());
        Ok(sound)
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(pos) = self.sound.iter().position(|s| s.id == id) {
            let s = self.sound.remove(pos);
            if let Some(file) = s.file {
                let _ = std::fs::remove_file(dir().join(file));
            }
        }
    }
}

/// Loads the audio of a sound.
pub fn load(sound: &Sound) -> anyhow::Result<Arc<SoundData>> {
    if let Some(b) = &sound.builtin {
        return synth::render(b).map(Arc::new).ok_or_else(|| anyhow::anyhow!("unknown sound {b}"));
    }
    let file = sound.file.as_ref().ok_or_else(|| anyhow::anyhow!("sound has no file"))?;
    decode(&dir().join(file)).map(Arc::new)
}

/// Decodes any file Symphonia understands into stereo at 48 kHz.
pub fn decode(path: &Path) -> anyhow::Result<SoundData> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format =
        symphonia::default::get_probe().probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())?;
    let track = format.default_track(TrackType::Audio).ok_or_else(|| anyhow::anyhow!("no audio in this file"))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| anyhow::anyhow!("unknown audio format"))?
        .clone();
    let mut decoder = symphonia::default::get_codecs().make_audio_decoder(&params, &AudioDecoderOptions::default())?;

    let mut rate = params.sample_rate.unwrap_or(RATE);
    let mut channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(2).max(1);
    let mut interleaved: Vec<f32> = Vec::new();
    let mut tmp: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(symphonia::core::errors::Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        rate = buf.spec().rate();
        channels = buf.spec().channels().count().max(1);
        tmp.clear();
        buf.copy_to_vec_interleaved(&mut tmp);
        interleaved.extend_from_slice(&tmp);
        // Keep imports sane: ten minutes at most.
        if interleaved.len() > rate as usize * channels * 600 {
            break;
        }
    }
    if interleaved.is_empty() {
        anyhow::bail!("the file has no sound in it");
    }
    let frames: Vec<[f32; 2]> =
        interleaved.chunks(channels).map(|c| if c.len() >= 2 { [c[0], c[1]] } else { [c[0], c[0]] }).collect();
    Ok(SoundData { frames: resample(&frames, rate, RATE) })
}

/// Windowed sinc resampling, for files that are not at 48 kHz.
pub fn resample(input: &[[f32; 2]], from: u32, to: u32) -> Vec<[f32; 2]> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    const HALF: i64 = 24;
    let ratio = to as f64 / from as f64;
    let cutoff = ratio.min(1.0) * 0.94;
    let out_len = (input.len() as f64 * ratio).ceil() as usize;
    let mut out = Vec::with_capacity(out_len);
    for j in 0..out_len {
        let center = j as f64 / ratio;
        let base = center.floor() as i64;
        let mut acc = [0.0f64; 2];
        let mut norm = 0.0f64;
        for k in base - HALF + 1..=base + HALF {
            if k < 0 || k as usize >= input.len() {
                continue;
            }
            let x = k as f64 - center;
            let sinc = if x.abs() < 1e-9 {
                cutoff
            } else {
                (std::f64::consts::PI * x * cutoff).sin() / (std::f64::consts::PI * x)
            };
            let w = 0.42
                + 0.5 * (std::f64::consts::PI * x / HALF as f64).cos()
                + 0.08 * (2.0 * std::f64::consts::PI * x / HALF as f64).cos();
            let c = sinc * w;
            norm += c;
            acc[0] += input[k as usize][0] as f64 * c;
            acc[1] += input[k as usize][1] as f64 * c;
        }
        let norm = if norm.abs() > 1e-9 { norm } else { 1.0 };
        out.push([(acc[0] / norm) as f32, (acc[1] / norm) as f32]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_keeps_tone() {
        let from = 44100;
        let input: Vec<[f32; 2]> = (0..44100)
            .map(|i| {
                let v = (std::f32::consts::TAU * 1000.0 * i as f32 / from as f32).sin() * 0.5;
                [v, v]
            })
            .collect();
        let out = resample(&input, from, 48000);
        assert!((out.len() as i64 - 48000).abs() <= 1);
        let mono: Vec<f32> = out.iter().map(|f| f[0]).collect();
        let f = crate::dsp::analysis::dominant_freq(&mono[1000..40000], 48000.0);
        assert!((f - 1000.0).abs() < 5.0, "{f}");
        let peak = mono[1000..40000].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 0.02, "{peak}");
    }

    #[test]
    fn decodes_a_wav() {
        let dir = std::env::temp_dir().join(format!("voicenux-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        write_wav(&path, 22050, &(0..22050).map(|i| ((i as f32 * 0.1).sin() * 12000.0) as i16).collect::<Vec<_>>());
        let d = decode(&path).unwrap();
        assert!((d.frames.len() as i64 - 48000).abs() <= 2, "{}", d.frames.len());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    pub fn write_wav(path: &Path, rate: u32, samples: &[i16]) {
        let mut b = Vec::new();
        let data_len = samples.len() as u32 * 2;
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            b.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }
}
