//! Installs what the AI voices need with one click: uv, its own Python, PyTorch, the RVC code
//! and the speech models, all inside `ai::dir()`. Also downloads the voices. Everything here
//! blocks: call it off the main thread.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, bail};

use super::{Model, dir, on_cpu};

/// Raise it when the helper needs other packages: the next click installs again.
const VERSION: &str = "1";
/// The uv that installs Python, when the computer has none.
const UV_VERSION: &str = "0.12.23";
/// PyTorch built for CUDA 12.6 runs with NVIDIA drivers from 525 on. The default build from
/// PyPI wants CUDA 13 and a very new driver.
const TORCH_CUDA: &str = "https://download.pytorch.org/whl/cu126";
const TORCH_CPU: &str = "https://download.pytorch.org/whl/cpu";
const TORCH: &str = "torch==2.14.1";
const PACKAGES: &[&str] = &["infer-rvc-python==1.3.1", "setuptools<80", "soxr"];
/// No voice file is near this big.
const MAX_MODEL: u64 = 400 * 1024 * 1024;

const HELPER: &str = include_str!("../../data/ai/helper.py");

/// The steps of the install, for its progress.
pub const STEPS: usize = 5;

/// Set to stop the install or download that runs. One runs at a time.
static STOP: AtomicBool = AtomicBool::new(false);

/// Call before an install or download starts, so an old stop does not end it.
pub fn begin() {
    STOP.store(false, Ordering::Relaxed);
}

/// Stops the install or download that runs, as soon as it can.
pub fn stop() {
    STOP.store(true, Ordering::Relaxed);
}

fn stopped() -> bool {
    STOP.load(Ordering::Relaxed)
}

/// What an install or download that was stopped ends with.
#[derive(Debug)]
pub struct Stopped;

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("stopped")
    }
}

impl std::error::Error for Stopped {}

/// About how much each step downloads, in MB, until it says itself: uv, Python, PyTorch, the
/// voice converter and the speech models.
pub fn step_mb() -> [u64; STEPS] {
    if on_cpu() { [15, 30, 250, 300, 560] } else { [15, 30, 3000, 300, 560] }
}

pub fn python() -> PathBuf {
    dir().join("venv").join("bin").join("python")
}

/// The helper script, written fresh, so it always fits this Voicenux.
pub fn write_helper() -> std::io::Result<PathBuf> {
    let path = dir().join("helper.py");
    std::fs::create_dir_all(dir())?;
    std::fs::write(&path, HELPER)?;
    Ok(path)
}

/// The PyTorch build the setting wants. The one for CUDA also runs on the processor, the one for
/// the processor is much smaller.
fn build() -> &'static str {
    if on_cpu() { "cpu" } else { "cuda" }
}

/// The engine is there, in a build that fits the setting: "1 cuda" or "1 cpu" in `ready`.
pub fn installed() -> bool {
    let ready = std::fs::read_to_string(dir().join("ready")).unwrap_or_default();
    let mut parts = ready.split_whitespace();
    python().is_file() && parts.next() == Some(VERSION) && parts.next().is_some_and(|b| b == "cuda" || b == build())
}

/// About how big the engine is, in GB, for the download dialog. Measured: PyTorch for the
/// graphics card brings 6 GB of NVIDIA libraries and the like.
pub fn engine_gb() -> u32 {
    if on_cpu() { 3 } else { 8 }
}

/// Where the helper keeps its speech models and the Hugging Face cache.
pub fn models_dir() -> PathBuf {
    dir().join("models")
}

pub fn hf_home() -> PathBuf {
    dir().join("hf")
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(concat!("Voicenux/", env!("CARGO_PKG_VERSION"), " (+https://github.com/voicenux/voicenux)"))
        .build()
        .into()
}

/// Downloads `url` into `path`, telling `progress` the bytes done and in all.
fn download(url: &str, path: &Path, max: u64, progress: &dyn Fn(u64, u64)) -> anyhow::Result<()> {
    let mut resp = agent().get(url).call().with_context(|| format!("download {url}"))?;
    let total: u64 =
        resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
    if total > max {
        bail!("{url} is too big");
    }
    let part = path.with_extension("part");
    std::fs::create_dir_all(path.parent().unwrap())?;
    let mut file = std::fs::File::create(&part)?;
    let mut reader = resp.body_mut().with_config().limit(max).reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        if stopped() {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err(Stopped.into());
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        if total > 0 {
            progress(done, total);
        }
    }
    file.sync_all()?;
    std::fs::rename(&part, path)?;
    Ok(())
}

/// Runs a command that downloads, and tells `progress` the bytes done and in all. uv names each
/// bigger download with its size as it starts, like "Downloading torch (846.7MiB)"; until then,
/// or for other commands, `expected` stands in. What came in so far is read from the network
/// counters of the computer, so other downloads at the same time make it look a bit further.
fn run_watched(cmd: &mut Command, expected: u64, progress: &dyn Fn(u64, u64)) -> anyhow::Result<()> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("{cmd:?}"))?;
    let stderr = child.stderr.take().expect("piped");
    let announced = Arc::new(AtomicU64::new(0));
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let reader = std::thread::spawn({
        let (announced, tail) = (announced.clone(), tail.clone());
        move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(bytes) = announced_size(&line) {
                    announced.fetch_add(bytes, Ordering::Relaxed);
                }
                let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                tail.push_back(line);
                if tail.len() > 6 {
                    tail.pop_front();
                }
            }
        }
    });
    let start = received();
    let total = || match announced.load(Ordering::Relaxed) {
        0 => expected,
        n => n,
    };
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if stopped() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(Stopped.into());
        }
        let total = total();
        progress(received().saturating_sub(start).min(total / 100 * 99), total);
        std::thread::sleep(Duration::from_millis(250));
    };
    let _ = reader.join();
    if !status.success() {
        let tail = tail.lock().unwrap_or_else(|e| e.into_inner());
        bail!("{}", tail.iter().cloned().collect::<Vec<_>>().join("\n"));
    }
    progress(total(), total());
    Ok(())
}

/// The size uv gives a download: "Downloading torch (846.7MiB)".
fn announced_size(line: &str) -> Option<u64> {
    let rest = line.trim().strip_prefix("Downloading ")?;
    let size = rest.rsplit_once('(')?.1.strip_suffix(')')?;
    let unit_at = size.find(|c: char| c.is_ascii_alphabetic())?;
    let n: f64 = size[..unit_at].trim().parse().ok()?;
    let unit = match &size[unit_at..] {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((n * unit) as u64)
}

/// Bytes the computer received so far, on every network device but the loopback.
fn received() -> u64 {
    let text = std::fs::read_to_string("/proc/net/dev").unwrap_or_default();
    text.lines()
        .skip(2)
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            (name.trim() != "lo").then(|| rest.split_whitespace().next()?.parse::<u64>().ok()).flatten()
        })
        .sum()
}

/// Runs a command and turns a failure into the end of what it wrote.
fn run(cmd: &mut Command) -> anyhow::Result<()> {
    let out = cmd.stdin(Stdio::null()).output().with_context(|| format!("{cmd:?}"))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = text.lines().rev().take(6).collect();
        bail!("{}", tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    Ok(())
}

/// uv, which brings Python and the packages: the one on the computer, else a copy of our own.
fn uv(progress: &dyn Fn(u64, u64)) -> anyhow::Result<PathBuf> {
    let own = dir().join("bin").join("uv");
    if own.is_file() {
        return Ok(own);
    }
    if let Some(found) =
        std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("uv")).find(|f| f.is_file()))
    {
        return Ok(found);
    }
    let arch = std::env::consts::ARCH;
    let url =
        format!("https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/uv-{arch}-unknown-linux-gnu.tar.gz");
    let tar = dir().join("bin").join("uv.tar.gz");
    download(&url, &tar, 100 * 1024 * 1024, progress)?;
    run(Command::new("tar").arg("-xzf").arg(&tar).arg("-C").arg(dir().join("bin")).arg("--strip-components=1"))?;
    let _ = std::fs::remove_file(&tar);
    if !own.is_file() {
        bail!("uv is not in its download");
    }
    Ok(own)
}

/// uv with everything it keeps in our folder.
fn uv_cmd(uv: &Path) -> Command {
    let mut c = Command::new(uv);
    c.env("UV_PYTHON_INSTALL_DIR", dir().join("python"))
        .env("UV_CACHE_DIR", dir().join("cache"))
        .env("UV_PYTHON_PREFERENCE", "only-managed")
        .env("UV_NO_PROGRESS", "1");
    c
}

/// Installs the AI engine. `progress` hears the step, counted from 1 to [`STEPS`], with its
/// bytes done and in all.
pub fn install_engine(progress: &dyn Fn(usize, u64, u64)) -> anyhow::Result<()> {
    let result = install_steps(progress);
    // Stopped halfway, the parts would only take room: some GB. The voices stay.
    if result.as_ref().is_err_and(|e| e.is::<Stopped>()) {
        for name in ["cache", "venv", "python", "bin", "models", "hf"] {
            let _ = std::fs::remove_dir_all(dir().join(name));
        }
        let _ = std::fs::remove_file(dir().join("helper.py"));
    }
    result
}

fn install_steps(progress: &dyn Fn(usize, u64, u64)) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir())?;
    let _ = std::fs::remove_file(dir().join("ready"));
    let mb = step_mb().map(|m| m * 1_000_000);
    progress(1, 0, mb[0]);
    let uv = uv(&|done, total| progress(1, done, total))?;
    let step = |n: usize| move |done, total| progress(n, done, total);
    run_watched(
        uv_cmd(&uv).args(["venv", "--allow-existing", "--python", "3.11"]).arg(dir().join("venv")),
        mb[1],
        &step(2),
    )?;
    let index = if on_cpu() { TORCH_CPU } else { TORCH_CUDA };
    // Installed again, so a switch from the processor build to the CUDA one really happens:
    // uv takes "2.14.1+cpu" for "2.14.1" too.
    run_watched(
        uv_cmd(&uv).args(["pip", "install", "--python"]).arg(python()).args([
            "--index-url",
            index,
            "--reinstall-package",
            "torch",
            "--reinstall-package",
            "torchaudio",
            "--reinstall-package",
            "torchvision",
            TORCH,
            "torchaudio",
            "torchvision",
        ]),
        mb[2],
        &step(3),
    )?;
    run_watched(uv_cmd(&uv).args(["pip", "install", "--python"]).arg(python()).args(PACKAGES), mb[3], &step(4))?;
    let script = write_helper()?;
    run_watched(
        Command::new(python()).arg(script).arg("prepare").arg("--models").arg(models_dir()).env("HF_HOME", hf_home()),
        mb[4],
        &step(5),
    )?;
    // The downloads uv kept are as big as the install, and not needed again.
    let _ = std::fs::remove_dir_all(dir().join("cache"));
    std::fs::write(dir().join("ready"), format!("{VERSION} {}", build()))?;
    Ok(())
}

/// Downloads the voice of a model: the model and its index.
pub fn download_model(model: &Model, progress: &dyn Fn(f32)) -> anyhow::Result<()> {
    if stopped() {
        return Err(Stopped.into());
    }
    // PyTorch files are zip archives, Faiss indexes start with an I.
    let share = if model.index().is_some() { 55.0 / model.mb as f32 } else { 1.0 };
    let files = [
        (Some(model.path()), model.pth, &b"PK"[..], 0.0, share),
        (model.index(), model.index.unwrap_or_default(), &b"I"[..], share, 1.0 - share),
    ];
    for (path, name, magic, start, share) in files {
        let Some(path) = path.filter(|p| !p.is_file()) else { continue };
        download(&model.url(name), &path, MAX_MODEL, &|done, total| {
            progress(start + share * done as f32 / total as f32)
        })?;
        let mut head = vec![0u8; magic.len()];
        let ok = std::fs::File::open(&path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && head == magic;
        if !ok {
            let _ = std::fs::remove_file(&path);
            bail!("{name} of {} is broken", model.id);
        }
    }
    // Files of a model this voice used before.
    let keep = [Some(model.path()), model.index()];
    if let Ok(dir) = std::fs::read_dir(dir().join("voices")) {
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.split(['.', '-']).next().unwrap_or_default();
            if stem == model.id && !keep.contains(&Some(e.path())) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(())
}

/// Space the AI folder takes, in bytes.
pub fn size() -> u64 {
    fn walk(p: &Path) -> u64 {
        let Ok(meta) = std::fs::symlink_metadata(p) else { return 0 };
        if meta.is_dir() {
            std::fs::read_dir(p).map(|d| d.flatten().map(|e| walk(&e.path())).sum()).unwrap_or(0)
        } else {
            meta.len()
        }
    }
    walk(&dir())
}

/// Removes the AI engine and all voices.
pub fn remove() -> std::io::Result<()> {
    super::bridge::stop();
    match std::fs::remove_dir_all(dir()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_sizes_uv_gives() {
        let mib = |n: f64| (n * 1024.0 * 1024.0) as u64;
        assert_eq!(super::announced_size("Downloading torch (846.7MiB)"), Some(mib(846.7)));
        let python = "Downloading cpython-3.11.17-linux-x86_64-gnu (download) (29.4MiB)";
        assert_eq!(super::announced_size(python), Some(mib(29.4)));
        assert_eq!(super::announced_size("Downloading x (1.5GiB)"), Some(mib(1536.0)));
        assert_eq!(super::announced_size(" Downloaded torch"), None);
    }

    #[test]
    fn watches_a_command_that_downloads() {
        let seen = std::cell::RefCell::new(Vec::new());
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "echo 'Downloading x (2MiB)' >&2; sleep 0.6"]);
        super::run_watched(&mut cmd, 5_000_000, &|done, total| seen.borrow_mut().push((done, total))).unwrap();
        let seen = seen.borrow();
        assert_eq!(*seen.last().unwrap(), (2 * 1024 * 1024, 2 * 1024 * 1024));
        let mut fail = std::process::Command::new("sh");
        fail.args(["-c", "echo nope >&2; exit 3"]);
        assert!(super::run_watched(&mut fail, 1, &|_, _| {}).unwrap_err().to_string().contains("nope"));
        // Stop ends it at once and says so. Here, not in a test of its own: the stop is for all.
        let mut long = std::process::Command::new("sh");
        long.args(["-c", "sleep 20"]);
        let started = std::time::Instant::now();
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            super::stop();
        });
        let err = super::run_watched(&mut long, 1, &|_, _| {}).unwrap_err();
        assert!(err.is::<super::Stopped>());
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        super::begin();
    }

    /// Installs for real into `ai::dir()` and downloads a voice, so only on request:
    /// `XDG_DATA_HOME=/somewhere VOICENUX_AI_CPU=1 cargo test -- --ignored installs_and_downloads`
    #[test]
    #[ignore]
    fn installs_and_downloads() {
        let _ = env_logger::builder().is_test(true).try_init();
        super::install_engine(&|n, done, total| eprintln!("step {n}: {done} of {total}")).unwrap();
        assert!(super::installed());
        let vader = &crate::ai::MODELS[4];
        super::download_model(vader, &|_| {}).unwrap();
        assert!(vader.downloaded());
    }
}
