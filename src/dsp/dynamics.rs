//! Level control: compressor, gate, limiter, auto gain and voice detection.

use super::util::{db_to_gain, gain_to_db, smooth_coef};
use super::{BuildCtx, Ctx, Effect};

/// Matches the loudness of a processed signal to the input it came from.
/// Effects like the robot change the level a lot, depending on the voice.
#[derive(Clone, Debug)]
pub struct Agc {
    in_ms: f32,
    out_ms: f32,
    coef: f32,
    gain: f32,
    gain_coef: f32,
}

impl Agc {
    pub fn new(rate: f32) -> Self {
        Self { in_ms: 0.0, out_ms: 0.0, coef: smooth_coef(250.0, rate), gain: 1.0, gain_coef: smooth_coef(50.0, rate) }
    }

    #[inline]
    pub fn next(&mut self, input: f32, out: f32) -> f32 {
        self.in_ms = input * input + (self.in_ms - input * input) * self.coef;
        self.out_ms = out * out + (self.out_ms - out * out) * self.coef;
        // Only follow while there is something to follow, so silence is not boosted.
        if self.in_ms > 1e-7 && self.out_ms > 1e-9 {
            let target = (self.in_ms / self.out_ms).sqrt().clamp(0.05, 8.0);
            self.gain = target + (self.gain - target) * self.gain_coef;
        }
        out * self.gain
    }
}

/// Voice activity: true while someone talks, with a short hang time.
#[derive(Clone, Debug)]
pub struct Vad {
    env: f32,
    att: f32,
    rel: f32,
    open: f32,
    close: f32,
    hang: usize,
    hang_left: usize,
    pub active: bool,
}

impl Vad {
    pub fn new(rate: f32) -> Self {
        Self {
            env: 0.0,
            att: smooth_coef(4.0, rate),
            rel: smooth_coef(60.0, rate),
            open: db_to_gain(-38.0),
            close: db_to_gain(-46.0),
            hang: (rate * 0.35) as usize,
            hang_left: 0,
            active: false,
        }
    }

    /// Returns `Some(true)` when talking starts and `Some(false)` when it stops.
    #[inline]
    pub fn next(&mut self, x: f32) -> Option<bool> {
        let x = x.abs();
        let c = if x > self.env { self.att } else { self.rel };
        self.env = x + (self.env - x) * c;
        if self.active {
            if self.env > self.close {
                self.hang_left = self.hang;
            } else if self.hang_left > 0 {
                self.hang_left -= 1;
            } else {
                self.active = false;
                return Some(false);
            }
        } else if self.env > self.open {
            self.active = true;
            self.hang_left = self.hang;
            return Some(true);
        }
        None
    }
}

pub struct Compressor {
    rate: f32,
    threshold: f32,
    ratio: f32,
    att: f32,
    rel: f32,
    makeup: f32,
    env_db: f32,
}

impl Compressor {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut c =
            Self { rate: ctx.rate, threshold: -20.0, ratio: 4.0, att: 0.0, rel: 0.0, makeup: 1.0, env_db: -120.0 };
        for (i, v) in p.iter().enumerate() {
            c.set_param(i, *v);
        }
        c
    }
}

impl Effect for Compressor {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            let lvl = gain_to_db(x.abs());
            let c = if lvl > self.env_db { self.att } else { self.rel };
            self.env_db = lvl + (self.env_db - lvl) * c;
            let over = self.env_db - self.threshold;
            // Soft knee of 6 dB.
            let reduce = if over <= -3.0 {
                0.0
            } else if over < 3.0 {
                (1.0 - 1.0 / self.ratio) * (over + 3.0).powi(2) / 12.0
            } else {
                (1.0 - 1.0 / self.ratio) * over
            };
            *x *= db_to_gain(-reduce) * self.makeup;
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.threshold = v,
            1 => self.ratio = v.max(1.0),
            2 => self.att = smooth_coef(v, self.rate),
            3 => self.rel = smooth_coef(v, self.rate),
            4 => self.makeup = db_to_gain(v),
            _ => {}
        }
    }
}

pub struct Gate {
    rate: f32,
    threshold: f32,
    env: f32,
    gain: f32,
    att: f32,
    rel: f32,
    env_rel: f32,
}

impl Gate {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let mut g = Self {
            rate: ctx.rate,
            threshold: db_to_gain(-50.0),
            env: 0.0,
            gain: 0.0,
            att: smooth_coef(2.0, ctx.rate),
            rel: smooth_coef(120.0, ctx.rate),
            env_rel: smooth_coef(30.0, ctx.rate),
        };
        for (i, v) in p.iter().enumerate() {
            g.set_param(i, *v);
        }
        g
    }

    #[inline]
    pub fn next(&mut self, x: f32) -> f32 {
        let a = x.abs();
        self.env = if a > self.env { a } else { a + (self.env - a) * self.env_rel };
        let target = if self.env > self.threshold { 1.0 } else { 0.0 };
        let c = if target > self.gain { self.att } else { self.rel };
        self.gain = target + (self.gain - target) * c;
        x * self.gain
    }
}

impl Effect for Gate {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            *x = self.next(*x);
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.threshold = db_to_gain(v),
            1 => self.rel = smooth_coef(v, self.rate),
            _ => {}
        }
    }
}

/// Brick wall at -1 dBFS without lookahead: fast attack, smooth release.
#[derive(Clone, Debug)]
pub struct Limiter {
    gain: f32,
    rel: f32,
    ceiling: f32,
}

impl Limiter {
    pub fn new(rate: f32) -> Self {
        Self { gain: 1.0, rel: smooth_coef(80.0, rate), ceiling: db_to_gain(-1.0) }
    }

    #[inline]
    pub fn next(&mut self, x: f32) -> f32 {
        let a = x.abs();
        let need = if a * self.gain > self.ceiling { self.ceiling / a } else { 1.0 };
        if need < self.gain {
            self.gain = need;
        } else {
            self.gain = need + (self.gain - need) * self.rel;
        }
        // Whatever still pokes out gets clipped softly.
        let y = x * self.gain;
        if y.abs() > self.ceiling { y.signum() * self.ceiling } else { y }
    }
}

pub struct Gain {
    gain: f32,
}

impl Gain {
    pub fn new(p: &[f32], _ctx: &BuildCtx) -> Self {
        Self { gain: db_to_gain(p.first().copied().unwrap_or(0.0)) }
    }
}

impl Effect for Gain {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        for x in buf.iter_mut() {
            *x *= self.gain;
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        if index == 0 {
            self.gain = db_to_gain(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_holds_ceiling() {
        let mut l = Limiter::new(48000.0);
        for i in 0..48000 {
            let y = l.next((i as f32 * 0.05).sin() * 4.0);
            assert!(y.abs() <= 0.892, "{y}");
        }
    }

    #[test]
    fn vad_opens_and_closes() {
        let mut v = Vad::new(48000.0);
        let mut events = vec![];
        for i in 0..96000 {
            let x = if (24000..48000).contains(&i) { (i as f32 * 0.06).sin() * 0.3 } else { 0.0 };
            if let Some(e) = v.next(x) {
                events.push(e);
            }
        }
        assert_eq!(events, vec![true, false]);
    }
}
