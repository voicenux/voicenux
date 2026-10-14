//! Phase vocoder pitch shifter with separate formant control.
//!
//! The spectrum of every frame is split into a smooth envelope (the formants, found with
//! the cepstrum) and the fine structure (the harmonics). The harmonics move by the pitch
//! ratio and the envelope by the formant ratio, so a voice can go up an octave without
//! sounding like a chipmunk, or the other way round.

use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use super::util::{DelayLine, TAU, semitones_to_ratio};
use super::{BuildCtx, Ctx, Effect};

pub struct PitchShifter {
    n: usize,
    hop: usize,
    rate: f32,
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex32>,
    work: Vec<Complex32>,
    cep: Vec<Complex32>,
    window: Vec<f32>,
    in_fifo: Vec<f32>,
    out_fifo: Vec<f32>,
    accum: Vec<f32>,
    rover: usize,
    last_phase: Vec<f32>,
    syn_phase: Vec<f32>,
    mag: Vec<f32>,
    phase: Vec<f32>,
    freq: Vec<f32>,
    out: Vec<Complex32>,
    peaks: Vec<usize>,
    env: Vec<f32>,
    lifter: usize,
    /// Pitch ratio, 2.0 is one octave up.
    pub ratio: f32,
    /// Formant ratio, 1.0 keeps the formants where they are.
    pub formant: f32,
}

impl PitchShifter {
    pub fn new(rate: f32, low_latency: bool) -> Self {
        let n = if low_latency || rate < 32000.0 { 1024 } else { 2048 };
        let n = if rate > 60000.0 { n * 2 } else { n };
        let hop = n / 8;
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);
        let ifft = planner.plan_fft_inverse(n);
        let scratch_len = fft.get_inplace_scratch_len().max(ifft.get_inplace_scratch_len());
        let window = (0..n).map(|k| 0.5 - 0.5 * (TAU * k as f32 / n as f32).cos()).collect();
        let half = n / 2 + 1;
        Self {
            n,
            hop,
            rate,
            fft,
            ifft,
            scratch: vec![Complex32::default(); scratch_len],
            work: vec![Complex32::default(); n],
            cep: vec![Complex32::default(); n],
            window,
            in_fifo: vec![0.0; n],
            out_fifo: vec![0.0; n],
            accum: vec![0.0; 2 * n],
            rover: n - hop,
            last_phase: vec![0.0; half],
            syn_phase: vec![0.0; half],
            mag: vec![0.0; half],
            phase: vec![0.0; half],
            freq: vec![0.0; half],
            out: vec![Complex32::default(); half],
            peaks: Vec::with_capacity(half),
            env: vec![1.0; half],
            // Keep quefrencies below the period of a 600 Hz voice: smooth enough to drop
            // the harmonics, fine enough to keep the formants.
            lifter: ((rate / 600.0) as usize).clamp(8, n / 4),
            ratio: 1.0,
            formant: 1.0,
        }
    }

    /// Delay the shifter adds, in samples.
    pub fn latency(&self) -> usize {
        self.n - self.hop
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        let latency = self.latency();
        for x in buf.iter_mut() {
            self.in_fifo[self.rover] = *x;
            *x = self.out_fifo[self.rover - latency];
            self.rover += 1;
            if self.rover >= self.n {
                self.rover = latency;
                self.frame();
            }
        }
    }

    /// One hop. Peaks keep the phases of their neighbour bins (identity phase locking after
    /// Laroche and Dolson), so harmonics stay clean and do not smear.
    fn frame(&mut self) {
        let n = self.n;
        let half = n / 2;
        let hop = self.hop as f32;
        let bin_hz = self.rate / n as f32;
        let expct = TAU * hop / n as f32;
        let ratio = self.ratio.clamp(0.25, 4.0);
        let formant = self.formant.clamp(0.25, 4.0);

        for k in 0..n {
            self.work[k] = Complex32::new(self.in_fifo[k] * self.window[k], 0.0);
        }
        self.fft.process_with_scratch(&mut self.work, &mut self.scratch);

        let mut max = 0.0f32;
        for k in 0..=half {
            let c = self.work[k];
            let mag = c.norm();
            let phase = c.im.atan2(c.re);
            let mut d = phase - self.last_phase[k] - k as f32 * expct;
            d -= TAU * (d / TAU).round();
            self.last_phase[k] = phase;
            self.freq[k] = (k as f32 + d / expct) * bin_hz;
            self.mag[k] = mag;
            self.phase[k] = phase;
            max = max.max(mag);
        }

        // When formants should move exactly with the pitch, a plain shift is all it takes.
        let use_env = (formant - ratio).abs() > 1e-3;
        if use_env {
            self.envelope();
            for k in 0..=half {
                self.mag[k] /= self.env[k];
            }
        }

        self.peaks.clear();
        let floor = max * 1e-5;
        for k in 2..half - 1 {
            let m = self.mag[k];
            if m > floor && m > self.mag[k - 1] && m >= self.mag[k + 1] && m > self.mag[k - 2] && m >= self.mag[k + 2] {
                self.peaks.push(k);
            }
        }

        self.out.fill(Complex32::default());
        let last_syn = &self.syn_phase;
        for (i, &p) in self.peaks.iter().enumerate() {
            // The region of a peak reaches to the lowest bin between it and its neighbours.
            let lo = if i == 0 {
                0
            } else {
                let prev = self.peaks[i - 1];
                (prev + 1..=p).min_by(|a, b| self.mag[*a].total_cmp(&self.mag[*b])).unwrap_or(p)
            };
            let hi = match self.peaks.get(i + 1) {
                Some(&next) => (p..next).min_by(|a, b| self.mag[*a].total_cmp(&self.mag[*b])).unwrap_or(p),
                None => half,
            };
            let target = (p as f32 * ratio).round() as isize;
            if target < 1 || target as usize >= half {
                continue;
            }
            let shift = target - p as isize;
            let new_freq = self.freq[p] * ratio;
            let phi = last_syn[target as usize] + TAU * new_freq * hop / self.rate;
            let rot = phi - self.phase[p];
            for k in lo..=hi {
                let j = k as isize + shift;
                if j < 0 || j as usize > half {
                    continue;
                }
                let (s, c) = (self.phase[k] + rot).sin_cos();
                self.out[j as usize] += Complex32::new(self.mag[k] * c, self.mag[k] * s);
            }
        }

        for j in 0..=half {
            let c = self.out[j];
            let phase = if c.norm_sqr() > 0.0 {
                c.im.atan2(c.re)
            } else {
                // Nothing here this frame: let the phase run on at the bin frequency.
                self.syn_phase[j] + j as f32 * expct
            };
            self.syn_phase[j] = phase - TAU * (phase / TAU).round();
        }

        if use_env {
            for j in 0..=half {
                let src = j as f32 / formant;
                let e = if src >= half as f32 {
                    0.0
                } else {
                    let i = src as usize;
                    let f = src - i as f32;
                    self.env[i] * (1.0 - f) + self.env[(i + 1).min(half)] * f
                };
                self.out[j] *= e;
            }
        }

        self.work[..=half].copy_from_slice(&self.out);
        for k in 1..half {
            self.work[n - k] = self.out[k].conj();
        }
        self.ifft.process_with_scratch(&mut self.work, &mut self.scratch);

        // Hann analysis and synthesis windows at 8x overlap add up to 3x the gain.
        let norm = 1.0 / (n as f32 * 3.0);
        for k in 0..n {
            self.accum[k] += self.window[k] * self.work[k].re * norm;
        }
        self.out_fifo[..self.hop].copy_from_slice(&self.accum[..self.hop]);
        self.accum.copy_within(self.hop.., 0);
        let len = self.accum.len();
        self.accum[len - self.hop..].fill(0.0);
        self.in_fifo.copy_within(self.hop.., 0);
    }

    /// Smooth spectral envelope of the current frame by cepstral liftering.
    fn envelope(&mut self) {
        let n = self.n;
        let half = n / 2;
        for k in 0..=half {
            self.cep[k] = Complex32::new((self.mag[k] + 1e-6).ln(), 0.0);
        }
        for k in 1..half {
            self.cep[n - k] = self.cep[k];
        }
        self.ifft.process_with_scratch(&mut self.cep, &mut self.scratch);
        let l = self.lifter;
        for q in 0..n {
            let dist = q.min(n - q);
            if dist >= l {
                self.cep[q] = Complex32::default();
            } else if dist > l / 2 {
                // Taper the cut so the envelope does not ring.
                let t = (dist - l / 2) as f32 / (l - l / 2) as f32;
                self.cep[q] *= 0.5 + 0.5 * (std::f32::consts::PI * t).cos();
            }
        }
        self.fft.process_with_scratch(&mut self.cep, &mut self.scratch);
        let scale = 1.0 / n as f32;
        for k in 0..=half {
            self.env[k] = (self.cep[k].re * scale).exp().max(1e-6);
        }
    }
}

/// Keeps the dry signal in time with a shifted one.
pub struct DryDelay {
    line: DelayLine,
    delay: usize,
}

impl DryDelay {
    pub fn new(delay: usize) -> Self {
        Self { line: DelayLine::new(delay + 2), delay }
    }

    #[inline]
    pub fn next(&mut self, x: f32) -> f32 {
        self.line.push(x);
        self.line.tap(self.delay)
    }
}

/// The "pitch" block: pitch and formant in semitones.
pub struct Pitch {
    shifter: PitchShifter,
    dry: DryDelay,
    mix: f32,
    scratch: Vec<f32>,
}

impl Pitch {
    pub const SEMITONES: usize = 0;
    pub const FORMANT: usize = 1;
    pub const MIX: usize = 2;

    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let shifter = PitchShifter::new(ctx.rate, ctx.low_latency);
        let dry = DryDelay::new(shifter.latency());
        let mut s = Self { shifter, dry, mix: 1.0, scratch: vec![0.0; ctx.max_block] };
        for (i, v) in p.iter().enumerate() {
            s.set_param(i, *v);
        }
        s
    }
}

impl Effect for Pitch {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        if self.mix >= 0.999 {
            self.shifter.process(buf);
            return;
        }
        let n = buf.len().min(self.scratch.len());
        let (buf, _) = buf.split_at_mut(n);
        self.scratch[..n].copy_from_slice(buf);
        self.shifter.process(buf);
        for (y, x) in buf.iter_mut().zip(&self.scratch[..n]) {
            *y = *y * self.mix + self.dry.next(*x) * (1.0 - self.mix);
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            Self::SEMITONES => self.shifter.ratio = semitones_to_ratio(v),
            Self::FORMANT => self.shifter.formant = semitones_to_ratio(v),
            Self::MIX => self.mix = v.clamp(0.0, 1.0),
            _ => {}
        }
    }
}

/// Up to three shifted copies of the voice on top of the dry voice: choirs, demons, duets.
pub struct Harmony {
    voices: Vec<(PitchShifter, f32)>,
    dry: DryDelay,
    dry_level: f32,
    level: f32,
    follow: f32,
    detune: f32,
    input: Vec<f32>,
    tmp: Vec<f32>,
}

impl Harmony {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let voices: Vec<(PitchShifter, f32)> =
            (0..3).map(|_| (PitchShifter::new(ctx.rate, ctx.low_latency), 0.0)).collect();
        let dry = DryDelay::new(voices[0].0.latency());
        let mut s = Self {
            voices,
            dry,
            dry_level: 1.0,
            level: 0.7,
            follow: 0.0,
            detune: 0.0,
            input: vec![0.0; ctx.max_block],
            tmp: vec![0.0; ctx.max_block],
        };
        for (i, v) in p.iter().enumerate() {
            s.set_param(i, *v);
        }
        s
    }

    fn retune(&mut self) {
        let detune = self.detune / 100.0;
        for (i, (sh, st)) in self.voices.iter_mut().enumerate() {
            let spread = match i {
                0 => detune,
                1 => -detune,
                _ => detune * 0.5,
            };
            let st = *st + spread;
            sh.ratio = semitones_to_ratio(st);
            sh.formant = semitones_to_ratio(st * self.follow);
        }
    }
}

impl Effect for Harmony {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        let n = buf.len().min(self.input.len());
        let buf = &mut buf[..n];
        self.input[..n].copy_from_slice(buf);
        for (y, x) in buf.iter_mut().zip(&self.input[..n]) {
            *y = self.dry.next(*x) * self.dry_level;
        }
        let active = self.voices.iter().filter(|(_, st)| *st != 0.0).count().max(1);
        let gain = self.level / (active as f32).sqrt();
        for (sh, st) in self.voices.iter_mut() {
            if *st == 0.0 {
                continue;
            }
            self.tmp[..n].copy_from_slice(&self.input[..n]);
            sh.process(&mut self.tmp[..n]);
            for (y, v) in buf.iter_mut().zip(&self.tmp[..n]) {
                *y += v * gain;
            }
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0..=2 => self.voices[index].1 = v,
            3 => self.level = v,
            4 => self.dry_level = v,
            5 => self.follow = v,
            6 => self.detune = v,
            _ => {}
        }
        self.retune();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::analysis::{dominant_freq, spectral_peak_in};

    fn saw(freq: f32, rate: f32, len: usize) -> Vec<f32> {
        (0..len).map(|i| ((i as f32 * freq / rate).fract() * 2.0 - 1.0) * 0.5).collect()
    }

    fn sine(freq: f32, rate: f32, len: usize) -> Vec<f32> {
        (0..len).map(|i| (TAU * freq * i as f32 / rate).sin() * 0.5).collect()
    }

    #[test]
    fn unity_ratio_keeps_level() {
        let rate = 48000.0;
        let mut ps = PitchShifter::new(rate, false);
        let mut x = sine(440.0, rate, 48000);
        ps.process(&mut x);
        let tail = &x[24000..];
        let peak = tail.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 0.05, "peak {peak}");
    }

    #[test]
    fn octave_up_doubles_frequency() {
        let rate = 48000.0;
        let mut ps = PitchShifter::new(rate, false);
        ps.ratio = 2.0;
        ps.formant = 2.0;
        let mut x = sine(220.0, rate, 48000);
        ps.process(&mut x);
        let f = dominant_freq(&x[16384..], rate);
        assert!((f - 440.0).abs() < 8.0, "got {f}");
    }

    #[test]
    fn formant_stays_when_asked() {
        let rate = 48000.0;
        // A buzzy 150 Hz voice with one resonance at 1000 Hz.
        let mut bp = crate::dsp::biquad::Biquad::new(crate::dsp::biquad::FilterType::Bandpass, 1000.0, 4.0, 0.0, rate);
        let src: Vec<f32> = saw(150.0, rate, 48000).into_iter().map(|v| bp.process(v)).collect();

        let mut keep = PitchShifter::new(rate, false);
        keep.ratio = semitones_to_ratio(7.0);
        keep.formant = 1.0;
        let mut a = src.clone();
        keep.process(&mut a);

        let mut chip = PitchShifter::new(rate, false);
        chip.ratio = semitones_to_ratio(7.0);
        chip.formant = semitones_to_ratio(7.0);
        let mut b = src.clone();
        chip.process(&mut b);

        let pa = spectral_peak_in(&a[16384..], rate, 500.0, 3000.0);
        let pb = spectral_peak_in(&b[16384..], rate, 500.0, 3000.0);
        assert!((pa - 1000.0).abs() < 250.0, "kept formant at {pa}");
        assert!((pb - 1498.0).abs() < 300.0, "moved formant at {pb}");
    }
}
