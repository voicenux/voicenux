//! Pitch correction: finds the sung note and pulls it to the nearest note of a scale.

use super::pitch::{DryDelay, PitchShifter};
use super::util::semitones_to_ratio;
use super::yin::Yin;
use super::{BuildCtx, Ctx, Effect};

pub const KEYS: &[&str] = &["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
pub const SCALES: &[&str] = &["Chromatic", "Major", "Minor", "Pentatonic", "Blues"];

fn scale_steps(scale: usize) -> &'static [i32] {
    match scale {
        1 => &[0, 2, 4, 5, 7, 9, 11],
        2 => &[0, 2, 3, 5, 7, 8, 10],
        3 => &[0, 2, 4, 7, 9],
        4 => &[0, 3, 5, 6, 7, 10],
        _ => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    }
}

/// Nearest MIDI note (fractional input) that lies in the scale.
pub fn snap(midi: f32, key: usize, scale: usize) -> f32 {
    let steps = scale_steps(scale);
    let base = midi.floor() as i32;
    let mut best = base as f32;
    let mut dist = f32::MAX;
    for cand in base - 6..=base + 6 {
        let rel = (cand - key as i32).rem_euclid(12);
        if steps.contains(&rel) {
            let d = (cand as f32 - midi).abs();
            if d < dist {
                dist = d;
                best = cand as f32;
            }
        }
    }
    best
}

pub struct AutoTune {
    yin: Yin,
    shifter: PitchShifter,
    dry: DryDelay,
    key: usize,
    scale: usize,
    speed: f32,
    mix: f32,
    formant: f32,
    shift: f32,
    ratio: f32,
    unvoiced: usize,
    rate: f32,
    scratch: Vec<f32>,
}

impl AutoTune {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        let shifter = PitchShifter::new(ctx.rate, ctx.low_latency);
        let dry = DryDelay::new(shifter.latency());
        let mut a = Self {
            yin: Yin::new(ctx.rate),
            shifter,
            dry,
            key: 0,
            scale: 0,
            speed: 0.9,
            mix: 1.0,
            formant: 1.0,
            shift: 0.0,
            ratio: 1.0,
            unvoiced: 0,
            rate: ctx.rate,
            scratch: vec![0.0; ctx.max_block],
        };
        for (i, v) in p.iter().enumerate() {
            a.set_param(i, *v);
        }
        a
    }

    fn update(&mut self, chunk: usize) {
        let f = self.yin.freq;
        let chunk_ms = chunk as f32 / self.rate * 1000.0;
        let target = if f > 0.0 {
            self.unvoiced = 0;
            let midi = 69.0 + 12.0 * (f / 440.0).log2();
            let note = snap(midi, self.key, self.scale) + self.shift;
            semitones_to_ratio(note - midi)
        } else {
            self.unvoiced += chunk;
            if (self.unvoiced as f32) < self.rate * 0.08 { self.ratio } else { semitones_to_ratio(self.shift) }
        };
        // Speed 1 snaps at once (the robotic effect), lower glides like a real singer.
        let tc = (1.0 - self.speed) * 160.0 + 0.5;
        let c = (-chunk_ms / tc).exp();
        self.ratio = target + (self.ratio - target) * c;
        self.shifter.ratio = self.ratio;
        self.shifter.formant = self.formant;
    }
}

impl Effect for AutoTune {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        let n = buf.len().min(self.scratch.len());
        let buf = &mut buf[..n];
        self.scratch[..n].copy_from_slice(buf);
        for chunk in buf.chunks_mut(64) {
            for x in chunk.iter() {
                self.yin.push(*x);
            }
            self.update(chunk.len());
            self.shifter.process(chunk);
        }
        if self.mix < 0.999 {
            for (y, x) in buf.iter_mut().zip(&self.scratch[..n]) {
                *y = *y * self.mix + self.dry.next(*x) * (1.0 - self.mix);
            }
        }
    }

    fn set_param(&mut self, index: usize, v: f32) {
        match index {
            0 => self.key = (v as usize).min(11),
            1 => self.scale = (v as usize).min(SCALES.len() - 1),
            2 => self.speed = v.clamp(0.0, 1.0),
            3 => self.shift = v,
            4 => self.formant = semitones_to_ratio(v),
            5 => self.mix = v.clamp(0.0, 1.0),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_scale() {
        // 61.4 is between C# and D. C major has no C#, so D.
        assert_eq!(snap(61.4, 0, 1), 62.0);
        assert_eq!(snap(61.4, 0, 0), 61.0);
        assert_eq!(snap(66.4, 0, 1), 67.0);
    }
}
