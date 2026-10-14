//! Noise layered on the voice (radio static, vinyl crackle, mains hum), radio beeps when
//! talking starts or stops, and a stutter glitch.

use super::biquad::{Biquad, FilterType, OnePole};
use super::dynamics::Vad;
use super::util::{Brown, DelayLine, Phasor, Pink, Rng, db_to_gain, sine, smooth_coef};
use super::{BuildCtx, Ctx, Effect};

pub const NOISE_TYPES: &[&str] = &["White", "Pink", "Brown", "Hiss", "Crackle", "Hum"];
pub const BEEP_TYPES: &[&str] = &["Space (Quindar)", "Roger beep", "Squelch", "Walkie-talkie", "Sci-fi"];

pub struct Noise {
    rate: f32,
    kind: usize,
    level: f32,
    follow: f32,
    rng: Rng,
    pink: Pink,
    brown: Brown,
    hp: OnePole,
    crackle_lp: OnePole,
    hum: Phasor,
    vad: Vad,
    talk: f32,
    talk_coef: f32,
}

impl Noise {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut n = Self {
            rate: ctx.rate,
            kind: 0,
            level: 0.03,
            follow: 0.0,
            rng: Rng::new(1234),
            pink: Pink::default(),
            brown: Brown::default(),
            hp: OnePole::new(3000.0, ctx.rate),
            crackle_lp: OnePole::new(4000.0, ctx.rate),
            hum: Phasor::default(),
            vad: Vad::new(ctx.rate),
            talk: 0.0,
            talk_coef: smooth_coef(30.0, ctx.rate),
        };
        for (i, v) in p.iter().enumerate() {
            n.set_param(i, *v);
        }
        n
    }

    #[inline]
    fn sample(&mut self) -> f32 {
        let w = self.rng.bipolar();
        match self.kind {
            1 => self.pink.next(w),
            2 => self.brown.next(w),
            3 => self.hp.highpass(w) * 0.8,
            4 => {
                let bed = self.pink.next(w) * 0.15;
                let pop = if self.rng.chance(12.0 / self.rate) { self.rng.bipolar() * 6.0 } else { 0.0 };
                self.crackle_lp.lowpass(bed + pop)
            }
            5 => {
                let p = self.hum.next(50.0, self.rate);
                (sine(p) + 0.5 * sine(p * 2.0) + 0.3 * sine(p * 3.0)) * 0.4
            }
            _ => w * 0.6,
        }
    }
}

impl Effect for Noise {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            self.vad.next(*x);
            let t = if self.vad.active { 1.0 } else { 0.0 };
            self.talk = t + (self.talk - t) * self.talk_coef;
            let g = self.level * ((1.0 - self.follow) + self.follow * self.talk);
            *x += self.sample() * g;
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.kind = (v as usize).min(NOISE_TYPES.len() - 1),
            1 => self.level = db_to_gain(v),
            2 => self.follow = v.clamp(0.0, 1.0),
            _ => {}
        }
    }
}

/// Beeps and squelch bursts when talking starts or stops.
pub struct Beeps {
    rate: f32,
    vad: Vad,
    kind: usize,
    level: f32,
    on_start: bool,
    on_end: bool,
    /// Which sound plays (true = the end sound) and how far in it is.
    playing: Option<(bool, usize)>,
    osc: Phasor,
    rng: Rng,
    bp: Biquad,
}

impl Beeps {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut b = Self {
            rate: ctx.rate,
            vad: Vad::new(ctx.rate),
            kind: 0,
            level: 0.25,
            on_start: true,
            on_end: true,
            playing: None,
            osc: Phasor::default(),
            rng: Rng::new(4321),
            bp: Biquad::new(FilterType::Bandpass, 2200.0, 0.9, 0.0, ctx.rate),
        };
        for (i, v) in p.iter().enumerate() {
            b.set_param(i, *v);
        }
        b
    }

    /// One sample of the current sound, or None when it is over.
    fn sound(&mut self, end: bool, i: usize) -> Option<f32> {
        let t = i as f32 / self.rate;
        let fade = |len: f32| -> f32 { (t / 0.005).min(1.0).min(((len - t) / 0.005).max(0.0)) };
        let tone = |s: &mut Self, f: f32| sine(s.osc.next(f, s.rate));
        let noise = |s: &mut Self| {
            let w = s.rng.bipolar();
            s.bp.process(w) * 2.5
        };
        match (self.kind, end) {
            (0, _) => {
                let len = 0.25;
                (t < len).then(|| tone(self, if end { 2475.0 } else { 2525.0 }) * fade(len) * 0.6)
            }
            (1, false) => (t < 0.06).then(|| tone(self, 1600.0) * fade(0.06) * 0.5),
            (1, true) => {
                let len = 0.18;
                (t < len).then(|| {
                    let f = if t < 0.09 { 1200.0 } else { 1650.0 };
                    tone(self, f) * fade(len) * 0.5
                })
            }
            (2, false) => (t < 0.05).then(|| noise(self) * fade(0.05) * 0.5),
            (2, true) | (3, true) => {
                let len = if self.kind == 3 { 0.32 } else { 0.2 };
                (t < len).then(|| noise(self) * (1.0 - t / len).powf(0.6) * fade(len))
            }
            (3, false) => (t < 0.07).then(|| tone(self, 1850.0) * fade(0.07) * 0.45),
            (_, end) => {
                let len = 0.14;
                (t < len).then(|| {
                    let k = t / len;
                    let f = if end { 1800.0 - 1200.0 * k } else { 600.0 + 1200.0 * k };
                    tone(self, f) * fade(len) * 0.45
                })
            }
        }
    }
}

impl Effect for Beeps {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            match self.vad.next(*x) {
                Some(true) if self.on_start => self.playing = Some((false, 0)),
                Some(false) if self.on_end => self.playing = Some((true, 0)),
                _ => {}
            }
            if let Some((end, i)) = self.playing {
                match self.sound(end, i) {
                    Some(s) => {
                        *x += s * self.level;
                        self.playing = Some((end, i + 1));
                    }
                    None => self.playing = None,
                }
            }
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.kind = (v as usize).min(BEEP_TYPES.len() - 1),
            1 => self.level = db_to_gain(v),
            2 => self.on_start = v >= 0.5,
            3 => self.on_end = v >= 0.5,
            _ => {}
        }
    }
}

/// Repeats short bits of the voice at random, like a bad connection or a broken robot.
pub struct Stutter {
    rate: f32,
    line: DelayLine,
    rng: Rng,
    chance: f32,
    len_ms: f32,
    repeats: usize,
    mix: f32,
    /// Segment length, samples played since the repeat began, total samples to play.
    active: Option<(usize, usize, usize)>,
}

impl Stutter {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut s = Self {
            rate: ctx.rate,
            line: DelayLine::new((ctx.rate * 1.2) as usize),
            rng: Rng::new(777),
            chance: 0.8,
            len_ms: 80.0,
            repeats: 3,
            mix: 1.0,
            active: None,
        };
        for (i, v) in p.iter().enumerate() {
            s.set_param(i, *v);
        }
        s
    }
}

impl Effect for Stutter {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        let fade = (self.rate * 0.003) as usize;
        for x in buf.iter_mut() {
            self.line.push(*x);
            if self.active.is_none() && self.rng.chance(self.chance / self.rate) {
                let len = (self.len_ms * self.rng.range(0.6, 1.4) * 0.001 * self.rate) as usize;
                let len = len.clamp(fade * 3, self.line.len() / (self.repeats + 2));
                let reps = 1 + (self.rng.next_u32() as usize % self.repeats.max(1));
                self.active = Some((len, 0, len * reps));
            }
            if let Some((len, pos, total)) = self.active {
                let within = pos % len;
                let delay = pos + len - within;
                let env = (within.min(len - within) as f32 / fade as f32).min(1.0);
                let rep = self.line.tap(delay) * env;
                *x = rep * self.mix + *x * (1.0 - self.mix);
                self.active = if pos + 1 >= total { None } else { Some((len, pos + 1, total)) };
            }
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.chance = v.max(0.0),
            1 => self.len_ms = v.clamp(10.0, 300.0),
            2 => self.repeats = (v as usize).clamp(1, 8),
            3 => self.mix = v.clamp(0.0, 1.0),
            _ => {}
        }
    }
}
