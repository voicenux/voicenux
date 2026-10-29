//! Whisper itself, built like candle-transformers' `whisper/model.rs` (MIT or Apache-2.0), from
//! the same weights. Only the decoder is different: it keeps the keys and values of what it has
//! read. So the list of names it should expect is read once, and each new word takes one step.
//! candle's decoder reads everything again for every word, ten times as slow with that list.

use candle_core::{D, Device, IndexOp, Module, Result, Tensor};
use candle_nn::{Conv1d, Conv1dConfig, Embedding, LayerNorm, Linear, VarBuilder, linear, linear_no_bias};
use candle_transformers::models::whisper::Config;

fn layer_norm(size: usize, vb: VarBuilder) -> Result<LayerNorm> {
    candle_nn::layer_norm(size, 1e-5, vb)
}

struct Attention {
    query: Linear,
    key: Linear,
    value: Linear,
    out: Linear,
    heads: usize,
}

impl Attention {
    fn load(n: usize, heads: usize, vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            query: linear(n, n, vb.pp("q_proj"))?,
            key: linear_no_bias(n, n, vb.pp("k_proj"))?,
            value: linear(n, n, vb.pp("v_proj"))?,
            out: linear(n, n, vb.pp("out_proj"))?,
            heads,
        })
    }

    /// What `x` takes from the keys `k` and values `v`.
    fn attend(&self, x: &Tensor, k: &Tensor, v: &Tensor, mask: Option<&Tensor>) -> Result<Tensor> {
        let q = self.query.forward(x)?;
        let (batch, len, n) = q.dims3()?;
        let size = n / self.heads;
        let scale = (size as f64).powf(-0.25);
        let heads = |t: &Tensor| -> Result<Tensor> {
            let (batch, len, _) = t.dims3()?;
            t.reshape((batch, len, self.heads, size))?.transpose(1, 2)
        };
        let q = (heads(&q)? * scale)?;
        let k = (heads(k)?.transpose(2, 3)? * scale)?;
        let mut w = q.matmul(&k)?;
        if let Some(mask) = mask {
            w = w.broadcast_add(mask)?;
        }
        let w = candle_nn::ops::softmax_last_dim(&w)?;
        w.matmul(&heads(v)?.contiguous()?)?.transpose(1, 2)?.reshape((batch, len, n))?.apply(&self.out)
    }
}

struct Block {
    attn: Attention,
    attn_ln: LayerNorm,
    /// To the sound, in the decoder.
    cross: Option<(Attention, LayerNorm)>,
    fc1: Linear,
    fc2: Linear,
    mlp_ln: LayerNorm,
}

impl Block {
    fn load(n: usize, heads: usize, cross: bool, vb: VarBuilder) -> Result<Self> {
        let cross = match cross {
            true => Some((
                Attention::load(n, heads, vb.pp("encoder_attn"))?,
                layer_norm(n, vb.pp("encoder_attn_layer_norm"))?,
            )),
            false => None,
        };
        Ok(Self {
            attn: Attention::load(n, heads, vb.pp("self_attn"))?,
            attn_ln: layer_norm(n, vb.pp("self_attn_layer_norm"))?,
            cross,
            fc1: linear(n, 4 * n, vb.pp("fc1"))?,
            fc2: linear(4 * n, n, vb.pp("fc2"))?,
            mlp_ln: layer_norm(n, vb.pp("final_layer_norm"))?,
        })
    }

    fn mlp(&self, x: &Tensor) -> Result<Tensor> {
        x + self.fc2.forward(&self.fc1.forward(&self.mlp_ln.forward(x)?)?.gelu()?)?
    }
}

/// The positions of the encoder, as Whisper makes them.
fn sinusoids(length: usize, channels: usize, device: &Device) -> Result<Tensor> {
    let step = 10000f32.ln() / (channels / 2 - 1) as f32;
    let inv: Vec<f32> = (0..channels / 2).map(|i| (i as f32 * -step).exp()).collect();
    let inv = Tensor::new(inv.as_slice(), device)?.unsqueeze(0)?;
    let at = Tensor::arange(0, length as u32, device)?.to_dtype(candle_core::DType::F32)?.unsqueeze(1)?;
    let shape = (length, channels / 2);
    let scaled = (at.broadcast_as(shape)? * inv.broadcast_as(shape)?)?;
    Tensor::cat(&[scaled.sin()?, scaled.cos()?], 1)
}

pub struct Whisper {
    conv1: Conv1d,
    conv2: Conv1d,
    audio_positions: Tensor,
    audio: Vec<Block>,
    audio_ln: LayerNorm,
    embed: Embedding,
    positions: Tensor,
    text: Vec<Block>,
    text_ln: LayerNorm,
    /// The keys and values of the sound, for each block of the decoder. See `hear`.
    sound: Vec<(Tensor, Tensor)>,
}

/// What the decoder has read so far: the keys and values of each block, and how many tokens.
#[derive(Clone)]
pub struct Read {
    blocks: Vec<(Tensor, Tensor)>,
    len: usize,
}

impl Whisper {
    pub fn load(vb: &VarBuilder, c: &Config) -> Result<Self> {
        let (n, enc, dec) = (c.d_model, vb.pp("model.encoder"), vb.pp("model.decoder"));
        let conv = |inputs, stride, vb: VarBuilder| -> Result<Conv1d> {
            let config = Conv1dConfig { padding: 1, stride, ..Default::default() };
            Ok(Conv1d::new(vb.get((n, inputs, 3), "weight")?, Some(vb.get(n, "bias")?), config))
        };
        let blocks = |count, heads, cross, vb: &VarBuilder| -> Result<Vec<Block>> {
            (0..count).map(|i| Block::load(n, heads, cross, vb.pp(format!("layers.{i}")))).collect()
        };
        Ok(Self {
            conv1: conv(c.num_mel_bins, 1, enc.pp("conv1"))?,
            conv2: conv(n, 2, enc.pp("conv2"))?,
            audio_positions: sinusoids(c.max_source_positions, n, vb.device())?,
            audio: blocks(c.encoder_layers, c.encoder_attention_heads, false, &enc)?,
            audio_ln: layer_norm(n, enc.pp("layer_norm"))?,
            embed: candle_nn::embedding(c.vocab_size, n, dec.pp("embed_tokens"))?,
            positions: dec.get((c.max_target_positions, n), "embed_positions.weight")?,
            text: blocks(c.decoder_layers, c.decoder_attention_heads, true, &dec)?,
            text_ln: layer_norm(n, dec.pp("layer_norm"))?,
            sound: Vec::new(),
        })
    }

    /// Listens to `mel` (1, bins, frames), for all that the decoder reads after.
    pub fn hear(&mut self, mel: &Tensor) -> Result<()> {
        let x = self.conv2.forward(&self.conv1.forward(mel)?.gelu()?)?.gelu()?.transpose(1, 2)?;
        let mut x = x.broadcast_add(&self.audio_positions.narrow(0, 0, x.dim(1)?)?)?;
        for b in &self.audio {
            let h = b.attn_ln.forward(&x)?;
            let (k, v) = (b.attn.key.forward(&h)?, b.attn.value.forward(&h)?);
            x = b.mlp(&(&x + b.attn.attend(&h, &k, &v, None)?)?)?;
        }
        let x = self.audio_ln.forward(&x)?;
        let mut sound = Vec::with_capacity(self.text.len());
        for b in &self.text {
            let (attn, _) = b.cross.as_ref().expect("decoder blocks look at the sound");
            sound.push((attn.key.forward(&x)?, attn.value.forward(&x)?));
        }
        self.sound = sound;
        Ok(())
    }

    /// Reads `tokens` after what it read before (None: from the start). Gives what it has read
    /// then, and its guess for the next token: logits for each one.
    pub fn read(&self, before: Option<&Read>, tokens: &[u32]) -> Result<(Read, Tensor)> {
        let device = self.positions.device();
        let (past, len) = (before.map_or(0, |r| r.len), tokens.len());
        let x = self.embed.forward(&Tensor::new(tokens, device)?.unsqueeze(0)?)?;
        let mut x = x.broadcast_add(&self.positions.narrow(0, past, len)?)?;
        // Each new token sees what came before it, not after.
        let mask = (len > 1)
            .then(|| {
                let m: Vec<f32> = (0..len)
                    .flat_map(|i| (0..past + len).map(move |j| if j > past + i { f32::NEG_INFINITY } else { 0.0 }))
                    .collect();
                Tensor::from_vec(m, (len, past + len), device)
            })
            .transpose()?;
        let mut read = Vec::with_capacity(self.text.len());
        for (i, b) in self.text.iter().enumerate() {
            let h = b.attn_ln.forward(&x)?;
            let (mut k, mut v) = (b.attn.key.forward(&h)?, b.attn.value.forward(&h)?);
            if let Some(before) = before {
                k = Tensor::cat(&[&before.blocks[i].0, &k], 1)?;
                v = Tensor::cat(&[&before.blocks[i].1, &v], 1)?;
            }
            x = (&x + b.attn.attend(&h, &k, &v, mask.as_ref())?)?;
            let (attn, ln) = b.cross.as_ref().expect("decoder blocks look at the sound");
            let (sk, sv) = &self.sound[i];
            x = (&x + attn.attend(&ln.forward(&x)?, sk, sv, None)?)?;
            x = b.mlp(&x)?;
            read.push((k, v));
        }
        let last = self.text_ln.forward(&x.i((.., len - 1..))?)?;
        let logits = last.matmul(&self.embed.embeddings().t()?.unsqueeze(0)?)?.flatten_all()?;
        Ok((Read { blocks: read, len: past + len }, logits))
    }
}

/// The token with the highest logit, of those `mask` leaves.
pub fn best(logits: &Tensor, mask: &Tensor) -> Result<u32> {
    logits.broadcast_add(mask)?.argmax(D::Minus1)?.to_scalar::<u32>()
}
