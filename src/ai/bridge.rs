//! Runs the AI helper and moves the voice between the engine and it. The effect in the chain
//! (`dsp::ai`) only touches lock-free rings and atomics, so the realtime thread never waits. A
//! thread of its own here does all the rest: it starts the helper when an AI voice plays, loads
//! its model, sends the voice in blocks and writes the answers back.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{MODELS, install, on_cpu};

/// What an AI effect and the bridge share.
pub struct Link {
    /// Index into [`MODELS`].
    pub model: AtomicUsize,
    /// Pitch shift in semitones, as the bits of an f32.
    pub key: AtomicU32,
    /// False once the effect is gone.
    pub alive: AtomicBool,
    /// Samples per block the helper sends back.
    pub block: AtomicUsize,
    /// Samples the effect keeps in store against a late block, so it does not click.
    pub cushion: AtomicUsize,
    /// Sample rate of the engine.
    pub rate: u32,
}

impl Link {
    pub fn new(model: usize, key: f32, rate: u32) -> Self {
        Self {
            rate,
            model: AtomicUsize::new(model),
            key: AtomicU32::new(key.to_bits()),
            alive: AtomicBool::new(true),
            block: AtomicUsize::new(0),
            cushion: AtomicUsize::new(usize::MAX),
        }
    }
}

/// The bridge's ends of an effect's rings.
pub struct Stream {
    pub link: Arc<Link>,
    pub input: rtrb::Consumer<f32>,
    pub output: rtrb::Producer<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// No AI voice plays.
    Off,
    Starting,
    Loading(&'static str),
    /// With about how much delay, in ms.
    Running(u32),
    /// The voice is not downloaded, or the engine not installed.
    Missing,
    Failed(String),
}

static STATUS: Mutex<Status> = Mutex::new(Status::Off);

pub fn status() -> Status {
    STATUS.lock().map(|s| s.clone()).unwrap_or(Status::Off)
}

fn set_status(s: Status) {
    if let Ok(mut now) = STATUS.lock()
        && *now != s
    {
        log::info!("AI: {s:?}");
        *now = s;
    }
}

enum Event {
    Attach(Stream),
    Frame(u8, Vec<u8>),
    Exited,
    Stop,
}

static EVENTS: OnceLock<Sender<Event>> = OnceLock::new();

fn events() -> &'static Sender<Event> {
    EVENTS.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        let back = tx.clone();
        let _ = std::thread::Builder::new().name("voicenux-ai".into()).spawn(move || run(rx, back));
        tx
    })
}

/// Hands a new stream to the bridge. Called from `Effect::new`, never on the realtime thread.
pub fn attach(stream: Stream) {
    let _ = events().send(Event::Attach(stream));
}

/// Stops the helper, for removing the engine.
pub fn stop() {
    if let Some(tx) = EVENTS.get() {
        let _ = tx.send(Event::Stop);
    }
}

struct Helper {
    child: Child,
    stdin: ChildStdin,
    ready: bool,
    model: Option<usize>,
    /// The rate of the stream the model was loaded for.
    rate: u32,
    /// Samples per block and of cushion at that rate, from the helper once the model is loaded.
    block: usize,
    cushion: usize,
    loading: bool,
    /// Started on the processor.
    cpu: bool,
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn send(stdin: &mut ChildStdin, kind: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut head = [kind, 0, 0, 0, 0];
    head[1..].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    stdin.write_all(&head)?;
    stdin.write_all(payload)?;
    stdin.flush()
}

fn start(back: Sender<Event>) -> anyhow::Result<Helper> {
    let script = install::write_helper()?;
    let mut child = Command::new(install::python())
        .arg(script)
        .arg("serve")
        .arg("--device")
        .arg(if on_cpu() { "cpu" } else { "cuda" })
        .arg("--models")
        .arg(install::models_dir())
        .env("HF_HOME", install::hf_home())
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.take().expect("piped");
    let mut stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            log::info!("{line}");
        }
    });
    std::thread::spawn(move || {
        let mut head = [0u8; 5];
        while stdout.read_exact(&mut head).is_ok() {
            let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
            let mut payload = vec![0u8; len];
            if stdout.read_exact(&mut payload).is_err() {
                break;
            }
            if back.send(Event::Frame(head[0], payload)).is_err() {
                return;
            }
        }
        let _ = back.send(Event::Exited);
    });
    Ok(Helper { child, stdin, ready: false, model: None, rate: 0, block: 0, cushion: 0, loading: false, cpu: on_cpu() })
}

/// Throws away what the effect sent: while the helper is not ready, old voice would only pile up.
fn drain(input: &mut rtrb::Consumer<f32>) {
    let n = input.slots();
    if let Ok(chunk) = input.read_chunk(n) {
        chunk.commit_all();
    }
}

fn run(rx: Receiver<Event>, back: Sender<Event>) {
    let mut stream: Option<Stream> = None;
    let mut helper: Option<Helper> = None;
    let mut idle = Instant::now();
    // After a failed start, wait before the next try instead of starting again and again.
    let mut retry_at = Instant::now();
    let mut frame: Vec<u8> = Vec::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(4)) {
            Ok(Event::Attach(s)) => stream = Some(s),
            Ok(Event::Stop) => {
                helper = None;
                stream = None;
                set_status(Status::Off);
            }
            Ok(Event::Exited) => {
                if helper.take().is_some() && stream.is_some() {
                    set_status(Status::Failed("The AI helper stopped".into()));
                    retry_at = Instant::now() + Duration::from_secs(10);
                }
            }
            Ok(Event::Frame(kind, payload)) => {
                let Some(h) = helper.as_mut() else { continue };
                match kind {
                    b'R' => {
                        let info: serde_json::Value = serde_json::from_slice(&payload).unwrap_or_default();
                        h.ready = true;
                        log::info!("AI helper ready on {}", info["device"]);
                    }
                    b'M' => {
                        let info: serde_json::Value = serde_json::from_slice(&payload).unwrap_or_default();
                        let samples = |key: &str| (info[key].as_f64().unwrap_or(0.25) * h.rate as f64).round() as usize;
                        h.block = samples("block").max(1);
                        h.cushion = samples("cushion");
                        h.loading = false;
                        let delay = (info["delay"].as_f64().unwrap_or(0.0) * 1000.0).round() as u32;
                        log::info!(
                            "AI blocks of {} ms, about {delay} ms delay",
                            info["block"].as_f64().unwrap_or(0.0) * 1000.0
                        );
                        set_status(Status::Running(delay));
                    }
                    b'A' => {
                        if let Some(s) = stream.as_mut() {
                            let samples = payload.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                            let n = s.output.slots().min(payload.len() / 4);
                            if let Ok(chunk) = s.output.write_chunk_uninit(n) {
                                chunk.fill_from_iter(samples);
                            }
                        }
                    }
                    b'E' => {
                        let text = String::from_utf8_lossy(&payload).into_owned();
                        log::warn!("AI helper: {text}");
                        h.loading = false;
                        set_status(Status::Failed(text));
                    }
                    _ => {}
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        if stream.as_ref().is_some_and(|s| !s.link.alive.load(Ordering::Acquire)) {
            stream = None;
            idle = Instant::now();
        }
        let Some(s) = stream.as_mut() else {
            // The helper holds a lot of memory on the graphics card: let it go after a while.
            if helper.is_some() && idle.elapsed() > Duration::from_secs(30) {
                helper = None;
            }
            if helper.is_none() {
                set_status(Status::Off);
            }
            continue;
        };
        let index = s.link.model.load(Ordering::Relaxed).min(MODELS.len() - 1);
        let model = &MODELS[index];
        if !super::ready(model) {
            set_status(Status::Missing);
            drain(&mut s.input);
            continue;
        }
        // The setting moved the AI to the processor or back: start again on the other one.
        if helper.as_ref().is_some_and(|h| h.cpu != on_cpu()) {
            helper = None;
        }
        if helper.is_none() {
            if Instant::now() < retry_at {
                drain(&mut s.input);
                continue;
            }
            set_status(Status::Starting);
            match start(back.clone()) {
                Ok(h) => helper = Some(h),
                Err(e) => {
                    log::warn!("AI helper: {e}");
                    set_status(Status::Failed(e.to_string()));
                    retry_at = Instant::now() + Duration::from_secs(10);
                    continue;
                }
            }
        }
        let h = helper.as_mut().expect("started");
        if !h.ready {
            drain(&mut s.input);
            continue;
        }
        // A new model, or a stream at another rate, loads again.
        if (h.model != Some(index) || h.rate != s.link.rate) && !h.loading {
            set_status(Status::Loading(model.name));
            h.loading = true;
            h.model = Some(index);
            h.rate = s.link.rate;
            let order = serde_json::json!({
                "model": model.path(),
                "index": model.index(),
                "pitch": model.pitch,
                "rate": s.link.rate,
            });
            if send(&mut h.stdin, b'L', order.to_string().as_bytes()).is_err() {
                helper = None;
                continue;
            }
        }
        if h.loading {
            drain(&mut s.input);
            continue;
        }
        s.link.block.store(h.block, Ordering::Relaxed);
        s.link.cushion.store(h.cushion, Ordering::Relaxed);
        while s.input.slots() >= h.block {
            let Ok(chunk) = s.input.read_chunk(h.block) else { break };
            frame.clear();
            frame.extend_from_slice(&f32::from_bits(s.link.key.load(Ordering::Relaxed)).to_le_bytes());
            let (a, b) = chunk.as_slices();
            for x in a.iter().chain(b) {
                frame.extend_from_slice(&x.to_le_bytes());
            }
            chunk.commit_all();
            if send(&mut h.stdin, b'A', &frame).is_err() {
                helper = None;
                break;
            }
        }
    }
}
