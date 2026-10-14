//! The starter sounds, made from scratch. No sample files, so no licenses to worry about.

use super::{RATE, SoundData};
use crate::dsp::biquad::{Biquad, FilterType, OnePole};
use crate::dsp::util::{Brown, Pink, Rng, TAU};

pub struct Builtin {
    pub id: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    render: fn() -> Vec<[f32; 2]>,
}

pub static BUILTIN: &[Builtin] = &[
    Builtin { id: "airhorn", name: "Air Horn", icon: "postal-horn", render: airhorn },
    Builtin { id: "sad-trombone", name: "Sad Trombone", icon: "trumpet", render: sad_trombone },
    Builtin { id: "rimshot", name: "Rimshot", icon: "joy", render: rimshot },
    Builtin { id: "drumroll", name: "Drum Roll", icon: "drum", render: drumroll },
    Builtin { id: "applause", name: "Applause", icon: "clapping", render: applause },
    Builtin { id: "tada", name: "Ta-da!", icon: "party-popper", render: tada },
    Builtin { id: "correct", name: "Correct", icon: "bell", render: correct },
    Builtin { id: "wrong", name: "Wrong", icon: "cross-mark", render: wrong },
    Builtin { id: "crickets", name: "Crickets", icon: "cricket", render: crickets },
    Builtin { id: "boing", name: "Boing", icon: "basketball", render: boing },
    Builtin { id: "laser", name: "Laser", icon: "high-voltage", render: laser },
    Builtin { id: "explosion", name: "Explosion", icon: "bomb", render: explosion },
    Builtin { id: "coin", name: "Coin", icon: "coin", render: coin },
    Builtin { id: "whoosh", name: "Whoosh", icon: "dashing-away", render: whoosh },
];

pub fn render(id: &str) -> Option<SoundData> {
    let b = BUILTIN.iter().find(|b| b.id == id)?;
    let mut frames = (b.render)();
    normalize(&mut frames, 0.7);
    Some(SoundData { frames })
}

const SR: f32 = RATE as f32;

fn len(seconds: f32) -> usize {
    (seconds * SR) as usize
}

fn normalize(frames: &mut [[f32; 2]], peak: f32) {
    let m = frames.iter().fold(0.0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    if m > 0.0 {
        for f in frames.iter_mut() {
            f[0] *= peak / m;
            f[1] *= peak / m;
        }
    }
}

fn mono(v: Vec<f32>) -> Vec<[f32; 2]> {
    v.into_iter().map(|x| [x, x]).collect()
}

/// Short fade at both ends against clicks.
fn fades(v: &mut [f32]) {
    let n = len(0.004).min(v.len() / 2);
    for i in 0..n {
        let g = i as f32 / n as f32;
        v[i] *= g;
        let j = v.len() - 1 - i;
        v[j] *= g;
    }
}

/// Band limited sawtooth, phase in [0, 1).
fn saw(phase: &mut f32, freq: f32) -> f32 {
    let dt = freq / SR;
    let v = 2.0 * *phase - 1.0 - crate::dsp::util::poly_blep(*phase, dt);
    *phase = (*phase + dt).fract();
    v
}

fn airhorn() -> Vec<[f32; 2]> {
    // BWAA BWAA BWAAAAAH
    let notes = [(0.0, 0.16), (0.22, 0.16), (0.44, 0.9)];
    let total = len(1.45);
    let mut out = vec![0.0f32; total];
    let mut ph = [0.0f32; 4];
    let mut lp = Biquad::new(FilterType::Lowpass, 3200.0, 0.9, 0.0, SR);
    let mut honk = Biquad::new(FilterType::Peak, 1100.0, 1.2, 8.0, SR);
    for (start, dur) in notes {
        let s = len(start);
        for i in 0..len(dur) {
            let t = i as f32 / SR;
            let bend = 1.0 - 0.06 * (-t * 30.0).exp();
            let f = 466.0 * bend;
            let env = (t / 0.01).min(1.0) * ((dur - t) / 0.03).clamp(0.0, 1.0);
            let v = saw(&mut ph[0], f)
                + saw(&mut ph[1], f * 1.007)
                + 0.7 * saw(&mut ph[2], f * 0.5)
                + 0.5 * saw(&mut ph[3], f * 1.5);
            out[s + i] += (v * 0.8).tanh() * env;
        }
    }
    let out: Vec<f32> = out.into_iter().map(|x| honk.process(lp.process(x))).collect();
    mono(out)
}

fn sad_trombone() -> Vec<[f32; 2]> {
    // Wah wah wah waaaah: D, C#, C, then a long B with vibrato.
    let notes = [(293.7, 0.42), (277.2, 0.42), (261.6, 0.42), (246.9, 1.5)];
    let mut out = Vec::new();
    let mut ph = 0.0f32;
    let mut ph2 = 0.0f32;
    for (k, (f, dur)) in notes.iter().enumerate() {
        let mut lp = Biquad::new(FilterType::Lowpass, 400.0, 2.0, 0.0, SR);
        for i in 0..len(*dur) {
            let t = i as f32 / SR;
            let vib = if k == 3 { 1.0 + 0.012 * (TAU * 5.0 * t).sin() * (t / 0.4).min(1.0) } else { 1.0 };
            let env = (t / 0.03).min(1.0) * ((dur - t) / 0.06).clamp(0.0, 1.0);
            // The "wah": the mute opens then closes again.
            let open = (t / dur * std::f32::consts::PI).sin();
            if i % 32 == 0 {
                lp.set(FilterType::Lowpass, 350.0 + 1600.0 * open, 2.5, 0.0, SR);
            }
            let v = saw(&mut ph, f * vib) + 0.5 * saw(&mut ph2, f * vib * 2.0);
            out.push(lp.process(v) * env);
        }
    }
    mono(out)
}

/// A drum: sine with falling pitch plus a noise click.
fn drum(out: &mut [f32], at: usize, f0: f32, f1: f32, decay: f32, noise: f32, rng: &mut Rng) {
    let mut ph = 0.0f32;
    for i in 0..len(decay * 4.0) {
        if at + i >= out.len() {
            break;
        }
        let t = i as f32 / SR;
        let f = f1 + (f0 - f1) * (-t * 25.0).exp();
        ph = (ph + f / SR).fract();
        let env = (-t / decay).exp();
        out[at + i] += ((TAU * ph).sin() + rng.bipolar() * noise * (-t * 60.0).exp()) * env;
    }
}

fn cymbal(out: &mut [f32], at: usize, decay: f32, rng: &mut Rng) {
    let mut hp = Biquad::new(FilterType::Highpass, 5000.0, 0.7, 0.0, SR);
    let freqs = [587.0f32, 845.0, 1123.0, 1440.0, 1820.0, 2398.0];
    let mut ph = [0.0f32; 6];
    for i in 0..len(decay * 4.0) {
        if at + i >= out.len() {
            break;
        }
        let t = i as f32 / SR;
        let mut metal = 0.0;
        for (p, f) in ph.iter_mut().zip(freqs) {
            *p = (*p + f / SR).fract();
            metal += if *p < 0.5 { 1.0 } else { -1.0 };
        }
        let v = hp.process(rng.bipolar() + metal * 0.15);
        out[at + i] += v * (-t / decay).exp() * 0.6;
    }
}

fn rimshot() -> Vec<[f32; 2]> {
    let mut rng = Rng::new(5);
    let mut out = vec![0.0f32; len(1.8)];
    drum(&mut out, 0, 260.0, 160.0, 0.09, 0.6, &mut rng);
    drum(&mut out, len(0.2), 200.0, 120.0, 0.1, 0.6, &mut rng);
    drum(&mut out, len(0.4), 230.0, 180.0, 0.05, 1.5, &mut rng);
    cymbal(&mut out, len(0.4), 0.35, &mut rng);
    mono(out)
}

fn drumroll() -> Vec<[f32; 2]> {
    let mut rng = Rng::new(9);
    let mut out = vec![0.0f32; len(4.2)];
    let roll: f32 = 2.6;
    let mut t = 0.0;
    while t < roll {
        let level = 0.25 + 0.75 * (t / roll).powi(2);
        let at = len(t);
        let mut snare = vec![0.0f32; len(0.12)];
        drum(&mut snare, 0, 240.0, 190.0, 0.03, 2.5, &mut rng);
        for (i, v) in snare.iter().enumerate() {
            if at + i < out.len() {
                out[at + i] += v * level * rng.range(0.8, 1.0);
            }
        }
        t += 1.0 / 22.0 + rng.range(-0.004, 0.004);
    }
    drum(&mut out, len(roll + 0.05), 120.0, 60.0, 0.25, 0.8, &mut rng);
    cymbal(&mut out, len(roll + 0.05), 0.6, &mut rng);
    mono(out)
}

fn applause() -> Vec<[f32; 2]> {
    let mut rng = Rng::new(21);
    let total = len(4.0);
    let mut out = vec![[0.0f32; 2]; total];
    let mut clap = vec![0.0f32; len(0.03)];
    let mut bp = Biquad::new(FilterType::Bandpass, 1400.0, 1.0, 0.0, SR);
    for (i, c) in clap.iter_mut().enumerate() {
        let t = i as f32 / SR;
        *c = bp.process(rng.bipolar()) * (-t / 0.006).exp();
    }
    let mut t = 0.0f32;
    while t < 3.7 {
        let swell = (t / 0.4).min(1.0) * ((3.7 - t) / 1.4).clamp(0.0, 1.0);
        let at = len(t);
        let pan = rng.unit();
        let amp = rng.range(0.3, 1.0) * swell;
        let pitch = rng.range(0.7, 1.4);
        for i in 0..clap.len() {
            let src = (i as f32 * pitch) as usize;
            if src >= clap.len() || at + i >= total {
                break;
            }
            out[at + i][0] += clap[src] * amp * (1.0 - pan);
            out[at + i][1] += clap[src] * amp * pan;
        }
        t += 1.0 / 180.0 * rng.range(0.2, 1.8);
    }
    out
}

/// Brass-like tone for fanfares.
fn brass(out: &mut [f32], at: usize, freqs: &[f32], dur: f32) {
    let mut ph = vec![0.0f32; freqs.len()];
    let mut lp = Biquad::new(FilterType::Lowpass, 500.0, 1.0, 0.0, SR);
    for i in 0..len(dur) {
        if at + i >= out.len() {
            break;
        }
        let t = i as f32 / SR;
        if i % 32 == 0 {
            lp.set(FilterType::Lowpass, 500.0 + 3500.0 * (t / 0.06).min(1.0), 1.0, 0.0, SR);
        }
        let env = (t / 0.02).min(1.0) * ((dur - t) / 0.08).clamp(0.0, 1.0);
        let vib = 1.0 + 0.004 * (TAU * 5.5 * t).sin() * (t / 0.3).min(1.0);
        let mut v = 0.0;
        for (p, f) in ph.iter_mut().zip(freqs) {
            v += saw(p, f * vib);
        }
        out[at + i] += lp.process(v) * env;
    }
}

fn tada() -> Vec<[f32; 2]> {
    let mut out = vec![0.0f32; len(1.9)];
    brass(&mut out, 0, &[466.2, 587.3, 698.5], 0.13);
    brass(&mut out, len(0.17), &[523.3, 659.3, 784.0, 1046.5], 1.6);
    mono(out)
}

/// A bell: inharmonic partials that each fade at their own speed.
fn bell(out: &mut [f32], at: usize, f: f32, dur: f32) {
    let partials = [(1.0, 1.0, 1.0), (2.76, 0.5, 0.6), (5.4, 0.25, 0.35), (8.93, 0.12, 0.2)];
    for i in 0..len(dur) {
        if at + i >= out.len() {
            break;
        }
        let t = i as f32 / SR;
        let mut v = 0.0;
        for (m, a, d) in partials {
            v += (TAU * f * m * t).sin() * a * (-t / (dur * d * 0.4)).exp();
        }
        out[at + i] += v * (t / 0.002).min(1.0);
    }
}

fn correct() -> Vec<[f32; 2]> {
    let mut out = vec![0.0f32; len(1.6)];
    bell(&mut out, 0, 1046.5, 1.2);
    bell(&mut out, len(0.14), 1318.5, 1.4);
    mono(out)
}

fn wrong() -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    let mut lp = OnePole::new(2500.0, SR);
    for (start, dur) in [(0.0f32, 0.28f32), (0.34, 0.7)] {
        while out.len() < len(start) {
            out.push(0.0);
        }
        for i in 0..len(dur) {
            let t = i as f32 / SR;
            let a = if (t * 150.0).fract() < 0.5 { 1.0 } else { -1.0 };
            let b = if (t * 156.0).fract() < 0.5 { 1.0 } else { -1.0 };
            let env = (t / 0.005).min(1.0) * ((dur - t) / 0.02).clamp(0.0, 1.0);
            out.push(lp.lowpass((a + b) * 0.5) * env);
        }
    }
    mono(out)
}

fn crickets() -> Vec<[f32; 2]> {
    let mut rng = Rng::new(77);
    let total = len(4.0);
    let mut out = vec![[0.0f32; 2]; total];
    for (c, (pan, f, every)) in [(0.25f32, 4600.0f32, 0.62f32), (0.8, 4300.0, 0.81)].into_iter().enumerate() {
        let mut t = 0.1 + c as f32 * 0.27;
        while t < 3.8 {
            // One chirp: three quick pulses.
            for p in 0..3 {
                let at = len(t + p as f32 * 0.032);
                for i in 0..len(0.022) {
                    if at + i >= total {
                        break;
                    }
                    let tt = i as f32 / SR;
                    let env = (tt / 0.022 * std::f32::consts::PI).sin();
                    let v = (TAU * f * tt).sin() * env * 0.5;
                    out[at + i][0] += v * (1.0 - pan);
                    out[at + i][1] += v * pan;
                }
            }
            t += every + rng.range(-0.05, 0.05);
        }
    }
    out
}

fn boing() -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    let mut ph = 0.0f32;
    for i in 0..len(1.1) {
        let t = i as f32 / SR;
        let f = 180.0 * (1.0 + 0.45 * (TAU * 13.0 * t).sin() * (-t * 3.5).exp()) * (1.0 + t * 0.3);
        ph = (ph + f / SR).fract();
        let v = (TAU * ph).sin() + 0.3 * (TAU * ph * 2.0).sin();
        out.push(v * (-t * 3.0).exp() * (t / 0.004).min(1.0));
    }
    fades(&mut out);
    mono(out)
}

fn laser() -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    let mut ph = 0.0f32;
    for i in 0..len(0.32) {
        let t = i as f32 / SR;
        let f = 2400.0 * (-t * 9.0).exp() + 180.0;
        ph = (ph + f / SR).fract();
        let sq = if ph < 0.5 { 1.0 } else { -1.0 };
        let v = (TAU * ph).sin() * 0.7 + sq * 0.3;
        out.push(v * (-t * 6.0).exp());
    }
    fades(&mut out);
    mono(out)
}

fn explosion() -> Vec<[f32; 2]> {
    let mut rng = Rng::new(31);
    let total = len(3.2);
    let mut out = vec![[0.0f32; 2]; total];
    let mut lp = [Biquad::default(), Biquad::default()];
    let mut br = [Brown::default(), Brown::default()];
    let mut ph = 0.0f32;
    for (i, frame) in out.iter_mut().enumerate() {
        let t = i as f32 / SR;
        if i % 64 == 0 {
            let c = 150.0 + 4000.0 * (-t * 2.5).exp();
            for f in &mut lp {
                f.set(FilterType::Lowpass, c, 0.8, 0.0, SR);
            }
        }
        let env = (t / 0.004).min(1.0) * (-t * 1.4).exp();
        ph = (ph + (30.0 + 50.0 * (-t * 8.0).exp()) / SR).fract();
        let thump = (TAU * ph).sin() * (-t * 4.0).exp() * 1.2;
        for ch in 0..2 {
            let w = rng.bipolar();
            let n = br[ch].next(w) * 0.6 + w * 0.4;
            frame[ch] = (lp[ch].process(n) * 2.0 + thump) * env;
        }
    }
    out
}

fn coin() -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    for (f, dur, decay) in [(987.8f32, 0.07f32, false), (1318.5, 0.45, true)] {
        for i in 0..len(dur) {
            let t = i as f32 / SR;
            let sq = if (t * f).fract() < 0.5 { 1.0 } else { -1.0 };
            let env = if decay { (-t * 7.0).exp() } else { 1.0 };
            out.push(sq * env * 0.5);
        }
    }
    fades(&mut out);
    mono(out)
}

fn whoosh() -> Vec<[f32; 2]> {
    let mut rng = Rng::new(3);
    let mut pink = Pink::default();
    let total = len(0.9);
    let mut bp = Biquad::default();
    let mut out = Vec::with_capacity(total);
    for i in 0..total {
        let x = i as f32 / total as f32;
        if i % 32 == 0 {
            let c = 300.0 + 2800.0 * (x * std::f32::consts::PI).sin().powi(2);
            bp.set(FilterType::Bandpass, c, 1.4, 0.0, SR);
        }
        let env = (x * std::f32::consts::PI).sin().powf(1.5);
        let v = bp.process(pink.next(rng.bipolar())) * env * 3.0;
        // Moves from left to right.
        out.push([v * (1.0 - x), v * x]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_renders() {
        for b in BUILTIN {
            let d = render(b.id).unwrap();
            assert!(d.seconds() > 0.2 && d.seconds() < 6.0, "{} is {} s", b.id, d.seconds());
            assert!(d.frames.iter().all(|f| f[0].is_finite() && f[1].is_finite()));
            let peak = d.frames.iter().fold(0.0f32, |m, f| m.max(f[0].abs()));
            assert!(
                (peak - 0.7).abs() < 0.05 || b.id == "crickets" || b.id == "applause" || b.id == "whoosh",
                "{} peak {peak}",
                b.id
            );
        }
    }
}
