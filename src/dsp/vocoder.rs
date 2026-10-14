//! Channel vocoder: the voice shapes a synth chord, a single note, or a note that
//! follows the voice (talk box).

use super::biquad::{Biquad, FilterType};
use super::dynamics::Agc;
use super::util::{Envelope, Rng, poly_blep};
use super::yin::Yin;
use super::{BuildCtx, Ctx, Effect};

pub const CARRIERS: &[&str] = &["Chord", "Single note", "Follow voice", "Noise"];
pub const CHORDS: &[&str] = &["Major", "Minor", "Power", "Major 7", "Minor 7", "Sus 4"];
pub const PROGRESSIONS: &[&str] = &["Off", "Pop", "Sad", "Epic"];

fn chord_notes(chord: usize) -> &'static [i32] {
    match chord {
        1 => &[0, 3, 7, 12, -12],
        2 => &[0, 7, 12, -12],
        3 => &[0, 4, 7, 11, -12],
        4 => &[0, 3, 7, 10, -12],
        5 => &[0, 5, 7, 12, -12],
        _ => &[0, 4, 7, 12, -12],
    }
}

/// Root offset and chord type for each step.
fn progression(p: usize) -> &'static [(i32, usize)] {
    match p {
        1 => &[(0, 0), (7, 0), (9, 1), (5, 0)],
        2 => &[(9, 1), (5, 0), (0, 0), (7, 0)],
        3 => &[(0, 1), (8, 0), (3, 0), (10, 0)],
        _ => &[(0, usize::MAX)],
    }
}

struct Band {
    m: [Biquad; 2],
    c: [Biquad; 2],
    env: Envelope,
}

struct Saw {
    phase: f32,
}

pub struct Vocoder {
    rate: f32,
    bands: Vec<Band>,
    saws: Vec<Saw>,
    notes: Vec<f32>,
    rng: Rng,
    yin: Yin,
    follow_freq: f32,
    sib: Biquad,
    agc: Agc,
    carrier: usize,
    root: f32,
    chord: usize,
    prog: usize,
    step: usize,
    step_len: usize,
    step_pos: usize,
    voice_mix: f32,
    mix: f32,
}

impl Vocoder {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let rate = ctx.rate;
        let count = 22;
        let (lo, hi) = (130.0f32, 7500.0f32.min(rate * 0.45));
        let r = (hi / lo).powf(1.0 / (count - 1) as f32);
        let bw = r.log2();
        let q = (2f32.powf(bw)).sqrt() / (2f32.powf(bw) - 1.0) * 1.4;
        let bands = (0..count)
            .map(|i| {
                let f = lo * r.powi(i);
                let bp = || Biquad::new(FilterType::Bandpass, f, q, 0.0, rate);
                Band { m: [bp(), bp()], c: [bp(), bp()], env: Envelope::new(3.0, 25.0, rate) }
            })
            .collect();
        let mut v = Self {
            rate,
            bands,
            saws: (0..6).map(|i| Saw { phase: i as f32 * 0.17 }).collect(),
            notes: Vec::with_capacity(6),
            rng: Rng::new(99),
            yin: Yin::new(rate),
            follow_freq: 110.0,
            sib: Biquad::new(FilterType::Highpass, 5000.0, 0.7, 0.0, rate),
            agc: Agc::new(rate),
            carrier: 0,
            root: 48.0,
            chord: 0,
            prog: 0,
            step: 0,
            step_len: (rate * 2.0) as usize,
            step_pos: 0,
            voice_mix: 0.15,
            mix: 1.0,
        };
        for (i, x) in p.iter().enumerate() {
            v.set_param(i, *x);
        }
        v.retune();
        v
    }

    fn retune(&mut self) {
        self.notes.clear();
        let steps = progression(self.prog);
        let (off, ch) = steps[self.step % steps.len()];
        let ch = if ch == usize::MAX { self.chord } else { ch };
        match self.carrier {
            0 => {
                for n in chord_notes(ch) {
                    let midi = self.root + off as f32 + *n as f32;
                    self.notes.push(440.0 * 2f32.powf((midi - 69.0) / 12.0));
                }
            }
            1 => self.notes.push(440.0 * 2f32.powf((self.root - 69.0) / 12.0)),
            _ => {}
        }
    }

    #[inline]
    fn carrier_sample(&mut self, talking: bool) -> f32 {
        let noise = self.rng.bipolar();
        match self.carrier {
            2 => {
                if talking && self.yin.freq > 0.0 {
                    let f = self.yin.freq;
                    self.follow_freq += (f - self.follow_freq) * 0.05;
                }
                let dt = self.follow_freq / self.rate;
                let s = &mut self.saws[0];
                let v = 2.0 * s.phase - 1.0 - poly_blep(s.phase, dt);
                s.phase = (s.phase + dt).fract();
                v + noise * 0.08
            }
            3 => noise,
            _ => {
                let mut sum = 0.0;
                for (i, f) in self.notes.iter().enumerate() {
                    let dt = f / self.rate;
                    let s = &mut self.saws[i];
                    sum += 2.0 * s.phase - 1.0 - poly_blep(s.phase, dt);
                    s.phase = (s.phase + dt).fract();
                }
                sum / (self.notes.len().max(1) as f32).sqrt() + noise * 0.06
            }
        }
    }
}

impl Effect for Vocoder {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            let input = *x;
            if self.carrier == 2 {
                self.yin.push(input);
            }
            if self.prog > 0 {
                self.step_pos += 1;
                if self.step_pos >= self.step_len {
                    self.step_pos = 0;
                    self.step += 1;
                    self.retune();
                }
            }
            let car = self.carrier_sample(true);
            let mut out = 0.0;
            for b in &mut self.bands {
                let m = b.m[0].process(input);
                let m = b.m[1].process(m);
                let e = b.env.next(m);
                let c = b.c[0].process(car);
                let c = b.c[1].process(c);
                out += c * e;
            }
            let out = self.agc.next(input, out) + self.sib.process(input) * self.voice_mix;
            *x = out * self.mix + input * (1.0 - self.mix);
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.carrier = (v as usize).min(CARRIERS.len() - 1),
            1 => self.root = v,
            2 => self.chord = (v as usize).min(CHORDS.len() - 1),
            3 => self.prog = (v as usize).min(PROGRESSIONS.len() - 1),
            4 => self.step_len = (self.rate * v.max(0.25)) as usize,
            5 => self.voice_mix = v,
            6 => self.mix = v.clamp(0.0, 1.0),
            _ => {}
        }
        self.retune();
    }
}
