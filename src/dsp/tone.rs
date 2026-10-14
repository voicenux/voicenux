//! Tone shaping: filters, band limits, distortion and bit crushing.

use super::biquad::{Biquad, Butter4, FilterType, OnePole};
use super::util::db_to_gain;
use super::{BuildCtx, Ctx, Effect};

pub const FILTER_TYPES: &[&str] = &["Low pass", "High pass", "Band pass", "Peak", "Low shelf", "High shelf", "Notch"];
pub const DRIVE_TYPES: &[&str] = &["Soft", "Hard", "Fuzz", "Fold"];

pub struct Filter {
    rate: f32,
    f: Biquad,
    kind: FilterType,
    freq: f32,
    q: f32,
    gain: f32,
}

impl Filter {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut f =
            Self { rate: ctx.rate, f: Biquad::default(), kind: FilterType::Lowpass, freq: 1000.0, q: 0.707, gain: 0.0 };
        for (i, v) in p.iter().enumerate() {
            f.set_param(i, *v);
        }
        f
    }
}

impl Effect for Filter {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            *x = self.f.process(*x);
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.kind = FilterType::from_index(v as usize),
            1 => self.freq = v,
            2 => self.q = v,
            3 => self.gain = v,
            _ => {}
        }
        self.f.set(self.kind, self.freq, self.q, self.gain, self.rate);
    }
}

/// Steep high and low cut with a mid boost: phones, radios and megaphones.
pub struct Band {
    rate: f32,
    low: f32,
    high: f32,
    boost: f32,
    hp: Butter4,
    lp: Butter4,
    mid: Biquad,
}

impl Band {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut b = Self {
            rate: ctx.rate,
            low: 300.0,
            high: 3400.0,
            boost: 0.0,
            hp: Butter4::default(),
            lp: Butter4::default(),
            mid: Biquad::default(),
        };
        for (i, v) in p.iter().enumerate() {
            b.set_param(i, *v);
        }
        b
    }
}

impl Effect for Band {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            *x = self.mid.process(self.lp.process(self.hp.process(*x)));
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.low = v,
            1 => self.high = v,
            2 => self.boost = v,
            _ => {}
        }
        self.hp = Butter4::new(FilterType::Highpass, self.low, self.rate);
        self.lp = Butter4::new(FilterType::Lowpass, self.high, self.rate);
        let center = (self.low * self.high).sqrt();
        self.mid = Biquad::new(FilterType::Peak, center, 0.8, self.boost, self.rate);
    }
}

pub struct Distortion {
    rate: f32,
    drive: f32,
    comp: f32,
    kind: usize,
    tone: OnePole,
    mix: f32,
    out: f32,
    dc: OnePole,
}

impl Distortion {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut d = Self {
            rate: ctx.rate,
            drive: 1.0,
            comp: 1.0,
            kind: 0,
            tone: OnePole::new(6000.0, ctx.rate),
            mix: 1.0,
            out: 1.0,
            dc: OnePole::new(15.0, ctx.rate),
        };
        for (i, v) in p.iter().enumerate() {
            d.set_param(i, *v);
        }
        d
    }

    #[inline]
    fn shape(&self, x: f32) -> f32 {
        match self.kind {
            1 => x.clamp(-1.0, 1.0),
            2 => {
                if x > 0.0 {
                    (x * 1.5).tanh()
                } else {
                    (x * 0.6).tanh() * 0.8
                }
            }
            3 => (x * 1.2).sin(),
            _ => x.tanh(),
        }
    }
}

impl Effect for Distortion {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            let dry = *x;
            let y = self.shape(dry * self.drive);
            let y = self.dc.highpass(self.tone.lowpass(y)) * self.comp * self.out;
            *x = y * self.mix + dry * (1.0 - self.mix);
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => {
                self.drive = db_to_gain(v);
                // Voices sit around -20 dBFS, so keep the result near that.
                self.comp = (1.0 / (self.drive * 0.25).max(1.0)).sqrt().max(0.12);
            }
            1 => self.kind = (v as usize).min(DRIVE_TYPES.len() - 1),
            2 => self.tone.set(v, self.rate),
            3 => self.mix = v.clamp(0.0, 1.0),
            4 => self.out = db_to_gain(v),
            _ => {}
        }
    }
}

pub struct Bitcrush {
    rate: f32,
    step: f32,
    phase: f32,
    hold: f32,
    levels: f32,
    mix: f32,
}

impl Bitcrush {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut b = Self { rate: ctx.rate, step: 1.0, phase: 0.0, hold: 0.0, levels: 128.0, mix: 1.0 };
        for (i, v) in p.iter().enumerate() {
            b.set_param(i, *v);
        }
        b
    }
}

impl Effect for Bitcrush {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            self.phase += self.step;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
                self.hold = (*x * self.levels).round() / self.levels;
            }
            *x = self.hold * self.mix + *x * (1.0 - self.mix);
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.levels = 2f32.powf(v.clamp(1.0, 16.0) - 1.0),
            1 => self.step = (v / self.rate).clamp(0.001, 1.0),
            2 => self.mix = v.clamp(0.0, 1.0),
            _ => {}
        }
    }
}
