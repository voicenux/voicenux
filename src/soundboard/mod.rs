//! Soundboard: the list of sounds, loading audio files and the starter sounds.

pub mod online;
pub mod starter;

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
    /// Id of a starter sound from [`starter`]. Its audio is a download of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<String>,
    #[serde(default = "full")]
    pub volume: f32,
    #[serde(default)]
    pub mode: PlayMode,
    /// Where playing starts, in seconds.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start: f32,
    /// Where playing ends, in seconds. None plays to the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f32>,
}

fn full() -> f32 {
    1.0
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

impl Sound {
    pub fn is_trimmed(&self) -> bool {
        self.start > 0.0 || self.end.is_some()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Library {
    /// The starter pack this soundboard has seen, see [`starter::PACK`]. Soundboards from before
    /// the field had the first one.
    #[serde(default = "first_pack")]
    pub starter: u32,
    #[serde(default)]
    pub sound: Vec<Sound>,
}

fn first_pack() -> u32 {
    1
}

pub fn dir() -> PathBuf {
    crate::config::config_dir().join("sounds")
}

fn library_path() -> PathBuf {
    crate::config::config_dir().join("soundboard.toml")
}

impl Library {
    /// The saved soundboard, or the starter pack on first start.
    pub fn load() -> Self {
        match std::fs::read_to_string(library_path()) {
            Ok(text) => match toml::from_str::<Self>(&text) {
                Ok(mut lib) => {
                    if lib.upgrade() {
                        lib.save();
                    }
                    lib
                }
                Err(e) => {
                    log::warn!("soundboard.toml is broken: {e}");
                    Self::starter()
                }
            },
            Err(_) => Self::starter(),
        }
    }

    /// Brings a soundboard from an older starter pack up to date: the sounds that are new join
    /// at the end. Returns whether something changed.
    fn upgrade(&mut self) -> bool {
        if self.starter >= starter::PACK {
            return false;
        }
        // Pack 2 has other audio for the old ids, so their cuts no longer fit.
        if self.starter < 2 {
            for s in self.sound.iter_mut().filter(|s| s.builtin.is_some()) {
                s.start = 0.0;
                s.end = None;
            }
        }
        for b in starter::BUILTIN.iter().filter(|b| b.since > self.starter) {
            if self.find(b.id).is_none() {
                self.sound.push(starter_sound(b));
            }
        }
        self.starter = starter::PACK;
        true
    }

    pub fn starter() -> Self {
        Self { starter: starter::PACK, sound: starter::BUILTIN.iter().map(starter_sound).collect() }
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
        path.file_name().ok_or_else(|| anyhow::anyhow!("no file name"))?;
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let target = free_path(&dir, &stem, &ext);
        std::fs::copy(path, &target)?;
        let name = path.file_stem().unwrap_or_default().to_string_lossy().replace(['_', '-'], " ");
        let sound = Sound {
            id: self.new_id(),
            name: name.trim().to_string(),
            icon: "loudspeaker".into(),
            file: Some(target.file_name().unwrap().to_string_lossy().to_string()),
            builtin: None,
            volume: 1.0,
            mode: PlayMode::Restart,
            start: 0.0,
            end: None,
        };
        self.sound.push(sound.clone());
        Ok(sound)
    }

    /// Adds a sound from audio in memory, like a download. The file gets a safe name made from
    /// `name`.
    pub fn add_bytes(&mut self, name: &str, ext: &str, data: &[u8], icon: &str) -> anyhow::Result<Sound> {
        decode_bytes(data.to_vec(), ext)?;
        let dir = dir();
        std::fs::create_dir_all(&dir)?;
        let stem: String = name.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        let stem = stem.trim_matches('-');
        let target = free_path(&dir, if stem.is_empty() { "sound" } else { stem }, &format!(".{ext}"));
        std::fs::write(&target, data)?;
        let sound = Sound {
            id: self.new_id(),
            name: name.to_string(),
            icon: icon.to_string(),
            file: Some(target.file_name().unwrap().to_string_lossy().to_string()),
            builtin: None,
            volume: 1.0,
            mode: PlayMode::Restart,
            start: 0.0,
            end: None,
        };
        self.sound.push(sound.clone());
        Ok(sound)
    }

    /// Takes a sound out of the list, with where it was. Its file stays, for Undo.
    pub fn take(&mut self, id: &str) -> Option<(usize, Sound)> {
        let pos = self.sound.iter().position(|s| s.id == id)?;
        Some((pos, self.sound.remove(pos)))
    }

    /// Moves a sound next to another one, before or after it.
    pub fn move_next_to(&mut self, id: &str, target: &str, after: bool) {
        if id == target {
            return;
        }
        let Some((_, s)) = self.take(id) else { return };
        let to = self.sound.iter().position(|s| s.id == target).map(|p| p + after as usize).unwrap_or(self.sound.len());
        self.sound.insert(to, s);
    }
}

fn starter_sound(b: &starter::Builtin) -> Sound {
    Sound {
        id: b.id.to_string(),
        name: b.name.to_string(),
        icon: b.icon.to_string(),
        file: None,
        builtin: Some(b.id.to_string()),
        volume: 1.0,
        mode: PlayMode::Restart,
        start: 0.0,
        end: None,
    }
}

/// Deletes the file of a removed sound. Starter sounds keep theirs, for when they come back.
pub fn delete_file(sound: &Sound) {
    if let Some(file) = &sound.file {
        let _ = std::fs::remove_file(dir().join(file));
    }
}

impl Library {
}

/// `dir/stem.ext`, or with a number added when that file is there already.
fn free_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut target = dir.join(format!("{stem}{ext}"));
    let mut n = 2;
    while target.exists() {
        target = dir.join(format!("{stem}-{n}{ext}"));
        n += 1;
    }
    target
}

/// Loads the audio of a sound.
pub fn load(sound: &Sound) -> anyhow::Result<Arc<SoundData>> {
    if let Some(b) = &sound.builtin {
        return starter::load(b).map(Arc::new).ok_or_else(|| anyhow::anyhow!("{b} is not downloaded yet"));
    }
    let file = sound.file.as_ref().ok_or_else(|| anyhow::anyhow!("sound has no file"))?;
    decode(&dir().join(file)).map(Arc::new)
}

/// The part of a sound from `start` to `end`, in seconds. Where it is cut, it fades in or out
/// over 5 ms, so the cut does not click.
pub fn trim(data: &SoundData, start: f32, end: Option<f32>) -> SoundData {
    let len = data.frames.len();
    let at = |s: f32| ((s.max(0.0) * RATE as f32) as usize).min(len);
    let a = at(start);
    let b = end.map_or(len, at).max(a);
    let mut frames = data.frames[a..b].to_vec();
    let n = frames.len();
    let fade = (RATE as usize / 200).min(n / 2);
    for i in 0..fade {
        let g = i as f32 / fade as f32;
        if a > 0 {
            frames[i] = frames[i].map(|x| x * g);
        }
        if b < len {
            frames[n - 1 - i] = frames[n - 1 - i].map(|x| x * g);
        }
    }
    SoundData { frames }
}

/// The loudest sample in each of `n` equal parts of a sound, from 0 to 1, for drawing it.
pub fn peaks(data: &SoundData, n: usize) -> Vec<f32> {
    let len = data.frames.len();
    (0..n)
        .map(|i| {
            let (a, b) = (i * len / n, ((i + 1) * len / n).max(i * len / n + 1).min(len));
            data.frames.get(a..b).map_or(0.0, |part| part.iter().fold(0.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs())))
        })
        .map(|p| p.min(1.0))
        .collect()
}

/// Decodes any file Symphonia understands into stereo at 48 kHz.
pub fn decode(path: &Path) -> anyhow::Result<SoundData> {
    let file = std::fs::File::open(path)?;
    decode_source(Box::new(file), path.extension().and_then(|e| e.to_str()))
}

/// The same for audio that is in memory already, like a download.
pub fn decode_bytes(data: Vec<u8>, ext: &str) -> anyhow::Result<SoundData> {
    decode_source(Box::new(std::io::Cursor::new(data)), Some(ext))
}

fn decode_source(source: Box<dyn symphonia::core::io::MediaSource>, ext: Option<&str>) -> anyhow::Result<SoundData> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext {
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
    const TAPS: usize = 2 * HALF as usize;
    let ratio = to as f64 / from as f64;
    let cutoff = ratio.min(1.0) * 0.94;
    let out_len = (input.len() as f64 * ratio).ceil() as usize;
    // Output j sits at j * down / up in the input, so its fraction takes one of `up` values. The
    // filter for each one is worked out once: working it out for every sample took half a second
    // for a ten-second clip. Odd rates with very many fractions round to the nearest of 4096.
    let g = gcd(from, to);
    let (up, down) = ((to / g) as u64, (from / g) as u64);
    let phases = up.min(4096);
    let table: Vec<[f64; TAPS]> = (0..phases)
        .map(|p| {
            let frac = p as f64 / phases as f64;
            std::array::from_fn(|t| {
                let x = (t as i64 - HALF + 1) as f64 - frac;
                let sinc = if x.abs() < 1e-9 {
                    cutoff
                } else {
                    (std::f64::consts::PI * x * cutoff).sin() / (std::f64::consts::PI * x)
                };
                let w = 0.42
                    + 0.5 * (std::f64::consts::PI * x / HALF as f64).cos()
                    + 0.08 * (2.0 * std::f64::consts::PI * x / HALF as f64).cos();
                sinc * w
            })
        })
        .collect();
    let mut out = Vec::with_capacity(out_len);
    for j in 0..out_len as u64 {
        let (mut base, mut phase) = ((j * down / up) as i64, j * down % up);
        if phases < up {
            phase = (phase * phases + up / 2) / up;
            if phase == phases {
                base += 1;
                phase = 0;
            }
        }
        let coeffs = &table[phase as usize];
        let mut acc = [0.0f64; 2];
        let mut norm = 0.0f64;
        for (t, c) in coeffs.iter().enumerate() {
            let k = base - HALF + 1 + t as i64;
            if k < 0 || k as usize >= input.len() {
                continue;
            }
            norm += c;
            acc[0] += input[k as usize][0] as f64 * c;
            acc[1] += input[k as usize][1] as f64 * c;
        }
        let norm = if norm.abs() > 1e-9 { norm } else { 1.0 };
        out.push([(acc[0] / norm) as f32, (acc[1] / norm) as f32]);
    }
    out
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(lib: &Library) -> Vec<&str> {
        lib.sound.iter().map(|s| s.id.as_str()).collect()
    }

    #[test]
    fn old_soundboards_get_the_new_pack() {
        let text = r#"
            [[sound]]
            id = "drumroll"
            name = "Drum Roll"
            icon = "drum"
            builtin = "drumroll"
            start = 0.8
            end = 5.2

            [[sound]]
            id = "sound-15"
            name = "Mine"
            icon = "loudspeaker"
            file = "mine.mp3"
            start = 1.0
        "#;
        let mut lib: Library = toml::from_str(text).unwrap();
        assert_eq!(lib.starter, 1);
        assert!(lib.upgrade());
        // The old cut does not fit the new audio. Own sounds keep theirs.
        assert!(!lib.find("drumroll").unwrap().is_trimmed());
        assert_eq!(lib.find("sound-15").unwrap().start, 1.0);
        // The memes come, a classic that was removed stays away.
        assert!(lib.find("vine-boom").is_some());
        assert!(lib.find("airhorn").is_none());
        assert!(!lib.upgrade());
        let saved: Library = toml::from_str(&toml::to_string_pretty(&lib).unwrap()).unwrap();
        assert_eq!(saved.starter, starter::PACK);
    }

    #[test]
    fn sounds_move_next_to_others() {
        let mut lib = Library::starter();
        lib.sound.retain(|s| ["airhorn", "sad-trombone", "rimshot", "drumroll"].contains(&s.id.as_str()));
        assert_eq!(ids(&lib), ["airhorn", "sad-trombone", "rimshot", "drumroll"]);
        lib.move_next_to("airhorn", "rimshot", true);
        assert_eq!(ids(&lib), ["sad-trombone", "rimshot", "airhorn", "drumroll"]);
        lib.move_next_to("drumroll", "sad-trombone", false);
        assert_eq!(ids(&lib), ["drumroll", "sad-trombone", "rimshot", "airhorn"]);
        lib.move_next_to("rimshot", "rimshot", true);
        assert_eq!(ids(&lib), ["drumroll", "sad-trombone", "rimshot", "airhorn"]);
        let (pos, s) = lib.take("sad-trombone").expect("a sound");
        assert_eq!((pos, s.id.as_str()), (1, "sad-trombone"));
        assert_eq!(ids(&lib), ["drumroll", "rimshot", "airhorn"]);
        assert!(lib.take("nothing").is_none());
    }

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
    fn trims_with_soft_cuts() {
        let data = SoundData { frames: vec![[0.5, -0.5]; RATE as usize * 2] };
        let all = trim(&data, 0.0, None);
        assert_eq!(all.frames.len(), data.frames.len());
        assert_eq!(all.frames[0], [0.5, -0.5], "no fade where nothing is cut");
        let part = trim(&data, 0.5, Some(1.5));
        assert_eq!(part.frames.len(), RATE as usize);
        assert_eq!(part.frames[0], [0.0, 0.0], "fades in at the cut");
        assert!(part.frames[part.frames.len() - 1][0].abs() < 0.01, "fades out at the cut");
        assert_eq!(part.frames[RATE as usize / 2], [0.5, -0.5]);
        // Out of range or the wrong way round gives a short or empty sound, never a crash.
        assert!(trim(&data, 5.0, Some(9.0)).frames.is_empty());
        assert!(trim(&data, 1.5, Some(0.5)).frames.is_empty());
    }

    #[test]
    fn peaks_follow_the_loudness() {
        let mut frames = vec![[0.0f32, 0.0]; 1000];
        frames[750] = [0.2, -0.9];
        let p = peaks(&SoundData { frames }, 4);
        assert_eq!(p, vec![0.0, 0.0, 0.0, 0.9]);
        assert_eq!(peaks(&SoundData { frames: vec![] }, 3), vec![0.0; 3]);
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
