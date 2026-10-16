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
}

impl Ai {
    pub fn new(p: &[f32], ctx: &BuildCtx) -> Self {
        if !ctx.live {
            return Self { io: None, playing: false };
        }
        let size = (ctx.rate * 4.0) as usize;
        let (to_helper, input) = rtrb::RingBuffer::new(size);
        let (output, from_helper) = rtrb::RingBuffer::new(size);
        let link = Arc::new(Link::new(p[0] as usize, p[1], ctx.rate as u32));
        bridge::attach(Stream { link: link.clone(), input, output });
        Self { io: Some((to_helper, from_helper, link)), playing: false }
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
        let have = from_helper.slots();
        // Starts once a little is waiting, and waits again when it runs dry.
        if !self.playing && have >= link.prime.load(Ordering::Relaxed).max(buf.len()) {
            self.playing = true;
        }
        let n = if self.playing { have.min(buf.len()) } else { 0 };
        if let Ok(chunk) = from_helper.read_chunk(n) {
            let (a, b) = chunk.as_slices();
            buf[..a.len()].copy_from_slice(a);
            buf[a.len()..n].copy_from_slice(b);
            chunk.commit_all();
        }
        buf[n..].fill(0.0);
        if n < buf.len() {
            self.playing = false;
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
}
