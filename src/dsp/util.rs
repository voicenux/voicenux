//! Small building blocks shared by the effects.

use std::f32::consts::PI;

pub const TAU: f32 = 2.0 * PI;

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.max(1e-9).log10()
}

pub fn semitones_to_ratio(st: f32) -> f32 {
    2f32.powf(st / 12.0)
}

/// Coefficient for a one-pole smoother that settles in roughly `ms` milliseconds.
pub fn smooth_coef(ms: f32, rate: f32) -> f32 {
    if ms <= 0.0 {
        return 0.0;
    }
    (-1.0 / (ms * 0.001 * rate)).exp()
}

/// A value that glides to its target, so parameter changes do not click.
#[derive(Clone, Copy, Debug)]
pub struct Smoothed {
    pub value: f32,
    pub target: f32,
    coef: f32,
}

impl Smoothed {
    pub fn new(value: f32, ms: f32, rate: f32) -> Self {
        Self { value, target: value, coef: smooth_coef(ms, rate) }
    }

    pub fn set(&mut self, target: f32) {
        self.target = target;
    }

    pub fn jump(&mut self, value: f32) {
        self.value = value;
        self.target = value;
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        self.value = self.target + (self.value - self.target) * self.coef;
        self.value
    }
}

/// Fast, deterministic noise. Good enough for audio, not for anything else.
#[derive(Clone, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in [0, 1).
    #[inline]
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in [-1, 1).
    #[inline]
    pub fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
}

/// Pink noise after Paul Kellet's economy filter.
#[derive(Clone, Debug, Default)]
pub struct Pink {
    b: [f32; 3],
}

impl Pink {
    #[inline]
    pub fn next(&mut self, white: f32) -> f32 {
        self.b[0] = 0.99765 * self.b[0] + white * 0.0990460;
        self.b[1] = 0.96300 * self.b[1] + white * 0.2965164;
        self.b[2] = 0.57000 * self.b[2] + white * 1.0526913;
        (self.b[0] + self.b[1] + self.b[2] + white * 0.1848) * 0.25
    }
}

/// Brown (red) noise: leaky integrated white noise.
#[derive(Clone, Debug, Default)]
pub struct Brown {
    v: f32,
}

impl Brown {
    #[inline]
    pub fn next(&mut self, white: f32) -> f32 {
        self.v = (self.v + white * 0.02) * 0.998;
        self.v * 3.5
    }
}

/// Peak follower with separate attack and release.
#[derive(Clone, Debug)]
pub struct Envelope {
    pub value: f32,
    att: f32,
    rel: f32,
}

impl Envelope {
    pub fn new(attack_ms: f32, release_ms: f32, rate: f32) -> Self {
        Self { value: 0.0, att: smooth_coef(attack_ms, rate), rel: smooth_coef(release_ms, rate) }
    }

    #[inline]
    pub fn next(&mut self, x: f32) -> f32 {
        let x = x.abs();
        let c = if x > self.value { self.att } else { self.rel };
        self.value = x + (self.value - x) * c;
        self.value
    }
}

/// Sine oscillator with phase in [0, 1).
#[derive(Clone, Debug, Default)]
pub struct Phasor {
    pub phase: f32,
}

impl Phasor {
    #[inline]
    pub fn next(&mut self, freq: f32, rate: f32) -> f32 {
        let p = self.phase;
        self.phase += freq / rate;
        self.phase -= self.phase.floor();
        p
    }
}

#[inline]
pub fn sine(phase: f32) -> f32 {
    (phase * TAU).sin()
}

/// Band-limited sawtooth step (polyBLEP).
#[inline]
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// A delay line with fractional (cubic) reads.
#[derive(Clone, Debug)]
pub struct DelayLine {
    buf: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    pub fn new(max_samples: usize) -> Self {
        Self { buf: vec![0.0; max_samples.max(4).next_power_of_two()], pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    pub fn push(&mut self, x: f32) {
        self.pos = (self.pos + 1) & (self.buf.len() - 1);
        self.buf[self.pos] = x;
    }

    /// Sample written `delay` samples ago (0 = the newest one).
    #[inline]
    pub fn tap(&self, delay: usize) -> f32 {
        let mask = self.buf.len() - 1;
        self.buf[(self.pos.wrapping_sub(delay)) & mask]
    }

    /// Hermite interpolated read, `delay` in samples (>= 1 for best results).
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let delay = delay.clamp(1.0, (self.buf.len() - 3) as f32);
        let i = delay.floor() as usize;
        let f = delay - i as f32;
        let xm1 = self.tap(i - 1);
        let x0 = self.tap(i);
        let x1 = self.tap(i + 1);
        let x2 = self.tap(i + 2);
        let c = (x1 - xm1) * 0.5;
        let v = x0 - x1;
        let w = c + v;
        let a = w + v + (x2 - x0) * 0.5;
        let b = w + a;
        ((a * f - b) * f + c) * f + x0
    }
}

/// Root mean square of a block.
pub fn rms(buf: &[f32]) -> f32 {
    if buf.is_empty() {
        return 0.0;
    }
    (buf.iter().map(|x| x * x).sum::<f32>() / buf.len() as f32).sqrt()
}

pub fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, x| m.max(x.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_reads_back() {
        let mut d = DelayLine::new(64);
        for i in 0..64 {
            d.push(i as f32);
        }
        assert_eq!(d.tap(0), 63.0);
        assert_eq!(d.tap(10), 53.0);
        assert!((d.read(10.5) - 52.5).abs() < 1e-3);
    }

    #[test]
    fn rng_is_uniform_enough() {
        let mut r = Rng::new(7);
        let mean = (0..10000).map(|_| r.unit()).sum::<f32>() / 10000.0;
        assert!((mean - 0.5).abs() < 0.02);
    }
}
