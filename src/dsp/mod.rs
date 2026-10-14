//! Everything that turns a voice into another voice. No allocations once built, so it is
//! safe to run on PipeWire's realtime thread.

pub mod ambience;
pub mod autotune;
pub mod biquad;
pub mod blocks;
pub mod chain;
pub mod dynamics;
pub mod modulation;
pub mod noise;
pub mod pitch;
pub mod space;
pub mod spectral;
pub mod tone;
pub mod util;
pub mod vocoder;
pub mod yin;

#[cfg(test)]
pub mod analysis;

/// Settings an effect is built with.
#[derive(Clone, Copy, Debug)]
pub struct BuildCtx {
    pub rate: f32,
    /// Smaller FFT frames: less delay, a bit less quality.
    pub low_latency: bool,
    /// Effects never get more samples than this per call.
    pub max_block: usize,
}

impl BuildCtx {
    pub const MAX_BLOCK: usize = 1024;

    pub fn new(rate: f32, low_latency: bool) -> Self {
        Self { rate, low_latency, max_block: Self::MAX_BLOCK }
    }

    #[cfg(test)]
    pub fn test() -> Self {
        Self { rate: 48000.0, low_latency: false, max_block: 48000 * 12 }
    }
}

/// What effects need to know while running.
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    /// Whether background sounds should play.
    pub background: bool,
}

impl Default for Ctx {
    fn default() -> Self {
        Self { background: true }
    }
}

pub trait Effect: Send {
    /// Processes a mono block in place.
    fn process(&mut self, buf: &mut [f32], ctx: &Ctx);
    /// Sets parameter `index` (the order of the block's parameter list).
    fn set_param(&mut self, index: usize, value: f32);
}
