//! RBJ cookbook biquads.

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterType {
    Lowpass,
    Highpass,
    Bandpass,
    Peak,
    LowShelf,
    HighShelf,
    Notch,
}

impl FilterType {
    pub const ALL: [FilterType; 7] = [
        FilterType::Lowpass,
        FilterType::Highpass,
        FilterType::Bandpass,
        FilterType::Peak,
        FilterType::LowShelf,
        FilterType::HighShelf,
        FilterType::Notch,
    ];

    pub fn from_index(i: usize) -> Self {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    pub fn new(kind: FilterType, freq: f32, q: f32, gain_db: f32, rate: f32) -> Self {
        let mut f = Self::default();
        f.set(kind, freq, q, gain_db, rate);
        f
    }

    pub fn set(&mut self, kind: FilterType, freq: f32, q: f32, gain_db: f32, rate: f32) {
        let freq = freq.clamp(10.0, rate * 0.49);
        let q = q.max(0.05);
        let w0 = 2.0 * PI * freq / rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a = 10f32.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterType::Lowpass => {
                let b1 = 1.0 - cos;
                (b1 / 2.0, b1, b1 / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            FilterType::Highpass => {
                let b1 = -(1.0 + cos);
                (-b1 / 2.0, b1, -b1 / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            FilterType::Bandpass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterType::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterType::Peak => {
                (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a)
            }
            FilterType::LowShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - s),
                    (a + 1.0) + (a - 1.0) * cos + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - s,
                )
            }
            FilterType::HighShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - s),
                    (a + 1.0) - (a - 1.0) * cos + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - s,
                )
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Fourth order Butterworth low or high pass: two biquads in a row.
#[derive(Clone, Copy, Debug, Default)]
pub struct Butter4 {
    a: Biquad,
    b: Biquad,
}

impl Butter4 {
    pub fn new(kind: FilterType, freq: f32, rate: f32) -> Self {
        Self { a: Biquad::new(kind, freq, 0.541_196, 0.0, rate), b: Biquad::new(kind, freq, 1.306_563, 0.0, rate) }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.b.process(self.a.process(x))
    }
}

/// One-pole low pass, cheap tone control.
#[derive(Clone, Copy, Debug, Default)]
pub struct OnePole {
    a: f32,
    z: f32,
}

impl OnePole {
    pub fn new(freq: f32, rate: f32) -> Self {
        let mut f = Self::default();
        f.set(freq, rate);
        f
    }

    pub fn set(&mut self, freq: f32, rate: f32) {
        self.a = (-2.0 * PI * freq.clamp(5.0, rate * 0.49) / rate).exp();
    }

    #[inline]
    pub fn lowpass(&mut self, x: f32) -> f32 {
        self.z = x + (self.z - x) * self.a;
        self.z
    }

    #[inline]
    pub fn highpass(&mut self, x: f32) -> f32 {
        x - self.lowpass(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain_at(f: &mut Biquad, freq: f32) -> f32 {
        let rate = 48000.0;
        let mut peak = 0.0f32;
        for i in 0..48000 {
            let y = f.process((2.0 * PI * freq * i as f32 / rate).sin());
            if i > 24000 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn lowpass_cuts_highs() {
        let mut f = Biquad::new(FilterType::Lowpass, 1000.0, 0.707, 0.0, 48000.0);
        assert!(gain_at(&mut f, 100.0) > 0.95);
        f = Biquad::new(FilterType::Lowpass, 1000.0, 0.707, 0.0, 48000.0);
        assert!(gain_at(&mut f, 10000.0) < 0.02);
    }

    #[test]
    fn peak_boosts() {
        let mut f = Biquad::new(FilterType::Peak, 1000.0, 1.0, 12.0, 48000.0);
        assert!((gain_at(&mut f, 1000.0) - 3.98).abs() < 0.1);
    }
}
