//! YIN pitch detection on a decimated copy of the signal.

use super::biquad::{Biquad, FilterType};

pub struct Yin {
    decim: usize,
    phase: usize,
    lp: [Biquad; 2],
    rate: f32,
    buf: Vec<f32>,
    pos: usize,
    filled: usize,
    win: usize,
    tau_min: usize,
    tau_max: usize,
    diff: Vec<f32>,
    frame: Vec<f32>,
    hop: usize,
    since: usize,
    /// Detected pitch in Hz, or 0 when the input has none.
    pub freq: f32,
    threshold: f32,
}

impl Yin {
    pub fn new(rate: f32) -> Self {
        let decim = ((rate / 12000.0).round() as usize).max(1);
        let drate = rate / decim as f32;
        let tau_min = (drate / 900.0) as usize;
        let tau_max = (drate / 65.0) as usize;
        let win = (drate * 0.025) as usize;
        let len = win + tau_max + 2;
        Self {
            decim,
            phase: 0,
            lp: [
                Biquad::new(FilterType::Lowpass, 2500.0, 0.541, 0.0, rate),
                Biquad::new(FilterType::Lowpass, 2500.0, 1.307, 0.0, rate),
            ],
            rate: drate,
            buf: vec![0.0; len],
            pos: 0,
            filled: 0,
            win,
            tau_min: tau_min.max(2),
            tau_max,
            diff: vec![0.0; tau_max + 2],
            frame: vec![0.0; len],
            hop: (drate * 0.008) as usize,
            since: 0,
            freq: 0.0,
            threshold: 0.18,
        }
    }

    /// Feed one sample at the full rate. Returns true when `freq` was updated.
    #[inline]
    pub fn push(&mut self, x: f32) -> bool {
        let y = self.lp[0].process(x);
        let y = self.lp[1].process(y);
        self.phase += 1;
        if self.phase < self.decim {
            return false;
        }
        self.phase = 0;
        let len = self.buf.len();
        self.buf[self.pos] = y;
        self.pos = (self.pos + 1) % len;
        self.filled = (self.filled + 1).min(len);
        self.since += 1;
        if self.since >= self.hop && self.filled == len {
            self.since = 0;
            self.detect();
            return true;
        }
        false
    }

    fn detect(&mut self) {
        let len = self.buf.len();
        for i in 0..len {
            self.frame[i] = self.buf[(self.pos + i) % len];
        }
        let x = &self.frame;
        let energy: f32 = x[..self.win].iter().map(|v| v * v).sum::<f32>() / self.win as f32;
        if energy < 1e-5 {
            self.freq = 0.0;
            return;
        }
        let mut running = 0.0;
        self.diff[0] = 1.0;
        for tau in 1..=self.tau_max {
            let mut d = 0.0;
            for j in 0..self.win {
                let e = x[j] - x[j + tau];
                d += e * e;
            }
            running += d;
            self.diff[tau] = if running > 0.0 { d * tau as f32 / running } else { 1.0 };
        }
        let mut found = None;
        let mut tau = self.tau_min;
        while tau < self.tau_max {
            if self.diff[tau] < self.threshold {
                while tau + 1 < self.tau_max && self.diff[tau + 1] < self.diff[tau] {
                    tau += 1;
                }
                found = Some(tau);
                break;
            }
            tau += 1;
        }
        self.freq = match found {
            Some(t) => {
                let (a, b, c) = (self.diff[t - 1], self.diff[t], self.diff[t + 1]);
                let den = a - 2.0 * b + c;
                let shift = if den.abs() > 1e-9 { 0.5 * (a - c) / den } else { 0.0 };
                self.rate / (t as f32 + shift.clamp(-1.0, 1.0))
            }
            None => 0.0,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_pitch_of_a_buzz() {
        for f in [90.0f32, 140.0, 220.0, 330.0] {
            let mut y = Yin::new(48000.0);
            for i in 0..24000 {
                let t = i as f32 / 48000.0;
                // Strong harmonics like a voice.
                let v = (t * f).fract() * 2.0 - 1.0;
                y.push(v * 0.4);
            }
            assert!((y.freq - f).abs() < f * 0.02, "wanted {f}, got {}", y.freq);
        }
    }
}
