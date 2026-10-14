//! Measuring helpers for the tests.

use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;

fn spectrum(x: &[f32]) -> Vec<f32> {
    let n = x.len().next_power_of_two() / 2;
    let x = &x[..n];
    let mut buf: Vec<Complex32> = x
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
            Complex32::new(v * w, 0.0)
        })
        .collect();
    FftPlanner::new().plan_fft_forward(n).process(&mut buf);
    buf[..n / 2].iter().map(|c| c.norm()).collect()
}

/// Frequency of the strongest bin.
pub fn dominant_freq(x: &[f32], rate: f32) -> f32 {
    spectral_peak_in(x, rate, 20.0, rate / 2.0)
}

/// Frequency of the strongest bin between `lo` and `hi`.
pub fn spectral_peak_in(x: &[f32], rate: f32, lo: f32, hi: f32) -> f32 {
    let s = spectrum(x);
    let n = s.len() * 2;
    let bin = rate / n as f32;
    let a = (lo / bin) as usize;
    let b = ((hi / bin) as usize).min(s.len() - 1);
    let mut best = a;
    for k in a..=b {
        if s[k] > s[best] {
            best = k;
        }
    }
    best as f32 * bin
}
