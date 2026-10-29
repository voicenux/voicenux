//! The part that runs on PipeWire's realtime thread. It never allocates, locks or frees:
//! new things arrive through a ring buffer and old things leave through another.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use nnnoiseless::DenoiseState;

use super::{Meters, RtCmd, Trash};
use crate::dsp::chain::Chain;
use crate::dsp::dynamics::{Gate, Limiter};
use crate::dsp::util::{db_to_gain, peak};
use crate::dsp::{BuildCtx, Ctx};
use crate::soundboard::{RATE as SOUND_RATE, SoundData};

const MAX: usize = 8192;
const MAX_SOUNDS: usize = 32;

/// A gain that glides over ~20 ms so switches never click.
#[derive(Clone, Copy)]
struct Ramp {
    value: f32,
    target: f32,
    step: f32,
}

impl Ramp {
    fn new(v: f32) -> Self {
        Self { value: v, target: v, step: 1.0 / 960.0 }
    }

    #[inline]
    fn next(&mut self) -> f32 {
        if self.value < self.target {
            self.value = (self.value + self.step).min(self.target);
        } else if self.value > self.target {
            self.value = (self.value - self.step).max(self.target);
        }
        self.value
    }
}

struct Playing {
    id: u32,
    data: Arc<SoundData>,
    pos: f64,
    volume: f32,
    /// Headphones only.
    preview: bool,
    /// Counts down to zero after a stop, for a short fade out.
    stopping: Option<u32>,
}

struct Denoiser {
    state: Box<DenoiseState<'static>>,
    input: [f32; DenoiseState::FRAME_SIZE],
    output: [f32; DenoiseState::FRAME_SIZE],
    fill: usize,
}

pub struct Processor {
    meters: Arc<Meters>,
    cmds: rtrb::Consumer<RtCmd>,
    trash: rtrb::Producer<Trash>,
    chain: Option<Box<Chain>>,
    old: Option<Box<Chain>>,
    fade: usize,
    fade_len: usize,
    voice_on: Ramp,
    hear: Ramp,
    mute: Ramp,
    /// Your voice while an app listens to you: nobody else hears it then.
    quiet: Ramp,
    /// Where the microphone goes while an app listens.
    listen: Option<rtrb::Producer<f32>>,
    hear_sounds: Ramp,
    background: bool,
    input_gain: f32,
    mic_volume: f32,
    monitor_volume: f32,
    sound_volume: f32,
    denoise: Option<Denoiser>,
    denoise_on: bool,
    gate: Gate,
    gate_on: bool,
    sounds: Vec<Playing>,
    limit_out: [Limiter; 2],
    limit_mon: [Limiter; 2],
    dry: Vec<f32>,
    wet: Vec<f32>,
    old_wet: Vec<f32>,
    snd: [Vec<f32>; 2],
    /// Previews, for the headphones only.
    pre: [Vec<f32>; 2],
}

impl Processor {
    pub fn new(meters: Arc<Meters>, cmds: rtrb::Consumer<RtCmd>, trash: rtrb::Producer<Trash>) -> Self {
        let rate = 48000.0;
        Self {
            meters,
            cmds,
            trash,
            chain: None,
            old: None,
            fade: 0,
            fade_len: 1,
            voice_on: Ramp::new(1.0),
            hear: Ramp::new(0.0),
            mute: Ramp::new(1.0),
            quiet: Ramp::new(1.0),
            listen: None,
            hear_sounds: Ramp::new(1.0),
            background: true,
            input_gain: 1.0,
            mic_volume: 1.0,
            monitor_volume: 0.8,
            sound_volume: 0.8,
            denoise: Some(Denoiser {
                state: DenoiseState::new(),
                input: [0.0; DenoiseState::FRAME_SIZE],
                output: [0.0; DenoiseState::FRAME_SIZE],
                fill: 0,
            }),
            denoise_on: false,
            gate: Gate::new(&[-50.0, 150.0], &BuildCtx::new(rate, false)),
            gate_on: false,
            sounds: Vec::with_capacity(MAX_SOUNDS),
            limit_out: [Limiter::new(rate), Limiter::new(rate)],
            limit_mon: [Limiter::new(rate), Limiter::new(rate)],
            dry: vec![0.0; MAX],
            wet: vec![0.0; MAX],
            old_wet: vec![0.0; MAX],
            snd: [vec![0.0; MAX], vec![0.0; MAX]],
            pre: [vec![0.0; MAX], vec![0.0; MAX]],
        }
    }

    fn throw(&mut self, t: Trash) {
        // A full trash queue is close to impossible. Dropping here is the lesser evil.
        let _ = self.trash.push(t);
    }

    fn commands(&mut self, rate: u32) {
        for _ in 0..64 {
            let Ok(cmd) = self.cmds.pop() else { break };
            match cmd {
                RtCmd::Chain(new) => {
                    if let Some(old) = self.old.take() {
                        self.throw(Trash::Chain(old));
                    }
                    self.old = self.chain.replace(new);
                    self.fade = 0;
                    self.fade_len = (rate as usize / 25).max(1);
                }
                RtCmd::Param { block, index, value } => {
                    if let Some(c) = &mut self.chain {
                        c.set_param(block as usize, index as usize, value);
                    }
                }
                RtCmd::VoiceOn(on) => self.voice_on.target = on as u8 as f32,
                RtCmd::HearMyself(on) => self.hear.target = on as u8 as f32,
                RtCmd::Mute(on) => self.mute.target = if on { 0.0 } else { 1.0 },
                RtCmd::HearSounds(on) => self.hear_sounds.target = on as u8 as f32,
                RtCmd::Background(on) => self.background = on,
                RtCmd::InputGain(db) => self.input_gain = db_to_gain(db),
                RtCmd::MicVolume(db) => self.mic_volume = db_to_gain(db),
                RtCmd::MonitorVolume(v) => self.monitor_volume = v,
                RtCmd::SoundVolume(v) => self.sound_volume = v,
                RtCmd::Denoise(on) => self.denoise_on = on,
                RtCmd::Gate(db) => {
                    self.gate_on = db.is_some();
                    if let Some(db) = db {
                        use crate::dsp::Effect;
                        self.gate.set_param(0, db);
                    }
                }
                RtCmd::Play { id, data, volume, restart, preview } => {
                    if restart {
                        for p in self.sounds.iter_mut().filter(|p| p.id == id) {
                            p.stopping.get_or_insert(480);
                        }
                    }
                    if self.sounds.len() >= MAX_SOUNDS {
                        let oldest = self.sounds.remove(0);
                        self.throw(Trash::Sound(oldest.data));
                    }
                    self.sounds.push(Playing { id, data, pos: 0.0, volume, preview, stopping: None });
                }
                RtCmd::Stop(id) => {
                    for p in self.sounds.iter_mut().filter(|p| p.id == id) {
                        p.stopping.get_or_insert(480);
                    }
                }
                RtCmd::StopAll => {
                    for p in &mut self.sounds {
                        p.stopping.get_or_insert(480);
                    }
                }
                RtCmd::SoundVolumeOf { id, volume } => {
                    for p in self.sounds.iter_mut().filter(|p| p.id == id) {
                        p.volume = volume;
                    }
                }
                RtCmd::Listen(ring) => {
                    self.quiet.target = if ring.is_some() { 0.0 } else { 1.0 };
                    if let Some(old) = std::mem::replace(&mut self.listen, ring) {
                        self.throw(Trash::Listen(old));
                    }
                }
            }
        }
    }

    /// One graph cycle. `input` is the microphone (None when nothing is linked).
    pub fn process(
        &mut self,
        rate: u32,
        input: Option<&[f32]>,
        out: [&mut [f32]; 2],
        monitor: Option<[&mut [f32]; 2]>,
    ) {
        self.meters.rate.store(rate, Ordering::Relaxed);
        self.commands(rate);
        let n = out[0].len().min(out[1].len());
        let mut done = 0;
        let [out_l, out_r] = out;
        let mut monitor = monitor;
        while done < n {
            let len = (n - done).min(MAX);
            let range = done..done + len;
            let mon = monitor.as_mut().map(|[l, r]| (&mut l[range.clone()], &mut r[range.clone()]));
            self.block(
                rate,
                input.map(|i| &i[range.clone()]),
                &mut out_l[range.clone()],
                &mut out_r[range.clone()],
                mon,
            );
            done += len;
        }
    }

    fn block(
        &mut self,
        rate: u32,
        input: Option<&[f32]>,
        out_l: &mut [f32],
        out_r: &mut [f32],
        monitor: Option<(&mut [f32], &mut [f32])>,
    ) {
        let n = out_l.len();
        let dry = &mut self.dry[..n];
        match input {
            Some(i) => {
                for (d, x) in dry.iter_mut().zip(i) {
                    *d = x * self.input_gain;
                }
            }
            None => dry.fill(0.0),
        }

        if self.denoise_on
            && rate == 48000
            && let Some(dn) = &mut self.denoise
        {
            for x in dry.iter_mut() {
                dn.input[dn.fill] = *x * 32768.0;
                *x = dn.output[dn.fill] / 32768.0;
                dn.fill += 1;
                if dn.fill == DenoiseState::FRAME_SIZE {
                    dn.fill = 0;
                    dn.state.process_frame(&mut dn.output, &dn.input);
                }
            }
        }
        if self.gate_on {
            for x in dry.iter_mut() {
                *x = self.gate.next(*x);
            }
        }
        self.meters.input.raise(peak(dry));
        if let Some(ring) = &mut self.listen {
            // A full ring drops the rest: nobody reads it anymore.
            for x in dry.iter() {
                if ring.push(*x).is_err() {
                    break;
                }
            }
        }

        let ctx = Ctx { background: self.background };
        let wet = &mut self.wet[..n];
        wet.copy_from_slice(dry);
        let chain_ok = self.chain.as_ref().is_some_and(|c| c.rate == rate);
        if chain_ok {
            self.chain.as_mut().unwrap().process(wet, &ctx);
        }
        let mut fading = false;
        if let Some(old) = &mut self.old {
            fading = true;
            let ow = &mut self.old_wet[..n];
            ow.copy_from_slice(dry);
            if old.rate == rate {
                old.process(ow, &ctx);
            }
            for (w, o) in wet.iter_mut().zip(ow.iter()) {
                let g = (self.fade as f32 / self.fade_len as f32).min(1.0);
                *w = *w * g + *o * (1.0 - g);
                self.fade += 1;
            }
        }
        if fading
            && self.fade >= self.fade_len
            && let Some(old) = self.old.take()
        {
            let _ = self.trash.push(Trash::Chain(old));
        }

        // Voice changer on/off, mute, listening and mic volume.
        let mut voice_peak = 0.0f32;
        for (w, d) in wet.iter_mut().zip(dry.iter()) {
            let on = self.voice_on.next();
            let v = (*w * on + *d * (1.0 - on)) * self.mute.next() * self.quiet.next() * self.mic_volume;
            voice_peak = voice_peak.max(v.abs());
            *w = v;
        }
        self.meters.output.raise(voice_peak);

        // Soundboard.
        let [sl, sr] = &mut self.snd;
        let (sl, sr) = (&mut sl[..n], &mut sr[..n]);
        sl.fill(0.0);
        sr.fill(0.0);
        let [pl, pr] = &mut self.pre;
        let (pl, pr) = (&mut pl[..n], &mut pr[..n]);
        pl.fill(0.0);
        pr.fill(0.0);
        let step = SOUND_RATE as f64 / rate as f64;
        let mut i = 0;
        while i < self.sounds.len() {
            let p = &mut self.sounds[i];
            let len = p.data.frames.len();
            let (bl, br) = if p.preview { (&mut *pl, &mut *pr) } else { (&mut *sl, &mut *sr) };
            let mut finished = false;
            for k in 0..n {
                let idx = p.pos as usize;
                if idx + 1 >= len {
                    finished = true;
                    break;
                }
                let frac = (p.pos - idx as f64) as f32;
                let a = p.data.frames[idx];
                let b = p.data.frames[idx + 1];
                let mut g = p.volume * self.sound_volume;
                if let Some(left) = &mut p.stopping {
                    g *= *left as f32 / 480.0;
                    if *left == 0 {
                        finished = true;
                        break;
                    }
                    *left -= 1;
                }
                bl[k] += (a[0] + (b[0] - a[0]) * frac) * g;
                br[k] += (a[1] + (b[1] - a[1]) * frac) * g;
                p.pos += step;
            }
            if finished {
                let p = self.sounds.swap_remove(i);
                let _ = self.trash.push(Trash::Sound(p.data));
            } else {
                i += 1;
            }
        }
        self.meters.sounds.raise(peak(sl).max(peak(sr)));
        for (slot, m) in self.meters.playing.iter().enumerate() {
            let v = self.sounds.get(slot).map(|p| {
                let progress = (p.pos / p.data.frames.len() as f64 * 1000.0) as u64;
                ((p.id as u64) << 32) | progress.min(1000)
            });
            m.store(v.unwrap_or(0), Ordering::Relaxed);
        }

        for k in 0..n {
            let v = wet[k];
            out_l[k] = self.limit_out[0].next(v + sl[k]);
            out_r[k] = self.limit_out[1].next(v + sr[k]);
        }
        if let Some((ml, mr)) = monitor {
            for k in 0..n {
                let h = self.hear.next();
                let s = self.hear_sounds.next();
                let v = wet[k] * h;
                ml[k] = self.limit_mon[0].next((v + sl[k] * s + pl[k]) * self.monitor_volume);
                mr[k] = self.limit_mon[1].next((v + sr[k] * s + pr[k]) * self.monitor_volume);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{AtomicF32, RtCmd};
    use crate::dsp::chain::BlockValues;

    fn setup() -> (Processor, rtrb::Producer<RtCmd>, rtrb::Consumer<Trash>, Arc<Meters>) {
        let meters = Arc::new(Meters::default());
        let (tx, rx) = rtrb::RingBuffer::new(64);
        let (ttx, trx) = rtrb::RingBuffer::new(64);
        (Processor::new(meters.clone(), rx, ttx), tx, trx, meters)
    }

    fn run(p: &mut Processor, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut l = vec![0.0; input.len()];
        let mut r = vec![0.0; input.len()];
        let mut ml = vec![0.0; input.len()];
        let mut mr = vec![0.0; input.len()];
        for k in (0..input.len()).step_by(256) {
            let e = (k + 256).min(input.len());
            p.process(48000, Some(&input[k..e]), [&mut l[k..e], &mut r[k..e]], Some([&mut ml[k..e], &mut mr[k..e]]));
        }
        (l, ml)
    }

    #[test]
    fn passes_voice_through_and_monitors_on_request() {
        let (mut p, mut tx, _trash, meters) = setup();
        let input: Vec<f32> = (0..48000).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
        let (out, mon) = run(&mut p, &input);
        assert!((out[40000] - input[40000]).abs() < 1e-3);
        assert!(mon.iter().all(|v| *v == 0.0), "hear myself is off");
        assert!(AtomicF32::take(&meters.input) > 0.25);

        tx.push(RtCmd::HearMyself(true)).unwrap();
        let (_, mon) = run(&mut p, &input);
        assert!((mon[40000] - input[40000] * 0.8).abs() < 1e-3);
    }

    #[test]
    fn swaps_chains_and_returns_the_old_one() {
        let (mut p, mut tx, mut trash, _) = setup();
        let ctx = BuildCtx::new(48000.0, false);
        let voice = [BlockValues { kind: "gain".into(), enabled: true, values: vec![-6.0] }];
        tx.push(RtCmd::Chain(Box::new(Chain::build(&voice, &ctx)))).unwrap();
        let input = vec![0.5f32; 48000];
        let (out, _) = run(&mut p, &input);
        assert!((out[30000] - 0.5 * db_to_gain(-6.0)).abs() < 1e-3);
        tx.push(RtCmd::Chain(Box::new(Chain::empty(48000)))).unwrap();
        run(&mut p, &input);
        assert!(matches!(trash.pop(), Ok(Trash::Chain(_))));

        tx.push(RtCmd::VoiceOn(false)).unwrap();
        let (out, _) = run(&mut p, &input);
        assert!((out[30000] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn plays_and_finishes_sounds() {
        let (mut p, mut tx, mut trash, meters) = setup();
        let data = Arc::new(SoundData { frames: vec![[0.25, 0.25]; 4800] });
        tx.push(RtCmd::Play { id: 7, data, volume: 1.0, restart: true, preview: false }).unwrap();
        tx.push(RtCmd::SoundVolume(1.0)).unwrap();
        let (out, mon) = run(&mut p, &vec![0.0; 2400]);
        assert!((out[1000] - 0.25).abs() < 1e-3);
        assert!((mon[1000] - 0.2).abs() < 1e-3, "sounds are heard at monitor volume");
        assert_eq!(meters.playing[0].load(Ordering::Relaxed) >> 32, 7);
        run(&mut p, &vec![0.0; 4800]);
        assert!(matches!(trash.pop(), Ok(Trash::Sound(_))));
        assert_eq!(meters.playing[0].load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_listening_app_gets_the_microphone_and_nobody_else_hears_it() {
        let (mut p, mut tx, mut trash, _) = setup();
        let (ring, mut heard) = rtrb::RingBuffer::new(48000);
        tx.push(RtCmd::Listen(Some(ring))).unwrap();
        let input = vec![0.5f32; 24000];
        let (out, _) = run(&mut p, &input);
        assert!(out[20000].abs() < 1e-6, "others hear nothing");
        assert_eq!(heard.slots(), 24000);
        assert!(heard.pop().is_ok_and(|x| (x - 0.5).abs() < 1e-6));
        tx.push(RtCmd::Listen(None)).unwrap();
        let (out, _) = run(&mut p, &input);
        assert!((out[20000] - 0.5).abs() < 1e-3, "and then they hear you again");
        assert!(matches!(trash.pop(), Ok(Trash::Listen(_))));
    }

    #[test]
    fn previews_only_reach_the_headphones() {
        let (mut p, mut tx, _, _) = setup();
        let data = Arc::new(SoundData { frames: vec![[0.25, 0.25]; 4800] });
        tx.push(RtCmd::HearSounds(false)).unwrap();
        tx.push(RtCmd::Play { id: 9, data, volume: 1.0, restart: true, preview: true }).unwrap();
        tx.push(RtCmd::SoundVolume(1.0)).unwrap();
        let (out, mon) = run(&mut p, &vec![0.0; 2400]);
        assert!(out[1000].abs() < 1e-6, "a preview never goes into the microphone");
        assert!((mon[1000] - 0.2).abs() < 1e-3, "a preview is heard even with sounds off for me");
    }
}
