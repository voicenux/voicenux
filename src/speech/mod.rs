//! Speech to text, to find a sound by what you say: in Find sounds, and for apps like the plugin
//! for OpenDeck. Whisper (small, by OpenAI, MIT) runs in plain Rust with candle, on the processor,
//! in about a second. The model is not in the app: Voicenux downloads it the first time it is
//! needed, or from Settings, about 970 MB, pinned to a commit on Hugging Face.
//!
//! Whisper gets the names it should expect, like the sounds on your soundboard and the ones most
//! people play. Without them, a German "vine boom" comes out as "Weinbumm" or "Bein Boom".

mod whisper;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use candle_core::{D, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self as m, Config, audio};

use whisper::Whisper;

/// Whisper hears at 16 kHz.
pub const RATE: u32 = 16000;
const REPO: &str = "https://huggingface.co/openai/whisper-small/resolve/973afd24965f72e36ca33b3055d56a652f456b4d";
/// The files of the model, with their sizes.
const FILES: &[(&str, u64)] =
    &[("config.json", 1967), ("tokenizer.json", 2_480_466), ("model.safetensors", 966_995_080)];
/// Sound names are short. More is Whisper going round in circles.
const MAX_TOKENS: usize = 24;
/// How much of the names to expect Whisper reads. It takes 223 tokens at most.
const PROMPT_TOKENS: usize = 200;

pub fn dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("voicenux").join("speech").join("whisper-small")
}

/// Where Voicenux 0.3.0 kept the smaller model it used. It goes once the new one is there.
fn old_dir() -> PathBuf {
    dir().with_file_name("whisper-base")
}

fn complete(path: &Path, size: u64) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == size)
}

/// Whether the model is downloaded.
pub fn ready() -> bool {
    FILES.iter().all(|(file, size)| complete(&dir().join(file), *size))
}

/// What the model takes on disk.
pub fn size() -> u64 {
    FILES.iter().map(|(_, size)| size).sum()
}

static DOWNLOADING: AtomicBool = AtomicBool::new(false);
static DONE: AtomicU64 = AtomicU64::new(0);

/// How far the download is, from 0 to 1, while it runs. Settings shows it.
pub fn progress() -> Option<f64> {
    DOWNLOADING.load(Ordering::Relaxed).then(|| DONE.load(Ordering::Relaxed) as f64 / size() as f64)
}

/// Deletes the model. It downloads again when it is needed.
pub fn remove() -> std::io::Result<()> {
    if DOWNLOADING.load(Ordering::Relaxed) {
        return Ok(());
    }
    let _ = std::fs::remove_dir_all(old_dir());
    match std::fs::remove_dir_all(dir()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Starts the download in the background, so the model is there once it is needed.
pub fn prepare() {
    if !ready() {
        std::thread::spawn(|| {
            if let Err(e) = download() {
                log::warn!("speech model: {e:#}");
            }
        });
    }
}

/// Downloads the model, unless it is there. Blocks: call it off the main thread.
pub fn download() -> anyhow::Result<()> {
    // One download at a time. A second caller waits for the first one.
    static ONE: Mutex<()> = Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    if !ready() {
        DOWNLOADING.store(true, Ordering::Relaxed);
        let result = fetch();
        DOWNLOADING.store(false, Ordering::Relaxed);
        result?;
    }
    let _ = std::fs::remove_dir_all(old_dir());
    Ok(())
}

fn fetch() -> anyhow::Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(concat!("Voicenux/", env!("CARGO_PKG_VERSION"), " (+https://github.com/voicenux/voicenux)"))
        .build()
        .into();
    DONE.store(0, Ordering::Relaxed);
    for (file, size) in FILES {
        let path = dir().join(file);
        if complete(&path, *size) {
            DONE.fetch_add(*size, Ordering::Relaxed);
            continue;
        }
        let url = format!("{REPO}/{file}");
        log::info!("downloading {url}");
        let mut resp = agent.get(&url).call().with_context(|| format!("download {url}"))?;
        std::fs::create_dir_all(dir())?;
        let part = dir().join(format!("{file}.part"));
        let mut out = std::fs::File::create(&part)?;
        let mut body = resp.body_mut().with_config().limit(*size + 1).reader();
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = body.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            DONE.fetch_add(n as u64, Ordering::Relaxed);
        }
        if !complete(&part, *size) {
            let _ = std::fs::remove_file(&part);
            bail!("{url} came in broken");
        }
        out.sync_all()?;
        std::fs::rename(&part, &path)?;
    }
    Ok(())
}

/// What `samples` (16 kHz, mono) say, none for nothing. `language` like "de" for German, or None
/// for English and the language of the app: then it reads them in both, the one it sounds like
/// first. A German "vine boom" is "Weinbumm" in German, so the English reading is worth a try.
/// `expect` are names it may hear, the likeliest first. Downloads the model first when needed.
/// Blocks: call it off the main thread.
pub fn transcribe(samples: &[f32], language: Option<&str>, expect: &[String]) -> anyhow::Result<Vec<String>> {
    // Whisper makes up words for silence.
    if samples.iter().fold(0f32, |p, x| p.max(x.abs())) < 0.01 {
        return Ok(Vec::new());
    }
    download()?;
    let started = std::time::Instant::now();
    let dir = dir();
    let device = Device::Cpu;
    let config: Config = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
    // SAFETY: the file is ours, complete, and nothing writes to it while it is mapped.
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], m::DTYPE, &device)? };
    let mut model = Whisper::load(&vb, &config)?;
    let tokens = Tokens::load(&dir.join("tokenizer.json"))?;

    // Ten seconds of mel are enough for a name, and three times as fast as the thirty Whisper
    // was made for. Longer clips get all thirty.
    let samples = &samples[..samples.len().min(m::N_SAMPLES)];
    let mel = audio::pcm_to_mel(&config, samples, &mel_filters(config.num_mel_bins));
    let frames = mel.len() / config.num_mel_bins;
    let mel = Tensor::from_vec(mel, (1, config.num_mel_bins, frames), &device)?;
    let used = if samples.len() / m::HOP_LENGTH < 900 { 1000 } else { m::N_FRAMES };
    model.hear(&mel.narrow(2, 0, used.min(frames))?)?;

    // Whisper's guess right after the start: whether anyone speaks, and in which language.
    let sot = tokens.id(m::SOT_TOKEN)?;
    let (start, first) = model.read(None, &[sot])?;
    if let Some(no_speech) = m::NO_SPEECH_TOKENS.iter().find_map(|t| tokens.id(t).ok()) {
        let p: f32 = candle_nn::ops::softmax(&first, D::Minus1)?.i(no_speech as usize)?.to_scalar()?;
        if p as f64 > m::NO_SPEECH_THRESHOLD {
            return Ok(Vec::new());
        }
    }
    let languages = match language {
        Some(code) => vec![tokens.id(&format!("<|{code}|>")).with_context(|| format!("no language \"{code}\""))?],
        None => {
            let app = crate::i18n::language();
            let mut codes = vec!["en".to_string()];
            if app != "en" {
                codes.push(app);
            }
            let mut ids: Vec<u32> = codes.iter().filter_map(|c| tokens.id(&format!("<|{c}|>")).ok()).collect();
            let logits: Vec<f32> = first.to_vec1()?;
            ids.sort_by(|a, b| logits[*b as usize].total_cmp(&logits[*a as usize]));
            ids
        }
    };
    // The names to expect, as if Whisper heard them just before. Read once for all languages.
    let prompt = prompt(&tokens, expect);
    let before = match prompt.is_empty() {
        true => start,
        false => model.read(None, &[&[tokens.id("<|startofprev|>")?][..], &prompt, &[sot]].concat())?.0,
    };
    let eot = tokens.id(m::EOT_TOKEN)?;
    // Only words: no timestamps, no special tokens.
    let mut mask = vec![0f32; config.vocab_size];
    for id in config.suppress_tokens.iter().map(|&t| t as usize).chain(eot as usize + 1..config.vocab_size) {
        if let Some(m) = mask.get_mut(id) {
            *m = f32::NEG_INFINITY;
        }
    }
    let mask = Tensor::new(mask.as_slice(), &device)?;
    let mut readings: Vec<String> = Vec::new();
    for language in languages {
        let task = [language, tokens.id(m::TRANSCRIBE_TOKEN)?, tokens.id(m::NO_TIMESTAMPS_TOKEN)?];
        let (mut read, mut logits) = model.read(Some(&before), &task)?;
        let mut said = Vec::new();
        for _ in 0..MAX_TOKENS {
            let next = whisper::best(&logits, &mask)?;
            // The same word over and over: Whisper is stuck.
            if next == eot || said.len() >= 3 && said[said.len() - 3..].iter().all(|&t| t == next) {
                break;
            }
            said.push(next);
            (read, logits) = model.read(Some(&read), &[next])?;
        }
        // The names it expected end in a comma, and so does what it heard at times.
        let text = once(&tokens.text(&said)).trim_end_matches([',', '.', ';', ':']).trim().to_string();
        if !text.is_empty() && !readings.iter().any(|r| crate::names::same_name(r, &text)) {
            readings.push(text);
        }
    }
    log::info!("heard {readings:?} in {:?}", started.elapsed());
    Ok(readings)
}

/// The names to expect as tokens, as many as fit.
fn prompt(tokens: &Tokens, expect: &[String]) -> Vec<u32> {
    let mut ids = Vec::new();
    for name in expect {
        let more = tokens.encode(&format!(" {},", name.trim()));
        if ids.len() + more.len() > PROMPT_TOKENS {
            break;
        }
        ids.extend(more);
    }
    ids
}

/// "Bra. Bra. Bra." once: Whisper says a short name again and again at times.
fn once(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let plain = |w: &str| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    for n in 1..=words.len() / 2 {
        if words.chunks(n).all(|c| c.iter().map(|w| plain(w)).eq(words[..c.len()].iter().map(|w| plain(w)))) {
            return words[..n].join(" ");
        }
    }
    text.to_string()
}

/// The words of the model: to read its answers, and to write the names it should expect.
struct Tokens {
    names: HashMap<u32, String>,
    ids: HashMap<String, u32>,
    bytes: HashMap<char, u8>,
    /// Which two parts make a word part, and how early: the lower, the earlier.
    merges: HashMap<(String, String), usize>,
}

impl Tokens {
    fn load(path: &Path) -> anyhow::Result<Self> {
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        let mut ids = HashMap::new();
        for (name, id) in json["model"]["vocab"].as_object().context("no vocab")? {
            ids.insert(name.clone(), id.as_u64().context("bad vocab")? as u32);
        }
        for t in json["added_tokens"].as_array().context("no added tokens")? {
            let (Some(name), Some(id)) = (t["content"].as_str(), t["id"].as_u64()) else { continue };
            ids.insert(name.to_string(), id as u32);
        }
        let merges = json["model"]["merges"]
            .as_array()
            .context("no merges")?
            .iter()
            .filter_map(|m| m.as_str()?.split_once(' ').map(|(a, b)| (a.to_string(), b.to_string())))
            .enumerate()
            .map(|(rank, pair)| (pair, rank))
            .collect();
        let names = ids.iter().map(|(name, id)| (*id, name.clone())).collect();
        Ok(Self { names, ids, bytes: byte_decoder(), merges })
    }

    fn id(&self, name: &str) -> anyhow::Result<u32> {
        self.ids.get(name).copied().with_context(|| format!("no token {name}"))
    }

    fn text(&self, ids: &[u32]) -> String {
        let mut bytes = Vec::new();
        for name in ids.iter().filter_map(|id| self.names.get(id)).filter(|n| !n.starts_with("<|")) {
            bytes.extend(name.chars().filter_map(|c| self.bytes.get(&c)));
        }
        String::from_utf8_lossy(&bytes).trim().to_string()
    }

    /// The tokens of `text`, like GPT-2 makes them: cut into words, each word as its bytes, then
    /// the parts merged in the order the model learned them.
    fn encode(&self, text: &str) -> Vec<u32> {
        let letters: HashMap<u8, char> = self.bytes.iter().map(|(c, b)| (*b, *c)).collect();
        let mut out = Vec::new();
        for word in words(text) {
            let mut parts: Vec<String> = word.bytes().map(|b| letters[&b].to_string()).collect();
            while let Some((_, i)) = parts
                .windows(2)
                .enumerate()
                .filter_map(|(i, w)| Some((*self.merges.get(&(w[0].clone(), w[1].clone()))?, i)))
                .min()
            {
                let merged = format!("{}{}", parts[i], parts[i + 1]);
                parts.splice(i..i + 2, [merged]);
            }
            out.extend(parts.iter().filter_map(|p| self.ids.get(p)));
        }
        out
    }
}

/// Text cut like GPT-2 does before it makes tokens: "'s" and the like, and runs of letters, of
/// digits or of other signs, each with the space before it.
fn words(text: &str) -> Vec<String> {
    let kind = |c: char| match c {
        c if c.is_alphabetic() => 0,
        c if c.is_numeric() => 1,
        c if c.is_whitespace() => 2,
        _ => 3,
    };
    let mut out: Vec<String> = Vec::new();
    let mut last = None;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some(short) = ["'s", "'t", "'re", "'ve", "'m", "'ll", "'d"].iter().find(|s| rest.starts_with(**s)) {
            out.push(short.to_string());
            rest = &rest[short.len()..];
            last = None;
            continue;
        }
        let k = kind(c);
        match out.last_mut() {
            Some(word) if last == Some(k) && k != 2 => word.push(c),
            // A space goes with the word after it.
            Some(word) if word == " " && k != 2 => word.push(c),
            _ => out.push(c.to_string()),
        }
        last = Some(k);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Tokens write bytes as letters, like GPT-2: printable ones as themselves, the others as letters
/// from 256 on. This turns them back.
fn byte_decoder() -> HashMap<char, u8> {
    let printable = |b: u32| matches!(b, 0x21..=0x7E | 0xA1..=0xAC | 0xAE..=0xFF);
    let mut next = 256;
    (0..256u32)
        .filter_map(|b| {
            let c = if printable(b) {
                b
            } else {
                next += 1;
                next - 1
            };
            Some((char::from_u32(c)?, b as u8))
        })
        .collect()
}

/// The mel filters of Whisper, like librosa makes them (Slaney): `n` rows of 201 weights.
fn mel_filters(n: usize) -> Vec<f32> {
    let bins = m::N_FFT / 2 + 1;
    // Linear up to 1 kHz, then logarithmic.
    let (step, knee) = (200.0 / 3.0, 1000.0);
    let log_step = 6.4f64.ln() / 27.0;
    let to_mel = |f: f64| if f >= knee { knee / step + (f / knee).ln() / log_step } else { f / step };
    let to_hz = |mel: f64| if mel >= knee / step { knee * (log_step * (mel - knee / step)).exp() } else { step * mel };
    let top = to_mel(RATE as f64 / 2.0);
    let edges: Vec<f64> = (0..n + 2).map(|i| to_hz(top * i as f64 / (n + 1) as f64)).collect();
    let mut weights = vec![0f32; n * bins];
    for i in 0..n {
        let norm = 2.0 / (edges[i + 2] - edges[i]);
        for j in 0..bins {
            let f = j as f64 * RATE as f64 / m::N_FFT as f64;
            let up = (f - edges[i]) / (edges[i + 1] - edges[i]);
            let down = (edges[i + 2] - f) / (edges[i + 2] - edges[i + 1]);
            weights[i * bins + j] = (up.min(down).max(0.0) * norm) as f32;
        }
    }
    weights
}

/// The microphone at 16 kHz, for Whisper.
pub fn resample(samples: &[f32], rate: u32) -> Vec<f32> {
    let frames: Vec<[f32; 2]> = samples.iter().map(|&x| [x, x]).collect();
    crate::soundboard::resample(&frames, rate, RATE).into_iter().map(|[l, _]| l).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mel_filters_are_the_ones_of_whisper() {
        // From the filters Whisper ships, for 80 rows.
        let w = mel_filters(80);
        let row = |r: usize| &w[r * 201..(r + 1) * 201];
        for (r, peak, value, nonzero) in [(0, 1, 0.024862595, 1), (40, 43, 0.014735566, 3), (79, 192, 0.0031647119, 14)]
        {
            let max = row(r).iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
            assert_eq!(max.0, peak);
            assert!((max.1 - value).abs() < 1e-7, "row {r}: {}", max.1);
            assert_eq!(row(r).iter().filter(|x| **x > 0.0).count(), nonzero);
        }
    }

    #[test]
    fn tokens_turn_back_into_text() {
        let bytes = byte_decoder();
        assert_eq!(bytes.len(), 256);
        assert_eq!(bytes[&'Ġ'], b' ');
        assert_eq!(bytes[&'a'], b'a');
        let t = Tokens {
            names: [
                (1, "Ġvine".into()),
                (2, "Ġboom".into()),
                (3, "Ġsch".into()),
                (4, "Ã¶n".into()),
                (9, "<|en|>".into()),
            ]
            .into(),
            ids: HashMap::new(),
            bytes,
            merges: HashMap::new(),
        };
        assert_eq!(t.text(&[9, 1, 2]), "vine boom");
        assert_eq!(t.text(&[3, 4]), "schön");
    }

    #[test]
    fn text_is_cut_into_words_like_gpt_2_does() {
        assert_eq!(words(" It's over,"), [" It", "'s", " over", ","]);
        assert_eq!(words(" Windows XP 2,"), [" Windows", " XP", " 2", ","]);
        assert_eq!(words(" Stück"), [" Stück"]);
    }

    #[test]
    fn a_name_said_again_and_again_counts_once() {
        assert_eq!(once("Bra. Bra. Bra. Bra."), "Bra.");
        assert_eq!(once("Fine boom. Fine boom."), "Fine boom.");
        assert_eq!(once("Vine boom"), "Vine boom");
        assert_eq!(once("Bra Bra Boom"), "Bra Bra Boom");
    }

    #[test]
    fn silence_says_nothing() {
        assert!(transcribe(&vec![0.0; 16000], None, &[]).unwrap().is_empty());
    }

    #[test]
    fn resamples_to_16_khz() {
        assert_eq!(resample(&vec![0.5; 48000], 48000).len(), 16000);
    }

    /// Needs the model, so only on request: `cargo test -- --ignored`. The tokens are the ones
    /// Hugging Face's tokenizers make.
    #[test]
    #[ignore]
    fn names_turn_into_the_tokens_of_whisper() {
        download().unwrap();
        let t = Tokens::load(&dir().join("tokenizer.json")).unwrap();
        assert_eq!(t.encode(" Vine Boom,"), [40569, 15523, 11]);
        assert_eq!(t.encode(" It's over,"), [467, 311, 670, 11]);
        assert_eq!(t.encode(" Ich muss Scheißen,"), [3141, 6425, 25321, 6230, 268, 11]);
        assert_eq!(t.encode(" ROBLOX oof,"), [9025, 17624, 46, 55, 277, 2670, 11]);
        assert_eq!(t.text(&t.encode(" Звук фотоаппарата,")), "Звук фотоаппарата,");
    }

    /// Downloads the model and a sound with words from MyInstants, so only on request:
    /// `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn hears_what_a_sound_says() {
        let _ = env_logger::builder().is_test(true).try_init();
        let found = crate::soundboard::online::search("emotional damage", 1).unwrap();
        let bytes = crate::soundboard::online::download(&found[0]).unwrap();
        let data = crate::soundboard::decode_bytes(bytes, &found[0].ext).unwrap();
        let mono: Vec<f32> = data.frames.iter().map(|[l, r]| (l + r) / 2.0).collect();
        let heard = transcribe(&resample(&mono, crate::soundboard::RATE), None, &[]).unwrap();
        assert!(heard[0].to_lowercase().contains("emotional damage"), "{heard:?}");
        assert!(transcribe(&resample(&mono, crate::soundboard::RATE), Some("de"), &[]).is_ok());
        assert!(transcribe(&resample(&mono, crate::soundboard::RATE), Some("xx"), &[]).is_err());
    }
}
