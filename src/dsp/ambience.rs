//! Background sounds: recordings from Freesound, all CC0 (see `data/ambience/SOURCES.md`). Each
//! one plays as a loop. They only play while Background is on.

use std::sync::OnceLock;

use super::util::db_to_gain;
use super::{BuildCtx, Ctx, Effect};

pub const AMBIENCES: &[&str] = &[
    "Bubbles",
    "Wind",
    "Rain",
    "Campfire",
    "Ocean",
    "Space",
    "Radio static",
    "Cave drips",
    "Heartbeat",
    "Computer",
    "Storm",
    "Crowd",
    "Birds",
    "Engine",
    "Shower",
    "Cauldron",
];

/// The recordings, in the order of `AMBIENCES`. Mono, leveled to the same loudness.
const FILES: [&[u8]; 16] = [
    include_bytes!("../../data/ambience/bubbles.ogg"),
    include_bytes!("../../data/ambience/wind.ogg"),
    include_bytes!("../../data/ambience/rain.ogg"),
    include_bytes!("../../data/ambience/campfire.ogg"),
    include_bytes!("../../data/ambience/ocean.ogg"),
    include_bytes!("../../data/ambience/space.ogg"),
    include_bytes!("../../data/ambience/static.ogg"),
    include_bytes!("../../data/ambience/drips.ogg"),
    include_bytes!("../../data/ambience/heartbeat.ogg"),
    include_bytes!("../../data/ambience/computer.ogg"),
    include_bytes!("../../data/ambience/storm.ogg"),
    include_bytes!("../../data/ambience/crowd.ogg"),
    include_bytes!("../../data/ambience/birds.ogg"),
    include_bytes!("../../data/ambience/engine.ogg"),
    include_bytes!("../../data/ambience/shower.ogg"),
    include_bytes!("../../data/ambience/cauldron.ogg"),
];

/// The loops at 48 kHz, decoded the first time a voice needs them. They stay until Voicenux
/// quits, so the realtime thread can switch between them without freeing anything.
static LOOPS: [OnceLock<Box<[i16]>>; 16] = [const { OnceLock::new() }; 16];

/// How long the end of a recording fades into its start, so the loop has no seam.
const SEAM: f32 = 1.5;

/// Decodes the loop of `kind` unless that happened already. Takes a moment the first time, so
/// never on the realtime thread.
pub fn load(kind: usize) -> &'static [i16] {
    let kind = kind.min(AMBIENCES.len() - 1);
    LOOPS[kind].get_or_init(|| match crate::soundboard::decode_bytes(FILES[kind].to_vec(), "ogg") {
        Ok(d) => {
            let mono: Vec<f32> = d.frames.iter().map(|f| (f[0] + f[1]) * 0.5).collect();
            make_loop(&mono, (SEAM * crate::soundboard::RATE as f32) as usize)
                .iter()
                .map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect()
        }
        Err(e) => {
            log::error!("could not decode the background {}: {e}", AMBIENCES[kind]);
            Box::default()
        }
    })
}

/// Cuts `fade` samples off the end and fades them into the start, at equal power: the
/// recordings are noisy, so the two halves do not cancel out.
fn make_loop(x: &[f32], fade: usize) -> Vec<f32> {
    let fade = fade.min(x.len() / 3);
    let n = x.len() - fade;
    let mut out = x[..n].to_vec();
    for (i, v) in out.iter_mut().take(fade).enumerate() {
        let t = i as f32 / fade as f32 * std::f32::consts::FRAC_PI_2;
        *v = *v * t.sin() + x[n + i] * t.cos();
    }
    out
}

pub struct Ambience {
    clip: &'static [i16],
    level: f32,
    /// Where in the loop, in samples of the recording.
    pos: f64,
    /// Recording samples per engine sample: 1 unless PipeWire runs at another rate than 48 kHz.
    step: f64,
    /// Fades in and out when Background is turned on or off, so it does not click.
    fade: f32,
    fade_step: f32,
}

impl Ambience {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        load(p.first().map_or(0, |v| *v as usize));
        let mut a = Self {
            clip: &[],
            level: db_to_gain(-16.0),
            pos: 0.0,
            step: crate::soundboard::RATE as f64 / ctx.rate as f64,
            fade: 0.0,
            fade_step: 1.0 / (0.08 * ctx.rate),
        };
        for (i, v) in p.iter().enumerate() {
            a.set_param(i, *v);
        }
        a
    }
}

impl Effect for Ambience {
    fn process(&mut self, buf: &mut [f32], ctx: &Ctx) {
        let target = if ctx.background { 1.0 } else { 0.0 };
        let n = self.clip.len();
        if n < 2 || (self.fade == 0.0 && target == 0.0) {
            return;
        }
        let gain = self.level / 32768.0;
        for x in buf.iter_mut() {
            self.fade = (self.fade + (target - self.fade).clamp(-self.fade_step, self.fade_step)).clamp(0.0, 1.0);
            let i = self.pos as usize;
            let f = (self.pos - i as f64) as f32;
            let (a, b) = (self.clip[i] as f32, self.clip[(i + 1) % n] as f32);
            *x += (a + (b - a) * f) * gain * self.fade;
            self.pos += self.step;
            if self.pos >= n as f64 {
                self.pos -= n as f64;
            }
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            // Only takes a loop that is decoded already: `load` it first.
            0 => {
                let kind = (v as usize).min(AMBIENCES.len() - 1);
                self.clip = LOOPS[kind].get().map_or(&[], |c| &c[..]);
                if !self.clip.is_empty() {
                    self.pos %= self.clip.len() as f64;
                }
            }
            1 => self.level = db_to_gain(v),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ambience_makes_sound_and_stays_sane() {
        let ctx = BuildCtx::test();
        for (kind, name) in AMBIENCES.iter().enumerate() {
            let mut a = Ambience::new(&[kind as f32, 0.0], &ctx);
            let mut x = vec![0.0; 48000 * 12];
            a.process(&mut x, &Ctx { background: true });
            let rms = crate::dsp::util::rms(&x);
            let peak = crate::dsp::util::peak(&x);
            assert!(rms > 0.005, "{name} too quiet: {rms}");
            assert!(peak <= 1.0 && x.iter().all(|v| v.is_finite()), "{name} too loud: {peak}");
        }
    }

    #[test]
    fn loops_have_no_seam() {
        for (kind, name) in AMBIENCES.iter().enumerate() {
            let c = load(kind);
            assert!(c.len() > 48000 * 15, "{name} is a short loop");
            // The step over the seam is no bigger than the steps inside the loop.
            let jump = |i: usize| (c[(i + 1) % c.len()] as i32 - c[i] as i32).abs();
            let inside = (0..c.len() - 1).map(jump).max().unwrap();
            assert!(jump(c.len() - 1) <= inside, "{name} clicks where it loops");
        }
    }

    #[test]
    fn silent_without_background() {
        let ctx = BuildCtx::test();
        let mut a = Ambience::new(&[1.0, 0.0], &ctx);
        let mut x = vec![0.0; 4800];
        a.process(&mut x, &Ctx { background: false });
        assert!(x.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn fades_out_when_turned_off() {
        let ctx = BuildCtx::test();
        let mut a = Ambience::new(&[11.0, 0.0], &ctx);
        let mut x = vec![0.0; 48000];
        a.process(&mut x, &Ctx { background: true });
        let mut y = vec![0.0; 48000];
        a.process(&mut y, &Ctx { background: false });
        assert!(y[..100].iter().any(|v| *v != 0.0), "stops at once");
        assert!(y[48000 / 4..].iter().all(|v| *v == 0.0), "goes on playing");
    }
}
