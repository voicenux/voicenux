//! Effects driven by an LFO: chorus, flanger, phaser, vibrato, tremolo and ring modulation.

use super::biquad::OnePole;
use super::util::{DelayLine, Phasor, Rng, TAU, sine};
use super::{BuildCtx, Ctx, Effect};

fn apply(e: &mut dyn Effect, p: &[f32]) {
    for (i, v) in p.iter().enumerate() {
        e.set_param(i, *v);
    }
}

/// Chorus (three voices) and flanger (one voice with feedback) share this code.
pub struct ModDelay {
    rate: f32,
    line: DelayLine,
    lfo: Phasor,
    speed: f32,
    depth: f32,
    base: f32,
    feedback: f32,
    mix: f32,
    voices: usize,
    last: f32,
}

impl ModDelay {
    pub fn chorus(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut m = Self::base(ctx, 3, 0.8, 3.0, 14.0);
        apply(&mut m, p);
        m
    }

    pub fn flanger(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut m = Self::base(ctx, 1, 0.25, 2.0, 1.5);
        apply(&mut m, p);
        m
    }

    fn base(ctx: &BuildCtx, voices: usize, speed: f32, depth: f32, base: f32) -> Self {
        Self {
            rate: ctx.rate,
            line: DelayLine::new((ctx.rate * 0.08) as usize),
            lfo: Phasor::default(),
            speed,
            depth,
            base,
            feedback: 0.0,
            mix: 0.5,
            voices,
            last: 0.0,
        }
    }
}

impl Effect for ModDelay {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        let ms = self.rate * 0.001;
        for x in buf.iter_mut() {
            let p = self.lfo.next(self.speed, self.rate);
            self.line.push(*x + self.last * self.feedback);
            let mut wet = 0.0;
            for v in 0..self.voices {
                let ph = p + v as f32 / self.voices as f32;
                let d = (self.base + self.depth * (0.5 + 0.5 * sine(ph))) * ms;
                wet += self.line.read(d.max(1.0));
            }
            wet /= self.voices as f32;
            self.last = wet;
            *x = *x * (1.0 - self.mix * 0.5) + wet * self.mix;
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.speed = v,
            1 => self.depth = v,
            2 => {
                if self.voices == 1 {
                    self.feedback = v.clamp(-0.95, 0.95)
                } else {
                    self.base = v
                }
            }
            3 => self.mix = v.clamp(0.0, 1.0),
            _ => {}
        }
    }
}

/// Pitch wobble from a modulated delay, with optional random drift.
pub struct Vibrato {
    rate: f32,
    line: DelayLine,
    lfo: Phasor,
    speed: f32,
    cents: f32,
    random: f32,
    rng: Rng,
    drift: OnePole,
    drift2: OnePole,
}

impl Vibrato {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut v = Self {
            rate: ctx.rate,
            line: DelayLine::new((ctx.rate * 0.1) as usize),
            lfo: Phasor::default(),
            speed: 5.0,
            cents: 30.0,
            random: 0.0,
            rng: Rng::new(17),
            drift: OnePole::new(3.0, ctx.rate),
            drift2: OnePole::new(3.0, ctx.rate),
        };
        apply(&mut v, p);
        v
    }
}

impl Effect for Vibrato {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        // Peak delay swing that gives the wanted pitch swing at this LFO speed.
        let ratio = 2f32.powf(self.cents / 1200.0) - 1.0;
        let amp = (ratio / (TAU * self.speed.max(0.05)) * self.rate).min(self.rate * 0.03);
        let center = amp + 3.0;
        for x in buf.iter_mut() {
            let p = self.lfo.next(self.speed, self.rate);
            let n = self.drift2.lowpass(self.drift.lowpass(self.rng.bipolar())) * 60.0;
            let wobble = sine(p) * (1.0 - self.random) + n.clamp(-1.0, 1.0) * self.random;
            self.line.push(*x);
            *x = self.line.read(center + amp * wobble);
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.speed = v,
            1 => self.cents = v,
            2 => self.random = v.clamp(0.0, 1.0),
            _ => {}
        }
    }
}

pub struct Tremolo {
    rate: f32,
    lfo: Phasor,
    speed: f32,
    depth: f32,
    square: bool,
}

impl Tremolo {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut t = Self { rate: ctx.rate, lfo: Phasor::default(), speed: 6.0, depth: 0.5, square: false };
        apply(&mut t, p);
        t
    }
}

impl Effect for Tremolo {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            let s = sine(self.lfo.next(self.speed, self.rate));
            let s = if self.square { (s * 6.0).tanh() } else { s };
            *x *= 1.0 - self.depth * (0.5 + 0.5 * s);
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.speed = v,
            1 => self.depth = v.clamp(0.0, 1.0),
            2 => self.square = v >= 0.5,
            _ => {}
        }
    }
}

pub struct RingMod {
    rate: f32,
    osc: Phasor,
    lfo: Phasor,
    freq: f32,
    wobble: f32,
    mix: f32,
}

impl RingMod {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut r =
            Self { rate: ctx.rate, osc: Phasor::default(), lfo: Phasor::default(), freq: 60.0, wobble: 0.0, mix: 1.0 };
        apply(&mut r, p);
        r
    }
}

impl Effect for RingMod {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            let w = sine(self.lfo.next(0.7, self.rate)) * self.wobble;
            let c = sine(self.osc.next(self.freq * (1.0 + w), self.rate));
            *x = *x * (1.0 - self.mix) + *x * c * self.mix;
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.freq = v,
            1 => self.wobble = v.clamp(0.0, 1.0),
            2 => self.mix = v.clamp(0.0, 1.0),
            _ => {}
        }
    }
}

pub struct Phaser {
    rate: f32,
    lfo: Phasor,
    speed: f32,
    depth: f32,
    feedback: f32,
    mix: f32,
    z: [f32; 8],
    last: f32,
}

impl Phaser {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut ph = Self {
            rate: ctx.rate,
            lfo: Phasor::default(),
            speed: 0.4,
            depth: 0.8,
            feedback: 0.5,
            mix: 0.5,
            z: [0.0; 8],
            last: 0.0,
        };
        apply(&mut ph, p);
        ph
    }
}

impl Effect for Phaser {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            let s = 0.5 + 0.5 * sine(self.lfo.next(self.speed, self.rate));
            let f = 250.0 * 2f32.powf(s * self.depth * 4.0);
            let t = (std::f32::consts::PI * f / self.rate).tan();
            let a = (t - 1.0) / (t + 1.0);
            let mut y = *x + self.last * self.feedback;
            for z in &mut self.z {
                let out = a * y + *z;
                *z = y - a * out;
                y = out;
            }
            self.last = y;
            *x = *x * (1.0 - self.mix * 0.5) + y * self.mix;
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.speed = v,
            1 => self.depth = v.clamp(0.0, 1.0),
            2 => self.feedback = v.clamp(-0.9, 0.9),
            3 => self.mix = v.clamp(0.0, 1.0),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::analysis::dominant_freq;

    #[test]
    fn ring_mod_splits_tone() {
        let ctx = BuildCtx::test();
        let mut r = RingMod::new(&[200.0, 0.0, 1.0], &ctx);
        let mut x: Vec<f32> = (0..48000).map(|i| (TAU * 1000.0 * i as f32 / 48000.0).sin()).collect();
        r.process(&mut x, &Ctx::default());
        let f = dominant_freq(&x, 48000.0);
        assert!((f - 800.0).abs() < 5.0 || (f - 1200.0).abs() < 5.0, "{f}");
    }

    #[test]
    fn vibrato_stays_finite() {
        let ctx = BuildCtx::test();
        let mut v = Vibrato::new(&[0.5, 120.0, 0.5], &ctx);
        let mut x: Vec<f32> = (0..96000).map(|i| (i as f32 * 0.03).sin()).collect();
        v.process(&mut x, &Ctx::default());
        assert!(x.iter().all(|s| s.is_finite() && s.abs() < 1.5));
    }
}
