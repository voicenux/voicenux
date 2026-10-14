//! Echo and reverb.

use super::biquad::OnePole;
use super::util::{DelayLine, Smoothed};
use super::{BuildCtx, Ctx, Effect};

pub struct Echo {
    rate: f32,
    line: DelayLine,
    time: Smoothed,
    feedback: f32,
    mix: f32,
    lp: OnePole,
    hp: OnePole,
}

impl Echo {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut e = Self {
            rate: ctx.rate,
            line: DelayLine::new((ctx.rate * 2.1) as usize),
            time: Smoothed::new(ctx.rate * 0.3, 120.0, ctx.rate),
            feedback: 0.4,
            mix: 0.4,
            lp: OnePole::new(5000.0, ctx.rate),
            hp: OnePole::new(150.0, ctx.rate),
        };
        for (i, v) in p.iter().enumerate() {
            e.set_param(i, *v);
        }
        e.time.jump(e.time.target);
        e
    }
}

impl Effect for Echo {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            let d = self.time.next();
            let y = self.line.read(d);
            let fb = self.hp.highpass(self.lp.lowpass(y)) * self.feedback;
            self.line.push(*x + fb);
            *x += y * self.mix;
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.time.set((v * 0.001 * self.rate).clamp(2.0, self.line.len() as f32 - 8.0)),
            1 => self.feedback = v.clamp(0.0, 0.95),
            2 => self.mix = v.clamp(0.0, 1.0),
            3 => self.lp.set(v, self.rate),
            _ => {}
        }
    }
}

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
}

impl Comb {
    #[inline]
    fn next(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.idx] = x + self.store * feedback;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    #[inline]
    fn next(&mut self, x: f32) -> f32 {
        let b = self.buf[self.idx];
        self.buf[self.idx] = x + b * 0.5;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        b - x
    }
}

/// Freeverb (Jezar at Dreampoint, public domain), mono, with pre delay.
pub struct Reverb {
    rate: f32,
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
    pre: DelayLine,
    pre_len: usize,
    feedback: f32,
    damp: f32,
    wet: f32,
    dry: f32,
    hp: OnePole,
}

impl Reverb {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let scale = ctx.rate / 44100.0;
        let combs = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617]
            .iter()
            .map(|&l| Comb { buf: vec![0.0; ((l as f32 * scale) as usize).max(1)], idx: 0, store: 0.0 })
            .collect();
        let allpasses = [556, 441, 341, 225]
            .iter()
            .map(|&l| Allpass { buf: vec![0.0; ((l as f32 * scale) as usize).max(1)], idx: 0 })
            .collect();
        let mut r = Self {
            rate: ctx.rate,
            combs,
            allpasses,
            pre: DelayLine::new((ctx.rate * 0.25) as usize),
            pre_len: 0,
            feedback: 0.84,
            damp: 0.2,
            wet: 0.5,
            dry: 0.8,
            hp: OnePole::new(120.0, ctx.rate),
        };
        for (i, v) in p.iter().enumerate() {
            r.set_param(i, *v);
        }
        r
    }
}

impl Effect for Reverb {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            self.pre.push(*x);
            let input = self.hp.highpass(self.pre.tap(self.pre_len)) * 0.015;
            let mut acc = 0.0;
            for c in &mut self.combs {
                acc += c.next(input, self.feedback, self.damp);
            }
            for a in &mut self.allpasses {
                acc = a.next(acc);
            }
            *x = *x * self.dry + acc * self.wet * 3.0;
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.feedback = v.clamp(0.0, 1.0) * 0.28 + 0.7,
            1 => self.damp = v.clamp(0.0, 1.0) * 0.4,
            2 => {
                let m = v.clamp(0.0, 1.0);
                self.wet = (m * std::f32::consts::FRAC_PI_2).sin();
                self.dry = (m * std::f32::consts::FRAC_PI_2).cos().max(0.25);
            }
            3 => self.pre_len = ((v * 0.001 * self.rate) as usize).min(self.pre.len() - 2),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_repeats_after_delay() {
        let ctx = BuildCtx::test();
        let mut e = Echo::new(&[100.0, 0.5, 1.0, 16000.0], &ctx);
        let mut x = vec![0.0; 24000];
        x[0] = 1.0;
        e.process(&mut x, &Ctx::default());
        let at = 4800;
        let peak_idx = (4000..6000).max_by(|a, b| x[*a].abs().total_cmp(&x[*b].abs())).unwrap();
        assert!((peak_idx as i32 - at).abs() < 4, "{peak_idx}");
    }

    #[test]
    fn reverb_has_a_tail() {
        let ctx = BuildCtx::test();
        let mut r = Reverb::new(&[0.8, 0.3, 0.5, 0.0], &ctx);
        let mut x = vec![0.0; 48000];
        x[0] = 1.0;
        r.process(&mut x, &Ctx::default());
        let tail: f32 = x[12000..24000].iter().map(|v| v * v).sum();
        assert!(tail > 1e-6);
        assert!(x.iter().all(|v| v.is_finite() && v.abs() < 2.0));
    }
}
