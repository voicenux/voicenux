//! Background sounds, made from scratch so there are no sample files to license.
//! They only play while "Background effects" is on.

use super::biquad::{Biquad, FilterType, OnePole};
use super::util::{Brown, Phasor, Pink, Rng, db_to_gain, sine};
use super::{BuildCtx, Ctx, Effect};

pub const AMBIENCES: &[&str] = &[
    "Bubbles",
    "Wind",
    "Rain",
    "Campfire",
    "Ocean",
    "Space",
    "Radio static",
    "Cave drips",
    "Heartbeat",
    "Computer",
    "Storm",
    "Crowd",
    "Birds",
    "Engine",
];

/// Brings every sound to about the same loudness at the same level setting.
const NORM: [f32; 14] = [0.6, 0.87, 0.6, 0.27, 0.23, 0.6, 0.9, 0.6, 0.8, 1.7, 0.34, 1.1, 2.6, 0.4];

/// A decaying sine whose pitch glides, the building block of bubbles, drips and chirps.
#[derive(Clone, Copy, Default)]
struct Blip {
    phase: f32,
    freq: f32,
    glide: f32,
    amp: f32,
    decay: f32,
    attack: f32,
    age: f32,
}

impl Blip {
    #[inline]
    fn next(&mut self, rate: f32) -> f32 {
        if self.amp < 1e-4 {
            return 0.0;
        }
        self.phase = (self.phase + self.freq / rate).fract();
        self.freq *= self.glide;
        self.age += 1.0;
        let att = (self.age / self.attack.max(1.0)).min(1.0);
        let v = sine(self.phase) * self.amp * att;
        self.amp *= self.decay;
        v
    }
}

fn spawn(blips: &mut [Blip], b: Blip) {
    if let Some(slot) = blips.iter_mut().min_by(|a, c| a.amp.total_cmp(&c.amp)) {
        *slot = b;
    }
}

fn decay_for(ms: f32, rate: f32) -> f32 {
    // Amplitude falls to 1/1000 in `ms`.
    (0.001f32.ln() / (ms * 0.001 * rate)).exp()
}

fn glide_for(ratio: f32, ms: f32, rate: f32) -> f32 {
    ratio.powf(1.0 / (ms * 0.001 * rate))
}

/// Smoothed random value in [0, 1], for slow wandering parameters.
struct Drift {
    value: f32,
    target: f32,
    coef: f32,
    every: usize,
    count: usize,
}

impl Drift {
    fn new(seconds: f32, rate: f32) -> Self {
        let every = (rate * seconds) as usize;
        Self { value: 0.5, target: 0.5, coef: (-1.0 / every as f32).exp(), every, count: 0 }
    }

    #[inline]
    fn next(&mut self, rng: &mut Rng) -> f32 {
        self.count += 1;
        if self.count >= self.every {
            self.count = 0;
            self.target = rng.unit();
        }
        self.value = self.target + (self.value - self.target) * self.coef;
        self.value
    }
}

struct State {
    rate: f32,
    rng: Rng,
    pink: Pink,
    brown: Brown,
    blips: [Blip; 10],
    f: [Biquad; 4],
    lp: [OnePole; 3],
    drift: [Drift; 3],
    osc: [Phasor; 4],
    t: usize,
    next_event: usize,
    burst: usize,
    env: f32,
    talkers: [(f32, f32, usize, Biquad); 6],
}

pub struct Ambience {
    kind: usize,
    level: f32,
    s: State,
}

impl Ambience {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let rate = ctx.rate;
        let bp = |f| Biquad::new(FilterType::Bandpass, f, 1.0, 0.0, rate);
        let mut a = Self {
            kind: 0,
            level: db_to_gain(-18.0),
            s: State {
                rate,
                rng: Rng::new(2024),
                pink: Pink::default(),
                brown: Brown::default(),
                blips: [Blip::default(); 10],
                f: [bp(500.0), bp(1500.0), bp(3000.0), bp(800.0)],
                lp: [OnePole::new(300.0, rate), OnePole::new(1500.0, rate), OnePole::new(80.0, rate)],
                drift: [Drift::new(1.5, rate), Drift::new(0.6, rate), Drift::new(4.0, rate)],
                osc: Default::default(),
                t: 0,
                next_event: 0,
                burst: 0,
                env: 0.0,
                talkers: std::array::from_fn(|i| (0.0, 0.0, i * 997, bp(700.0))),
            },
        };
        for (i, v) in p.iter().enumerate() {
            a.set_param(i, *v);
        }
        a
    }
}

impl State {
    fn sec(&self, s: f32) -> usize {
        (s * self.rate) as usize
    }

    fn event_due(&mut self, min_s: f32, max_s: f32) -> bool {
        if self.t >= self.next_event {
            let wait = self.rng.range(min_s, max_s);
            self.next_event = self.t + self.sec(wait);
            true
        } else {
            false
        }
    }

    fn blips(&mut self) -> f32 {
        let rate = self.rate;
        self.blips.iter_mut().map(|b| b.next(rate)).sum()
    }

    fn bubbles(&mut self) -> f32 {
        let rate = self.rate;
        if self.event_due(0.15, 1.4) {
            self.burst = 1 + (self.rng.next_u32() % 6) as usize;
        }
        if self.burst > 0 && self.rng.chance(25.0 / rate) {
            self.burst -= 1;
            let freq = self.rng.range(350.0, 1500.0);
            let ms = self.rng.range(25.0, 70.0);
            spawn(
                &mut self.blips,
                Blip {
                    freq,
                    glide: glide_for(self.rng.range(1.6, 2.6), ms, rate),
                    amp: self.rng.range(0.25, 0.6),
                    decay: decay_for(ms, rate),
                    attack: rate * 0.002,
                    ..Blip::default()
                },
            );
        }
        let w = self.rng.bipolar();
        let rumble = self.lp[0].lowpass(self.brown.next(w)) * 0.5;
        self.blips() + rumble
    }

    fn wind(&mut self) -> f32 {
        let w = self.rng.bipolar();
        let n = self.pink.next(w);
        if self.t.is_multiple_of(64) {
            let c = 250.0 + 900.0 * self.drift[0].value;
            self.f[0].set(FilterType::Bandpass, c, 1.4, 0.0, self.rate);
            let whistle = 1400.0 + 1600.0 * self.drift[2].value;
            self.f[1].set(FilterType::Bandpass, whistle, 9.0, 0.0, self.rate);
        }
        self.drift[0].next(&mut self.rng);
        self.drift[2].next(&mut self.rng);
        let gust = 0.25 + 0.75 * self.drift[1].next(&mut self.rng).powi(2);
        (self.f[0].process(n) * 2.2 + self.f[1].process(n) * 0.7) * gust
    }

    fn rain(&mut self, heavy: bool) -> f32 {
        let rate = self.rate;
        let w = self.rng.bipolar();
        let bed = self.f[2].process(self.pink.next(w)) * if heavy { 0.9 } else { 0.5 };
        let drops_per_s = if heavy { 260.0 } else { 110.0 };
        if self.rng.chance(drops_per_s / rate) {
            self.env = self.env.max(self.rng.range(0.2, 1.0));
        }
        self.env *= 0.992;
        let drop = self.f[3].process(self.rng.bipolar()) * self.env * 1.8;
        bed + drop
    }

    fn campfire(&mut self) -> f32 {
        let rate = self.rate;
        let w = self.rng.bipolar();
        let roar = self.lp[0].lowpass(self.brown.next(w)) * (0.5 + 0.5 * self.drift[1].next(&mut self.rng));
        if self.event_due(0.05, 0.6) {
            self.burst = 1 + (self.rng.next_u32() % 4) as usize;
        }
        if self.burst > 0 && self.rng.chance(40.0 / rate) {
            self.burst -= 1;
            self.env = self.rng.range(0.4, 1.0);
        }
        self.env *= 0.94;
        let crackle = self.f[2].process(self.rng.bipolar() * self.env) * 1.6;
        let hiss = self.lp[1].highpass(self.rng.bipolar()) * 0.04;
        roar * 1.2 + crackle + hiss
    }

    fn ocean(&mut self) -> f32 {
        let period = self.sec(8.0);
        let pos = (self.t % period) as f32 / period as f32;
        let swell = if pos < 0.35 { (pos / 0.35).powi(2) } else { (1.0 - (pos - 0.35) / 0.65).powf(1.5) };
        let c = 250.0 + 2500.0 * swell;
        if self.t.is_multiple_of(64) {
            self.lp[1].set(c, self.rate);
        }
        let w = self.rng.bipolar();
        let n = self.pink.next(w);
        self.lp[1].lowpass(n) * (0.25 + 0.9 * swell) * 1.6 + self.lp[2].lowpass(self.brown.next(w)) * 0.6
    }

    fn space(&mut self) -> f32 {
        let rate = self.rate;
        let mut drone = 0.0;
        for (i, f) in [55.0, 55.35, 82.4, 110.2].iter().enumerate() {
            drone += sine(self.osc[i].next(*f, rate)) * [0.35, 0.35, 0.2, 0.12][i];
        }
        let wob = 0.6 + 0.4 * self.drift[2].next(&mut self.rng);
        if self.event_due(3.0, 9.0) {
            spawn(
                &mut self.blips,
                Blip {
                    freq: self.rng.range(1200.0, 2600.0),
                    glide: 1.0,
                    amp: 0.12,
                    decay: decay_for(1800.0, rate),
                    attack: rate * 0.01,
                    ..Blip::default()
                },
            );
        }
        let air = self.f[1].process(self.pink.next(self.rng.bipolar())) * 0.15 * wob;
        drone * wob * 0.8 + air + self.blips()
    }

    fn static_(&mut self) -> f32 {
        let w = self.rng.bipolar();
        if self.event_due(0.03, 0.15) {
            self.env = self.rng.range(0.3, 1.0);
        }
        let pop = if self.rng.chance(6.0 / self.rate) { self.rng.bipolar() * 3.0 } else { 0.0 };
        (self.f[1].process(w) * self.env + pop * 0.3) * 1.2
    }

    fn drips(&mut self) -> f32 {
        let rate = self.rate;
        if self.event_due(0.35, 1.8) {
            let freq = self.rng.range(700.0, 1400.0);
            spawn(
                &mut self.blips,
                Blip {
                    freq,
                    glide: glide_for(2.2, 40.0, rate),
                    amp: self.rng.range(0.3, 0.7),
                    decay: decay_for(90.0, rate),
                    attack: rate * 0.001,
                    ..Blip::default()
                },
            );
        }
        let w = self.rng.bipolar();
        self.blips() + self.lp[2].lowpass(self.brown.next(w)) * 0.5
    }

    fn heartbeat(&mut self) -> f32 {
        let rate = self.rate;
        let beat = self.sec(60.0 / 66.0);
        let pos = self.t % beat;
        if pos == 0 || pos == self.sec(0.28) {
            let strong = pos == 0;
            spawn(
                &mut self.blips,
                Blip {
                    freq: if strong { 62.0 } else { 70.0 },
                    glide: glide_for(0.65, 100.0, rate),
                    amp: if strong { 1.0 } else { 0.7 },
                    decay: decay_for(if strong { 160.0 } else { 130.0 }, rate),
                    attack: rate * 0.004,
                    ..Blip::default()
                },
            );
        }
        let b = self.blips();
        self.lp[0].lowpass(b) * 1.3
    }

    fn computer(&mut self) -> f32 {
        let rate = self.rate;
        const NOTES: [f32; 7] = [880.0, 1046.5, 1174.7, 1318.5, 1568.0, 1760.0, 2093.0];
        if self.burst == 0 && self.event_due(0.6, 2.8) {
            self.burst = 3 + (self.rng.next_u32() % 7) as usize;
        }
        if self.burst > 0 && self.rng.chance(9.0 / rate) {
            self.burst -= 1;
            let note = NOTES[(self.rng.next_u32() % 7) as usize];
            spawn(
                &mut self.blips,
                Blip {
                    freq: note,
                    glide: 1.0,
                    amp: 0.22,
                    decay: decay_for(self.rng.range(70.0, 160.0), rate),
                    attack: rate * 0.002,
                    ..Blip::default()
                },
            );
        }
        let bleeps = (self.blips() * 2.5).tanh() * 0.5;
        let p = self.osc[0].next(60.0, rate);
        let hum = (sine(p) * 0.5 + sine(p * 2.0) * 0.3 + sine(p * 3.0) * 0.15) * 0.12;
        let fan = self.lp[1].lowpass(self.pink.next(self.rng.bipolar())) * 0.12;
        bleeps + hum + fan
    }

    fn storm(&mut self) -> f32 {
        let rain = self.rain(true);
        if self.event_due(7.0, 16.0) {
            self.burst = self.sec(4.5);
        }
        let mut thunder = 0.0;
        if self.burst > 0 {
            self.burst -= 1;
            let left = self.burst as f32 / self.sec(4.5) as f32;
            let shape = if left > 0.92 { (1.0 - left) / 0.08 } else { left.powf(1.4) };
            let w = self.rng.bipolar();
            let rumble = self.lp[0].lowpass(self.brown.next(w)) * (0.6 + 0.4 * self.drift[1].next(&mut self.rng));
            thunder = rumble * shape * 1.2;
        }
        rain + thunder
    }

    fn crowd(&mut self) -> f32 {
        let rate = self.rate;
        const FORMANTS: [f32; 6] = [450.0, 700.0, 950.0, 1250.0, 1800.0, 2500.0];
        let mut babble = 0.0;
        for i in 0..self.talkers.len() {
            let (amp, target, ref mut left, ref mut f) = self.talkers[i];
            let mut amp = amp;
            let mut target = target;
            if *left == 0 {
                *left = (self.rng.range(0.08, 0.3) * rate) as usize;
                target = if self.rng.chance(0.75) { self.rng.range(0.4, 1.0) } else { 0.0 };
                let fq = FORMANTS[(self.rng.next_u32() % 6) as usize];
                f.set(FilterType::Bandpass, fq, 3.0, 0.0, rate);
            }
            *left -= 1;
            amp += (target - amp) * 0.002;
            let n = self.rng.bipolar();
            babble += f.process(n) * amp;
            self.talkers[i].0 = amp;
            self.talkers[i].1 = target;
        }
        let roar = self.lp[1].lowpass(self.pink.next(self.rng.bipolar()));
        let cheer = 0.5 + 0.5 * self.drift[2].next(&mut self.rng).powi(3);
        babble * 0.5 + roar * 0.6 * cheer
    }

    fn birds(&mut self) -> f32 {
        let rate = self.rate;
        if self.burst == 0 && self.event_due(1.0, 4.0) {
            self.burst = 2 + (self.rng.next_u32() % 5) as usize;
        }
        if self.burst > 0 && self.rng.chance(7.0 / rate) {
            self.burst -= 1;
            let up = self.rng.chance(0.5);
            let ms = self.rng.range(40.0, 110.0);
            spawn(
                &mut self.blips,
                Blip {
                    freq: if up { self.rng.range(2500.0, 3500.0) } else { self.rng.range(4000.0, 5500.0) },
                    glide: glide_for(if up { 1.6 } else { 0.6 }, ms, rate),
                    amp: 0.25,
                    decay: decay_for(ms * 1.3, rate),
                    attack: rate * 0.008,
                    ..Blip::default()
                },
            );
        }
        let leaves = self.f[2].process(self.pink.next(self.rng.bipolar())) * 0.08 * self.drift[1].next(&mut self.rng);
        self.blips() + leaves
    }

    fn engine(&mut self) -> f32 {
        let rate = self.rate;
        let mut hum = 0.0;
        for (i, f) in [46.0f32, 92.3, 138.0].iter().enumerate() {
            let p = self.osc[i].next(*f, rate);
            hum += (2.0 * p - 1.0) * [0.4, 0.25, 0.12][i];
        }
        let hum = self.lp[0].lowpass(hum);
        let air =
            self.lp[1].lowpass(self.pink.next(self.rng.bipolar())) * (0.8 + 0.2 * self.drift[1].next(&mut self.rng));
        hum * 1.2 + air * 0.7
    }
}

impl Effect for Ambience {
    fn process(&mut self, buf: &mut [f32], ctx: &Ctx) {
        if !ctx.background {
            return;
        }
        for x in buf.iter_mut() {
            let s = &mut self.s;
            s.t += 1;
            let v = match self.kind {
                0 => s.bubbles(),
                1 => s.wind(),
                2 => s.rain(false),
                3 => s.campfire(),
                4 => s.ocean(),
                5 => s.space(),
                6 => s.static_(),
                7 => s.drips(),
                8 => s.heartbeat(),
                9 => s.computer(),
                10 => s.storm(),
                11 => s.crowd(),
                12 => s.birds(),
                _ => s.engine(),
            };
            *x += v * self.level * NORM[self.kind];
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => {
                self.kind = (v as usize).min(AMBIENCES.len() - 1);
                let rate = self.s.rate;
                let (f2, f3) = match self.kind {
                    2 | 10 => (
                        Biquad::new(FilterType::Highpass, 900.0, 0.7, 0.0, rate),
                        Biquad::new(FilterType::Highpass, 2500.0, 0.7, 0.0, rate),
                    ),
                    3 => (Biquad::new(FilterType::Highpass, 1800.0, 0.7, 0.0, rate), Biquad::default()),
                    12 => (Biquad::new(FilterType::Highpass, 3000.0, 0.7, 0.0, rate), Biquad::default()),
                    _ => (self.s.f[2], self.s.f[3]),
                };
                self.s.f[2] = f2;
                self.s.f[3] = f3;
                if self.kind == 6 {
                    self.s.f[1] = Biquad::new(FilterType::Bandpass, 1900.0, 0.8, 0.0, rate);
                }
                if self.kind == 13 {
                    self.s.lp[1].set(700.0, rate);
                }
            }
            1 => self.level = db_to_gain(v),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ambience_makes_sound_and_stays_sane() {
        let ctx = BuildCtx::test();
        for (kind, name) in AMBIENCES.iter().enumerate() {
            let mut a = Ambience::new(&[kind as f32, 0.0], &ctx);
            let mut x = vec![0.0; 48000 * 12];
            a.process(&mut x, &Ctx { background: true });
            let rms = crate::dsp::util::rms(&x);
            let peak = crate::dsp::util::peak(&x);
            assert!(rms > 0.003, "{name} too quiet: {rms}");
            assert!(peak < 4.0 && x.iter().all(|v| v.is_finite()), "{name} too loud: {peak}");
        }
    }

    #[test]
    fn silent_without_background() {
        let ctx = BuildCtx::test();
        let mut a = Ambience::new(&[1.0, 0.0], &ctx);
        let mut x = vec![0.0; 4800];
        a.process(&mut x, &Ctx { background: false });
        assert!(x.iter().all(|v| *v == 0.0));
    }
}

#[cfg(test)]
mod levels {
    use super::*;

    #[test]
    #[ignore]
    fn print_levels() {
        let ctx = BuildCtx::test();
        for (kind, name) in AMBIENCES.iter().enumerate() {
            let mut a = Ambience::new(&[kind as f32, 0.0], &ctx);
            let mut x = vec![0.0; 48000 * 12];
            a.process(&mut x, &Ctx { background: true });
            println!("{name:14} rms {:.3} peak {:.3}", crate::dsp::util::rms(&x), crate::dsp::util::peak(&x));
        }
    }
}
