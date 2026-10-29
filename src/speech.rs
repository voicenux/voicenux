//! Speech to text, for apps that find a sound by what you say, like the plugin for OpenDeck.
//! Whisper (base, by OpenAI, MIT) runs in plain Rust with candle, on the processor, in about half
//! a second. The model is not in the app: Voicenux downloads it the first time an app listens,
//! about 290 MB, pinned to a commit on Hugging Face.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, bail};
use candle_core::{D, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self as m, Config, audio};

/// Whisper hears at 16 kHz.
pub const RATE: u32 = 16000;
const REPO: &str = "https://huggingface.co/openai/whisper-base/resolve/e37978b90ca9030d5170a5c07aadb050351a65bb";
/// The files of the model, with their sizes.
const FILES: &[(&str, u64)] =
    &[("config.json", 1983), ("tokenizer.json", 2_480_466), ("model.safetensors", 290_403_936)];
/// Sound names are short. More is Whisper going round in circles.
const MAX_TOKENS: usize = 24;

pub fn dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("voicenux").join("speech").join("whisper-base")
}

fn complete(path: &Path, size: u64) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == size)
}

/// Whether the model is downloaded.
pub fn ready() -> bool {
    FILES.iter().all(|(file, size)| complete(&dir().join(file), *size))
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
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(concat!("Voicenux/", env!("CARGO_PKG_VERSION"), " (+https://github.com/voicenux/voicenux)"))
        .build()
        .into();
    for (file, size) in FILES {
        let path = dir().join(file);
        if complete(&path, *size) {
            continue;
        }
        let url = format!("{REPO}/{file}");
        log::info!("downloading {url}");
        let mut resp = agent.get(&url).call().with_context(|| format!("download {url}"))?;
        std::fs::create_dir_all(dir())?;
        let part = dir().join(format!("{file}.part"));
        let mut out = std::fs::File::create(&part)?;
        std::io::copy(&mut resp.body_mut().with_config().limit(*size + 1).reader(), &mut out)?;
        if !complete(&part, *size) {
            let _ = std::fs::remove_file(&part);
            bail!("{url} came in broken");
        }
        out.sync_all()?;
        std::fs::rename(&part, &path)?;
    }
    Ok(())
}

/// What `samples` (16 kHz, mono) say, or "" for nothing. `language` like "de" for German, or
/// None for English or the language of the app, whichever it sounds like. Downloads the model
/// first when needed. Blocks: call it off the main thread.
pub fn transcribe(samples: &[f32], language: Option<&str>) -> anyhow::Result<String> {
    // Whisper makes up words for silence.
    if samples.iter().fold(0f32, |p, x| p.max(x.abs())) < 0.01 {
        return Ok(String::new());
    }
    download()?;
    let started = std::time::Instant::now();
    let dir = dir();
    let device = Device::Cpu;
    let config: Config = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
    // SAFETY: the file is ours, complete, and nothing writes to it while it is mapped.
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], m::DTYPE, &device)? };
    let mut model = m::model::Whisper::load(&vb, config.clone())?;
    let tokens = Tokens::load(&dir.join("tokenizer.json"))?;

    // Ten seconds of mel are enough for a name, and three times as fast as the thirty Whisper
    // was made for. Longer clips get all thirty.
    let samples = &samples[..samples.len().min(m::N_SAMPLES)];
    let mel = audio::pcm_to_mel(&config, samples, &mel_filters(config.num_mel_bins));
    let frames = mel.len() / config.num_mel_bins;
    let mel = Tensor::from_vec(mel, (1, config.num_mel_bins, frames), &device)?;
    let used = if samples.len() / m::HOP_LENGTH < 900 { 1000 } else { m::N_FRAMES };
    let features = model.encoder.forward(&mel.narrow(2, 0, used.min(frames))?, true)?;

    let sot = tokens.id(m::SOT_TOKEN)?;
    let language = match language {
        Some(code) => tokens.id(&format!("<|{code}|>")).with_context(|| format!("no language \"{code}\""))?,
        None => {
            let app = crate::i18n::language();
            let mut codes = vec!["en".to_string()];
            if app != "en" {
                codes.push(app);
            }
            let ids: Vec<u32> = codes.iter().filter_map(|c| tokens.id(&format!("<|{c}|>")).ok()).collect();
            let first = model.decoder.forward(&Tensor::new(&[[sot]], &device)?, &features, true)?;
            let logits: Vec<f32> = model.decoder.final_linear(&first.i(..1)?)?.i(0)?.i(0)?.to_vec1()?;
            ids.into_iter().max_by(|a, b| logits[*a as usize].total_cmp(&logits[*b as usize])).context("no language")?
        }
    };
    let eot = tokens.id(m::EOT_TOKEN)?;
    let no_timestamps = tokens.id(m::NO_TIMESTAMPS_TOKEN)?;
    let no_speech = m::NO_SPEECH_TOKENS.iter().find_map(|t| tokens.id(t).ok());
    // Only words: no timestamps, no special tokens.
    let mut mask = vec![0f32; config.vocab_size];
    for id in config.suppress_tokens.iter().map(|&t| t as usize).chain(eot as usize + 1..config.vocab_size) {
        if let Some(m) = mask.get_mut(id) {
            *m = f32::NEG_INFINITY;
        }
    }
    let mask = Tensor::new(mask.as_slice(), &device)?;
    let mut ids = vec![sot, language, tokens.id(m::TRANSCRIBE_TOKEN)?, no_timestamps];
    let start = ids.len();
    for i in 0..MAX_TOKENS {
        let input = Tensor::new(ids.as_slice(), &device)?.unsqueeze(0)?;
        let ys = model.decoder.forward(&input, &features, i == 0)?;
        // How sure Whisper is that nobody speaks: its guess right after the start token.
        if i == 0
            && let Some(no_speech) = no_speech
        {
            let logits = model.decoder.final_linear(&ys.i((..1, ..1))?)?.i(0)?.i(0)?;
            let p: f32 = candle_nn::ops::softmax(&logits, D::Minus1)?.i(no_speech as usize)?.to_scalar()?;
            if p as f64 > m::NO_SPEECH_THRESHOLD {
                return Ok(String::new());
            }
        }
        let (_, len, _) = ys.dims3()?;
        let logits = model.decoder.final_linear(&ys.i((..1, len - 1..))?)?.i(0)?.i(0)?;
        let next = logits.broadcast_add(&mask)?.argmax(D::Minus1)?.to_scalar::<u32>()?;
        // The same word over and over: Whisper is stuck.
        if next == eot || ids.len() >= start + 3 && ids[ids.len() - 3..].iter().all(|&t| t == next) {
            break;
        }
        ids.push(next);
    }
    let text = tokens.text(&ids[start..]);
    log::info!("heard \"{text}\" in {:?}", started.elapsed());
    Ok(text)
}

/// The words of the model. Only for reading its answers: no need to write in its tokens.
struct Tokens {
    names: HashMap<u32, String>,
    ids: HashMap<String, u32>,
    bytes: HashMap<char, u8>,
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
        let names = ids.iter().map(|(name, id)| (*id, name.clone())).collect();
        Ok(Self { names, ids, bytes: byte_decoder() })
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
        };
        assert_eq!(t.text(&[9, 1, 2]), "vine boom");
        assert_eq!(t.text(&[3, 4]), "schön");
    }

    #[test]
    fn silence_says_nothing() {
        assert_eq!(transcribe(&vec![0.0; 16000], None).unwrap(), "");
    }

    #[test]
    fn resamples_to_16_khz() {
        assert_eq!(resample(&vec![0.5; 48000], 48000).len(), 16000);
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
        let text = transcribe(&resample(&mono, crate::soundboard::RATE), None).unwrap();
        assert!(text.to_lowercase().contains("emotional damage"), "{text}");
        assert!(transcribe(&resample(&mono, crate::soundboard::RATE), Some("de")).is_ok());
        assert!(transcribe(&resample(&mono, crate::soundboard::RATE), Some("xx")).is_err());
    }
}
