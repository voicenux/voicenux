//! Soundboard: the list of sounds, loading audio files and the starter sounds.

pub mod found;
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
}

impl PlayMode {
    pub const ALL: [PlayMode; 3] = [PlayMode::Restart, PlayMode::Toggle, PlayMode::Overlap];

    /// The name in the config and the API.
    pub fn id(self) -> &'static str {
        match self {
            PlayMode::Restart => "restart",
            PlayMode::Toggle => "toggle",
            PlayMode::Overlap => "overlap",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    /// A short name, for the buttons side by side under the soundboard.
    pub fn short(self) -> &'static str {
        match self {
            PlayMode::Restart => "From the start",
            PlayMode::Toggle => "Play/Stop",
            PlayMode::Overlap => "On top",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PlayMode::Restart => "Play from the start",
            PlayMode::Toggle => "Play and stop",
            PlayMode::Overlap => "Play on top",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            PlayMode::Restart => "media-skip-backward-symbolic",
            PlayMode::Toggle => "media-playback-start-symbolic",
            PlayMode::Overlap => "edit-copy-symbolic",
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
    /// Brought to the loudness of the starter sounds when it loads, see [`level`]. Sounds found on
    /// MyInstants are, since most are very loud there.
    #[serde(default, skip_serializing_if = "is_false")]
    pub level: bool,
    /// Where playing starts, in seconds.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start: f32,
    /// Where playing ends, in seconds. None plays to the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f32>,
    /// The folder it is in, by name. None for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// Starred, for a random favorite.
    #[serde(default, skip_serializing_if = "is_false")]
    pub favorite: bool,
    /// The file on MyInstants it came from, so it is not added twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

fn full() -> f32 {
    1.0
}

fn is_false(v: &bool) -> bool {
    !*v
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
    /// How sounds were given pictures when this soundboard last looked, see [`PICTURES`].
    #[serde(default)]
    pub pictures: u32,
    /// Folders by name, in the order of their tabs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
    #[serde(default)]
    pub sound: Vec<Sound>,
}

fn first_pack() -> u32 {
    1
}

/// Raised when sounds can find better pictures by their names, see [`crate::ui::emoji::guess`].
/// Sounds that still have the loudspeaker then look again, once.
pub const PICTURES: u32 = 1;

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
    /// at the end. Sounds with the loudspeaker look for a better picture. Returns whether
    /// something changed.
    fn upgrade(&mut self) -> bool {
        let mut changed = false;
        if self.pictures < PICTURES {
            for s in self.sound.iter_mut().filter(|s| s.builtin.is_none() && s.icon == "loudspeaker") {
                if let Some(icon) = crate::ui::emoji::guess(&s.name) {
                    s.icon = icon.to_string();
                }
            }
            self.pictures = PICTURES;
            changed = true;
        }
        if self.starter >= starter::PACK {
            return changed;
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
        Self {
            starter: starter::PACK,
            pictures: PICTURES,
            folders: Vec::new(),
            sound: starter::BUILTIN.iter().map(starter_sound).collect(),
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
    /// Adds a file in the soundboard folder as a new sound, see [`copy_in`].
    pub fn add_file(&mut self, file: String, name: &str, icon: &str) -> Sound {
        let sound = Sound {
            id: self.new_id(),
            name: name.trim().to_string(),
            icon: icon.to_string(),
            file: Some(file),
            builtin: None,
            volume: 1.0,
            level: false,
            start: 0.0,
            end: None,
            folder: None,
            favorite: false,
            source: None,
        };
        self.sound.push(sound.clone());
        sound
    }

    /// Adds a sound from audio in memory, like a download. The caller decoded it already, so it
    /// is known to play. The file gets a safe name made from `name`.
    pub fn add_bytes(&mut self, name: &str, ext: &str, data: &[u8], icon: &str) -> anyhow::Result<Sound> {
        let dir = dir();
        std::fs::create_dir_all(&dir)?;
        let target = free_path(&dir, &file_stem(name), &format!(".{ext}"));
        std::fs::write(&target, data)?;
        Ok(self.add_file(target.file_name().unwrap().to_string_lossy().to_string(), name, icon))
    }

    /// `name`, or with a number after it when a folder has that name already.
    pub fn free_folder_name(&self, name: &str) -> String {
        crate::order::free_name(&self.folders, name)
    }

    /// Renames a folder, and moves its sounds along. False when the name is empty or taken.
    pub fn rename_folder(&mut self, old: &str, new: &str) -> bool {
        let new = new.trim();
        if new.is_empty() || (new != old && self.folders.iter().any(|f| f == new)) {
            return false;
        }
        for f in self.folders.iter_mut().filter(|f| *f == old) {
            *f = new.to_string();
        }
        for s in self.sound.iter_mut().filter(|s| s.folder.as_deref() == Some(old)) {
            s.folder = Some(new.to_string());
        }
        true
    }

    /// Removes a folder. Its sounds stay, in no folder.
    pub fn remove_folder(&mut self, name: &str) {
        self.folders.retain(|f| f != name);
        for s in self.sound.iter_mut().filter(|s| s.folder.as_deref() == Some(name)) {
            s.folder = None;
        }
    }

    /// Moves a folder before or after another one. Folders stay at the start of the soundboard.
    pub fn move_folder(&mut self, name: &str, target: &str, after: bool) {
        crate::order::move_one(&mut self.folders, name, target, after);
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

    /// Moves sounds next to `target`, before or after it, in the order they had. The target may
    /// be one of them: then they go where it was.
    pub fn move_group(&mut self, ids: &[String], target: &str, after: bool) {
        crate::order::move_group(&mut self.sound, |s| s.id.as_str(), ids, target, after);
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
        level: false,
        start: 0.0,
        end: None,
        folder: None,
        favorite: false,
        source: None,
    }
}

/// Deletes the file of a removed sound. Starter sounds keep theirs, for when they come back.
pub fn delete_file(sound: &Sound) {
    if let Some(file) = &sound.file {
        let _ = std::fs::remove_file(dir().join(file));
    }
}

impl Library {}

/// Whether two names are the same sound, whatever the case and the punctuation: "Vine Boom!" and
/// "vine boom" are.
pub fn same_name(a: &str, b: &str) -> bool {
    let plain = |s: &str| {
        let s: String =
            s.chars().flat_map(char::to_lowercase).map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let a = plain(a);
    !a.is_empty() && a == plain(b)
}

/// The sounds of the switches, on and off: Kenney's Interface Sounds (CC0), see
/// `data/feedback/SOURCES.md`. None if they do not decode, which a test rules out.
pub fn cues() -> Option<&'static [Arc<SoundData>; 2]> {
    static CUES: std::sync::OnceLock<Option<[Arc<SoundData>; 2]>> = std::sync::OnceLock::new();
    CUES.get_or_init(|| {
        let cue = |bytes: &[u8]| decode_bytes(bytes.to_vec(), "ogg").ok().map(Arc::new);
        Some([cue(include_bytes!("../../data/feedback/on.ogg"))?, cue(include_bytes!("../../data/feedback/off.ogg"))?])
    })
    .as_ref()
}

/// A safe file name made from the name of a sound, without its extension.
fn file_stem(name: &str) -> String {
    let stem: String = name.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let stem = stem.trim_matches('-');
    if stem.is_empty() { "sound".into() } else { stem.into() }
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
    decode(&dir().join(file)).map(|d| if sound.level { level(d) } else { d }).map(Arc::new)
}

/// Copies an audio file into the soundboard folder and gives back the new file name and a name
/// for the sound. It decodes the file first, so broken files are refused right away. That takes
/// a moment: run it off the main thread, then add the file with [`Library::add_file`].
pub fn copy_in(path: &Path) -> anyhow::Result<(String, String)> {
    decode(path)?;
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    path.file_name().ok_or_else(|| anyhow::anyhow!("no file name"))?;
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let target = free_path(&dir, &stem, &ext);
    std::fs::copy(path, &target)?;
    Ok((target.file_name().unwrap().to_string_lossy().to_string(), stem.replace(['_', '-'], " ")))
}

/// Brings a sound to about -16 LUFS, with peaks at -1 dB at most, so sounds from MyInstants are
/// about as loud as each other: the starter sounds and the ones found there. Loudness here is the
/// mean power of the 100 ms blocks that are not silence, without the K-weighting of LUFS: close
/// enough for short clips.
pub fn level(mut d: SoundData) -> SoundData {
    /// -16 LUFS in this measure: LUFS adds the power of both channels and takes 0.69 dB off.
    const TARGET_DB: f32 = -18.3;
    const PEAK: f32 = 0.89;
    let powers: Vec<f32> = d
        .frames
        .chunks(RATE as usize / 10)
        .map(|c| c.iter().map(|f| (f[0] * f[0] + f[1] * f[1]) / 2.0).sum::<f32>() / c.len() as f32)
        .filter(|p| *p > 1e-7)
        .collect();
    if powers.is_empty() {
        return d;
    }
    // Like LUFS, quiet parts 10 dB under the rest do not count.
    let mean = powers.iter().sum::<f32>() / powers.len() as f32;
    let loud: Vec<f32> = powers.into_iter().filter(|p| *p > mean * 0.1).collect();
    let db = 10.0 * (loud.iter().sum::<f32>() / loud.len() as f32).log10();
    let peak = d.frames.iter().fold(0.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    let gain = 10f32.powf((TARGET_DB - db) / 20.0).min(PEAK / peak.max(1e-6));
    for f in &mut d.frames {
        *f = f.map(|x| x * gain);
    }
    d
}

/// What plays of a sound: its cut, or without one, all but the silent edges.
pub fn playable(sound: &Sound, data: Arc<SoundData>) -> Arc<SoundData> {
    if sound.is_trimmed() {
        return Arc::new(trim(&data, sound.start, sound.end));
    }
    let (start, end) = audible(&data);
    if start == 0.0 && end == data.frames.len() as f32 / RATE as f32 {
        return data;
    }
    Arc::new(trim(&data, start, Some(end)))
}

/// Where a sound starts and ends, in seconds: without the silent edges (under -60 dB), with
/// 10 ms to spare. All of it when it is silent all through.
pub fn audible(data: &SoundData) -> (f32, f32) {
    const SILENCE: f32 = 0.001;
    let len = data.frames.len();
    let loud = |f: &[f32; 2]| f[0].abs().max(f[1].abs()) > SILENCE;
    let (Some(a), Some(b)) = (data.frames.iter().position(loud), data.frames.iter().rposition(loud)) else {
        return (0.0, len as f32 / RATE as f32);
    };
    let spare = RATE as usize / 100;
    let (a, b) = (a.saturating_sub(spare), (b + 1 + spare).min(len));
    (a as f32 / RATE as f32, b as f32 / RATE as f32)
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

    #[test]
    fn the_sounds_of_the_switches_are_short() {
        let [on, off] = cues().expect("they decode");
        for cue in [on, off] {
            assert!((0.2..0.6).contains(&cue.seconds()), "{}", cue.seconds());
            let peak = cue.frames.iter().fold(0f32, |p, [l, r]| p.max(l.abs()).max(r.abs()));
            assert!(peak > 0.5, "audible: {peak}");
        }
    }

    #[test]
    fn names_match_whatever_the_case_and_the_punctuation() {
        assert!(same_name("Vine Boom", "vine boom."));
        assert!(same_name("What the hell (speed up)", "what the hell speed up"));
        assert!(same_name("Ärger!", "ärger"));
        assert!(!same_name("Vine Boom", "Vine Boom 2"));
        assert!(!same_name("", "  ?! "), "nothing is no name");
    }

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
    fn loudspeakers_look_for_a_picture_once() {
        let text = r#"
            starter = 2

            [[sound]]
            id = "sound-1"
            name = "Fart"
            icon = "loudspeaker"

            [[sound]]
            id = "sound-2"
            name = "Helmut"
            icon = "loudspeaker"

            [[sound]]
            id = "sound-3"
            name = "German song"
            icon = "dog"
        "#;
        let mut lib: Library = toml::from_str(text).unwrap();
        assert!(lib.upgrade());
        assert_eq!(lib.find("sound-1").unwrap().icon, "pile-of-poo");
        // No word for it: it keeps the loudspeaker. A picture someone picked stays.
        assert_eq!(lib.find("sound-2").unwrap().icon, "loudspeaker");
        assert_eq!(lib.find("sound-3").unwrap().icon, "dog");
        // Only once: a loudspeaker picked later stays too.
        lib.sound[0].icon = "loudspeaker".into();
        assert!(!lib.upgrade());
        assert_eq!(lib.find("sound-1").unwrap().icon, "loudspeaker");
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
    fn groups_move_together() {
        let order = |lib: &Library| lib.sound.iter().map(|s| s.id.clone()).collect::<Vec<_>>().join(" ");
        let mut lib = Library::starter();
        lib.sound.retain(|s| ["airhorn", "sad-trombone", "rimshot", "drumroll", "applause"].contains(&s.id.as_str()));
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(order(&lib), "airhorn sad-trombone rimshot drumroll applause");
        // To the end, behind the last one, in the order they had.
        lib.move_group(&ids(&["drumroll", "sad-trombone"]), "applause", true);
        assert_eq!(order(&lib), "airhorn rimshot applause sad-trombone drumroll");
        lib.move_group(&ids(&["drumroll", "sad-trombone"]), "airhorn", false);
        assert_eq!(order(&lib), "sad-trombone drumroll airhorn rimshot applause");
        // Dropped on one of themselves: they stay where they are.
        lib.move_group(&ids(&["sad-trombone", "drumroll"]), "drumroll", true);
        assert_eq!(order(&lib), "sad-trombone drumroll airhorn rimshot applause");
    }

    #[test]
    fn folders_keep_their_sounds() {
        let mut lib = Library::starter();
        let name = lib.free_folder_name("Memes");
        lib.folders.push(name.clone());
        assert_eq!(lib.free_folder_name("Memes"), "Memes 2");
        lib.sound[0].folder = Some(name.clone());
        assert!(lib.rename_folder("Memes", "Best"));
        assert_eq!(lib.sound[0].folder.as_deref(), Some("Best"));
        lib.folders.push("Other".into());
        assert!(!lib.rename_folder("Best", "Other"), "two folders with one name");
        assert!(!lib.rename_folder("Best", "  "));
        let saved: Library = toml::from_str(&toml::to_string_pretty(&lib).unwrap()).unwrap();
        assert_eq!(saved.folders, ["Best", "Other"]);
        assert_eq!(saved.sound[0].folder.as_deref(), Some("Best"));
        // Folders move among themselves.
        lib.folders.push("Third".into());
        lib.move_folder("Third", "Best", false);
        assert_eq!(lib.folders, ["Third", "Best", "Other"]);
        lib.move_folder("Third", "Other", true);
        assert_eq!(lib.folders, ["Best", "Other", "Third"]);
        lib.move_folder("Third", "Third", false);
        lib.move_folder("Third", "Gone", false);
        assert_eq!(lib.folders, ["Best", "Other", "Third"]);
        lib.folders.retain(|f| f != "Third");
        // Removing a folder keeps its sounds, in no folder.
        lib.remove_folder("Best");
        assert_eq!(lib.folders, ["Other"]);
        assert_eq!(lib.sound[0].folder, None);
        assert_eq!(lib.sound.len(), Library::starter().sound.len());
    }

    #[test]
    fn silent_edges_do_not_play() {
        // 0.5 s of nothing, 0.2 s of sound with a quiet hiss after it, 0.3 s of nothing.
        let mut frames = vec![[0.0f32, 0.0]; RATE as usize];
        for f in &mut frames[RATE as usize / 2..RATE as usize * 7 / 10] {
            *f = [0.4, -0.4];
        }
        frames[RATE as usize * 7 / 10] = [0.0005, 0.0];
        let data = Arc::new(SoundData { frames });
        let (a, b) = audible(&data);
        assert!((a - 0.49).abs() < 0.001 && (b - 0.71).abs() < 0.001, "{a} {b}");
        let mut sound = Library::starter().sound.remove(0);
        assert!((playable(&sound, data.clone()).seconds() - 0.22).abs() < 0.001);
        // A cut of your own plays as it is, silence and all.
        sound.start = 0.1;
        assert!((playable(&sound, data.clone()).seconds() - 0.9).abs() < 0.001);
        // Silent all through: nothing to go by, so all of it.
        let quiet = Arc::new(SoundData { frames: vec![[0.0, 0.0]; 4800] });
        assert_eq!(audible(&quiet), (0.0, 0.1));
    }

    #[test]
    fn levels_loud_and_quiet_sounds() {
        let tone = |amp: f32| SoundData { frames: (0..RATE).map(|i| [amp * (i as f32 * 0.05).sin(); 2]).collect() };
        let peak = |d: &SoundData| d.frames.iter().fold(0.0f32, |m, f| m.max(f[0].abs()));
        // A loud tone comes down and a quiet one up, to the same level.
        let (loud, quiet) = (peak(&level(tone(1.0))), peak(&level(tone(0.01))));
        assert!(quiet > 0.1 && quiet < 0.3, "{quiet}");
        assert!((loud - quiet).abs() < 0.01, "{loud} {quiet}");
        // A short, peaky click stops at -1 dB.
        let mut click = tone(0.0);
        click.frames[100] = [1.0, 1.0];
        click.frames[101] = [-1.0, -1.0];
        assert!(peak(&level(click)) <= 0.9);
        // Silence stays silence.
        assert_eq!(peak(&level(tone(0.0))), 0.0);
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
