//! An AI voice in the chain: hands the voice to the AI helper and plays what comes back (see
//! `ai::bridge`). Only lock-free rings and atomics, so it is safe on the realtime thread. Until
//! the character comes back it stays quiet, so your own voice never slips through.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::{BuildCtx, Ctx, Effect};
use crate::ai::bridge::{self, Link, Stream};

pub struct Ai {
    /// None in a chain that is only measured: then the voice passes as it is.
    io: Option<(rtrb::Producer<f32>, rtrb::Consumer<f32>, Arc<Link>)>,
    playing: bool,
    /// Samples since the first block came back, while not playing yet.
    waited: usize,
}

impl Ai {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        if !ctx.live {
            return Self { io: None, playing: false, waited: 0 };
        }
        let size = (ctx.rate * 4.0) as usize;
        let (to_helper, input) = rtrb::RingBuffer::new(size);
        let (output, from_helper) = rtrb::RingBuffer::new(size);
        let link = Arc::new(Link::new(p[0] as usize, p[1], ctx.rate as u32));
        bridge::attach(Stream { link: link.clone(), input, output });
        Self { io: Some((to_helper, from_helper, link)), playing: false, waited: 0 }
    }
}

impl Drop for Ai {
    fn drop(&mut self) {
        if let Some((_, _, link)) = &self.io {
            link.alive.store(false, Ordering::Release);
        }
    }
}

impl Effect for Ai {
    fn process(&mut self, buf: &mut [f32], _ctx: &Ctx) {
        let Some((to_helper, from_helper, link)) = &mut self.io else { return };
        let n = to_helper.slots().min(buf.len());
        if let Ok(chunk) = to_helper.write_chunk_uninit(n) {
            chunk.fill_from_iter(buf[..n].iter().copied());
        }
        let block = link.block.load(Ordering::Relaxed);
        let cushion = link.cushion.load(Ordering::Relaxed).max(buf.len());
        let mut have = from_helper.slots();
        // After a stall the helper catches up with several blocks at once. Skip the oldest, so
        // the delay does not grow.
        if have > block.saturating_add(cushion.saturating_mul(2)) {
            let skip = have - block - cushion;
            if let Ok(chunk) = from_helper.read_chunk(skip) {
                chunk.commit_all();
                have -= skip;
            }
        }
        // Starts a cushion after a block came back, so the next one is in time. When it runs dry,
        // it waits like that again.
        if !self.playing && have > 0 {
            self.waited += buf.len();
            self.playing = self.waited >= cushion;
        }
        let n = if self.playing { have.min(buf.len()) } else { 0 };
        if let Ok(chunk) = from_helper.read_chunk(n) {
            let (a, b) = chunk.as_slices();
            buf[..a.len()].copy_from_slice(a);
            buf[a.len()..n].copy_from_slice(b);
            chunk.commit_all();
        }
        buf[n..].fill(0.0);
        if self.playing && n < buf.len() {
            self.playing = false;
            self.waited = 0;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some((_, _, link)) = &self.io else { return };
        match index {
            0 => link.model.store(value as usize, Ordering::Relaxed),
            1 => link.key.store(value.to_bits(), Ordering::Relaxed),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_measured_chain_lets_the_voice_pass() {
        let mut ai = Ai::new(&[0.0, 0.0], &BuildCtx::test());
        let mut buf = vec![0.5f32; 512];
        ai.process(&mut buf, &Ctx::default());
        assert!(buf.iter().all(|x| *x == 0.5));
    }

    /// An effect with rings but no bridge, and the producer that plays the helper.
    fn live(block: usize, cushion: usize) -> (Ai, rtrb::Producer<f32>) {
        let (to_helper, _) = rtrb::RingBuffer::new(48000);
        let (output, from_helper) = rtrb::RingBuffer::new(48000);
        let link = Arc::new(Link::new(0, 0.0, 48000));
        link.block.store(block, Ordering::Relaxed);
        link.cushion.store(cushion, Ordering::Relaxed);
        (Ai { io: Some((to_helper, from_helper, link)), playing: false, waited: 0 }, output)
    }

    fn answer(helper: &mut rtrb::Producer<f32>, n: usize) {
        for _ in 0..n {
            helper.push(1.0).unwrap();
        }
    }

    #[test]
    fn waits_a_cushion_after_the_first_block() {
        let (mut ai, mut helper) = live(4096, 1024);
        let mut buf = vec![0.0f32; 512];
        answer(&mut helper, 4096);
        ai.process(&mut buf, &Ctx::default());
        assert!(buf.iter().all(|x| *x == 0.0), "plays too early");
        ai.process(&mut buf, &Ctx::default());
        assert!(buf.iter().all(|x| *x == 1.0), "does not play after the cushion");
    }

    #[test]
    fn skips_what_piled_up() {
        let (mut ai, mut helper) = live(4096, 1024);
        let mut buf = vec![0.0f32; 512];
        answer(&mut helper, 3 * 4096);
        ai.process(&mut buf, &Ctx::default());
        let (_, from_helper, _) = ai.io.as_ref().unwrap();
        assert_eq!(from_helper.slots(), 4096 + 1024);
    }
}
