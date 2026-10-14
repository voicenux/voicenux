//! Effects that rebuild the voice from its spectrum: robot (all phases zero, so every
//! frame becomes one buzzy pulse) and whisper (random phases, so the voice turns to breath).

use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use super::dynamics::Agc;
use super::pitch::DryDelay;
use super::util::{Rng, TAU};
use super::{BuildCtx, Ctx, Effect};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Robot,
    Whisper,
}

pub struct Spectral {
    mode: Mode,
    n: usize,
    rate: f32,
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex32>,
    work: Vec<Complex32>,
    window: Vec<f32>,
    history: Vec<f32>,
    hist_pos: usize,
    out: Vec<f32>,
    out_pos: usize,
    since: usize,
    hop: usize,
    rng: Rng,
    agc: Agc,
    dry: DryDelay,
    mix: f32,
}

impl Spectral {
    fn build(mode: Mode, ctx: &BuildCtx) -> Self {
        let n = if mode == Mode::Robot { 1024 } else { 512 };
        let n = if ctx.rate > 60000.0 { n * 2 } else { n };
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);
        let ifft = planner.plan_fft_inverse(n);
        let scratch = fft.get_inplace_scratch_len().max(ifft.get_inplace_scratch_len());
        Self {
            mode,
            n,
            rate: ctx.rate,
            fft,
            ifft,
            scratch: vec![Complex32::default(); scratch],
            work: vec![Complex32::default(); n],
            window: (0..n).map(|k| 0.5 - 0.5 * (TAU * k as f32 / n as f32).cos()).collect(),
            history: vec![0.0; n],
            hist_pos: 0,
            out: vec![0.0; n * 2],
            out_pos: 0,
            since: 0,
            hop: n / 4,
            rng: Rng::new(0x5eed),
            agc: Agc::new(ctx.rate),
            dry: DryDelay::new(n),
            mix: 1.0,
        }
    }

    pub fn robot(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut s = Self::build(Mode::Robot, ctx);
        for (i, v) in p.iter().enumerate() {
            s.set_param(i, *v);
        }
        s
    }

    pub fn whisper(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut s = Self::build(Mode::Whisper, ctx);
        for (i, v) in p.iter().enumerate() {
            s.set_param(i, *v);
        }
        s
    }

    fn frame(&mut self) {
        let n = self.n;
        for k in 0..n {
            let x = self.history[(self.hist_pos + k) % n];
            self.work[k] = Complex32::new(x * self.window[k], 0.0);
        }
        self.fft.process_with_scratch(&mut self.work, &mut self.scratch);
        for k in 0..n {
            let mag = self.work[k].norm();
            self.work[k] = match self.mode {
                // Zero phase, shifted to the middle of the frame (-1 for odd bins).
                Mode::Robot => Complex32::new(if k % 2 == 0 { mag } else { -mag }, 0.0),
                Mode::Whisper => {
                    let (s, c) = (self.rng.unit() * TAU).sin_cos();
                    Complex32::new(mag * c, mag * s)
                }
            };
        }
        self.ifft.process_with_scratch(&mut self.work, &mut self.scratch);
        let norm = 1.0 / n as f32;
        let len = self.out.len();
        for k in 0..n {
            self.out[(self.out_pos + k) % len] += self.work[k].re * self.window[k] * norm;
        }
    }
}

impl Effect for Spectral {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        let len = self.out.len();
        for x in buf.iter_mut() {
            let input = *x;
            self.history[self.hist_pos] = input;
            self.hist_pos = (self.hist_pos + 1) % self.n;
            self.since += 1;
            if self.since >= self.hop {
                self.since = 0;
                self.frame();
            }
            let y = self.out[self.out_pos];
            self.out[self.out_pos] = 0.0;
            self.out_pos = (self.out_pos + 1) % len;
            let wet = self.agc.next(input, y);
            let dry = self.dry.next(input);
            *x = wet * self.mix + dry * (1.0 - self.mix);
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match (self.mode, index) {
            (Mode::Robot, 0) => {
                self.hop = ((self.rate / v.clamp(30.0, 600.0)) as usize).clamp(16, self.n);
            }
            (_, 1) | (Mode::Whisper, 0) => self.mix = v.clamp(0.0, 1.0),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::analysis::dominant_freq;

    #[test]
    fn robot_buzzes_at_its_pitch() {
        let ctx = BuildCtx::test();
        let mut r = Spectral::robot(&[150.0, 1.0], &ctx);
        let mut rng = Rng::new(3);
        // Noise in, so any pitch in the output comes from the robot.
        let mut x: Vec<f32> = (0..48000).map(|_| rng.bipolar() * 0.3).collect();
        let mut b = crate::dsp::biquad::Biquad::new(crate::dsp::biquad::FilterType::Lowpass, 400.0, 0.7, 0.0, 48000.0);
        r.process(&mut x, &Ctx::default());
        let x: Vec<f32> = x.into_iter().map(|v| b.process(v)).collect();
        let f = dominant_freq(&x[8192..], 48000.0);
        assert!((f - 150.0).abs() < 6.0, "got {f}");
    }
}
