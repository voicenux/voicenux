//! A voice as it runs: its blocks in a row.

use super::biquad::{Biquad, FilterType};
use super::blocks;
use super::util::{TAU, rms};
use super::{BuildCtx, Ctx, Effect};

/// A buzzy test voice that talks in syllables, to measure loudness with.
pub fn talking(rate: f32, seconds: f32) -> Vec<f32> {
    let mut f1 = Biquad::new(FilterType::Bandpass, 700.0, 1.5, 0.0, rate);
    let mut f2 = Biquad::new(FilterType::Bandpass, 1800.0, 2.0, 0.0, rate);
    let mut phase = 0.0f32;
    (0..(rate * seconds) as usize)
        .map(|i| {
            let t = i as f32 / rate;
            let f0 = 130.0 + 20.0 * (t * 1.3).sin();
            phase = (phase + f0 / rate).fract();
            let saw = phase * 2.0 - 1.0;
            let syllables = (0.5 + 0.5 * (t * TAU * 3.0).sin()).powi(2);
            (f1.process(saw) + f2.process(saw) * 0.5) * syllables * 0.6
        })
        .collect()
}

/// One block as the engine needs it: its kind and parameter values in spec order.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockValues {
    pub kind: String,
    pub enabled: bool,
    pub values: Vec<f32>,
}

pub struct Chain {
    pub rate: u32,
    blocks: Vec<(Box<dyn Effect>, bool)>,
    max_block: usize,
    /// Brings the voice back to the loudness of the dry voice.
    trim: f32,
}

impl Chain {
    pub fn build(defs: &[BlockValues], ctx: &BuildCtx) -> Self {
        let blocks = defs
            .iter()
            .filter_map(|d| {
                let spec = blocks::find(&d.kind)?;
                Some(((spec.build)(&d.values, ctx), d.enabled))
            })
            .collect();
        Self { rate: ctx.rate as u32, blocks, max_block: ctx.max_block, trim: 1.0 }
    }

    /// Builds the chain and measures how much louder or quieter it makes a talking voice,
    /// so every voice ends up about as loud as your own.
    pub fn build_leveled(defs: &[BlockValues], ctx: &BuildCtx) -> Self {
        let mut chain = Self::build(defs, ctx);
        let mut probe = Self::build(defs, ctx);
        let input = talking(ctx.rate, 1.6);
        let mut x = input.clone();
        let quiet = Ctx { background: false };
        for block in x.chunks_mut(512) {
            probe.process(block, &quiet);
        }
        let skip = (ctx.rate * 0.6) as usize;
        let rin = rms(&input[skip..]);
        let rout = rms(&x[skip..]);
        if rout > 1e-6 && rout.is_finite() {
            chain.trim = (rin / rout).clamp(0.25, 2.0);
        }
        chain
    }

    #[cfg(test)]
    pub fn empty(rate: u32) -> Self {
        Self { rate, blocks: Vec::new(), max_block: BuildCtx::MAX_BLOCK, trim: 1.0 }
    }

    pub fn process(&mut self, buf: &mut [f32], ctx: &Ctx) {
        for chunk in buf.chunks_mut(self.max_block) {
            for (b, on) in &mut self.blocks {
                if *on {
                    b.process(chunk, ctx);
                }
            }
            if self.trim != 1.0 {
                for x in chunk.iter_mut() {
                    *x *= self.trim;
                }
            }
        }
    }

    pub fn set_param(&mut self, block: usize, index: usize, value: f32) {
        if let Some((b, _)) = self.blocks.get_mut(block) {
            b.set_param(index, value);
        }
    }
}
