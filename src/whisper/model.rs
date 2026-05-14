use std::collections::HashMap;
use anyhow::{Context, Result};
use super::ops::*;
use super::ggml::{GgmlFile, HParams};

pub struct Whisper {
    pub hp: HParams,
    t: HashMap<String, Vec<f32>>,
}

impl Whisper {
    pub fn from_ggml(g: GgmlFile) -> Self {
        Self { hp: g.hparams, t: g.tensors }
    }

    fn get(&self, name: &str) -> Result<&[f32]> {
        self.t.get(name)
            .map(|v| v.as_slice())
            .with_context(|| format!("tensor '{}' not found", name))
    }

    fn get_or_zeros(&self, name: &str, n: usize) -> Vec<f32> {
        self.t.get(name).cloned().unwrap_or_else(|| vec![0f32; n])
    }

    /// Audio encoder: mel [n_mels, 3000] → [n_audio_ctx, n_audio_state]
    pub fn encode(&self, mel: &[f32]) -> Result<Vec<f32>> {
        let hp = &self.hp;
        let (d, t_in) = (hp.n_audio_state, 3000usize);

        // conv1: [d, n_mels, 3] weight, [d] bias
        let c1w = self.get("encoder.conv1.weight")?;
        let c1b = self.get("encoder.conv1.bias")?;
        let mut x = conv1d(mel, c1w, c1b, hp.n_mels, d, t_in, 3, 1, 1);
        gelu(&mut x);

        // conv2: stride 2 → t_enc = 1500
        let c2w = self.get("encoder.conv2.weight")?;
        let c2b = self.get("encoder.conv2.bias")?;
        let t_enc = hp.n_audio_ctx;
        let mut x = conv1d(&x, c2w, c2b, d, d, t_in, 3, 2, 1);
        gelu(&mut x);

        // transpose: [d, t_enc] → [t_enc, d]
        let mut xt = vec![0f32; t_enc * d];
        for i in 0..t_enc {
            for j in 0..d {
                xt[i * d + j] = x[j * t_enc + i];
            }
        }

        // add positional embedding [t_enc, d]
        let pe = self.get("encoder.positional_embedding")?;
        add_inplace(&mut xt, pe);

        // transformer layers
        for layer in 0..hp.n_audio_layer {
            let pfx = format!("encoder.blocks.{}", layer);

            let ln1_w = self.get(&format!("{}.attn_ln.weight", pfx))?;
            let ln1_b = self.get(&format!("{}.attn_ln.bias", pfx))?;
            let qw = self.get(&format!("{}.attn.query.weight", pfx))?;
            let qb = self.get(&format!("{}.attn.query.bias", pfx))?;
            let kw = self.get(&format!("{}.attn.key.weight", pfx))?;
            let vw = self.get(&format!("{}.attn.value.weight", pfx))?;
            let vb = self.get(&format!("{}.attn.value.bias", pfx))?;
            let ow = self.get(&format!("{}.attn.out.weight", pfx))?;
            let ob = self.get(&format!("{}.attn.out.bias", pfx))?;

            let normed = layer_norm(&xt, ln1_w, ln1_b, t_enc, d);
            let attn = mha(&normed, qw, qb, kw, vw, vb, ow, ob, t_enc, d, hp.n_audio_head);
            add_inplace(&mut xt, &attn);

            let ln2_w = self.get(&format!("{}.mlp_ln.weight", pfx))?;
            let ln2_b = self.get(&format!("{}.mlp_ln.bias", pfx))?;
            let m0w = self.get(&format!("{}.mlp.0.weight", pfx))?;
            let m0b = self.get(&format!("{}.mlp.0.bias", pfx))?;
            let m2w = self.get(&format!("{}.mlp.2.weight", pfx))?;
            let m2b = self.get(&format!("{}.mlp.2.bias", pfx))?;

            let normed = layer_norm(&xt, ln2_w, ln2_b, t_enc, d);
            let mut h = linear(&normed, m0w, m0b, t_enc, d, 4 * d);
            gelu(&mut h);
            let ff = linear(&h, m2w, m2b, t_enc, 4 * d, d);
            add_inplace(&mut xt, &ff);
        }

        // post-encoder layernorm
        let lnp_w = self.get("encoder.ln_post.weight")?;
        let lnp_b = self.get("encoder.ln_post.bias")?;
        Ok(layer_norm(&xt, lnp_w, lnp_b, t_enc, d))
    }

    /// One decoder forward pass over the full token sequence so far.
    /// tokens: current sequence (grows each step)
    /// enc: encoder output [t_enc, d]
    /// Returns logits [n_vocab] for the next token.
    pub fn decode_step(&self, tokens: &[u32], enc: &[f32], t_enc: usize) -> Result<Vec<f32>> {
        let hp = &self.hp;
        let d = hp.n_text_state;
        let t = tokens.len();

        let emb_w = self.get("decoder.token_embedding.weight")?; // [n_vocab, d]
        let pos_emb = self.get("decoder.positional_embedding")?;  // [n_text_ctx, d]

        // token + positional embeddings
        let mut x = vec![0f32; t * d];
        for (i, &tok) in tokens.iter().enumerate() {
            let tok = tok as usize;
            for j in 0..d {
                x[i * d + j] = emb_w[tok * d + j] + pos_emb[i * d + j];
            }
        }

        for layer in 0..hp.n_text_layer {
            let pfx = format!("decoder.blocks.{}", layer);

            // self-attention
            let ln1_w = self.get(&format!("{}.attn_ln.weight", pfx))?;
            let ln1_b = self.get(&format!("{}.attn_ln.bias", pfx))?;
            let qw = self.get(&format!("{}.attn.query.weight", pfx))?;
            let qb = self.get(&format!("{}.attn.query.bias", pfx))?;
            let kw = self.get(&format!("{}.attn.key.weight", pfx))?;
            let vw = self.get(&format!("{}.attn.value.weight", pfx))?;
            let vb = self.get(&format!("{}.attn.value.bias", pfx))?;
            let ow = self.get(&format!("{}.attn.out.weight", pfx))?;
            let ob = self.get(&format!("{}.attn.out.bias", pfx))?;

            let normed = layer_norm(&x, ln1_w, ln1_b, t, d);
            let sa = causal_mha(&normed, qw, qb, kw, vw, vb, ow, ob, t, d, hp.n_text_head);
            add_inplace(&mut x, &sa);

            // cross-attention
            let ln2_w = self.get(&format!("{}.cross_attn_ln.weight", pfx))?;
            let ln2_b = self.get(&format!("{}.cross_attn_ln.bias", pfx))?;
            let xqw = self.get(&format!("{}.cross_attn.query.weight", pfx))?;
            let xqb = self.get(&format!("{}.cross_attn.query.bias", pfx))?;
            let xkw = self.get(&format!("{}.cross_attn.key.weight", pfx))?;
            let xvw = self.get(&format!("{}.cross_attn.value.weight", pfx))?;
            let xvb = self.get(&format!("{}.cross_attn.value.bias", pfx))?;
            let xow = self.get(&format!("{}.cross_attn.out.weight", pfx))?;
            let xob = self.get(&format!("{}.cross_attn.out.bias", pfx))?;

            let normed = layer_norm(&x, ln2_w, ln2_b, t, d);
            let ca = cross_attn(&normed, enc, xqw, xqb, xkw, xvw, xvb, xow, xob,
                                t, t_enc, d, hp.n_text_head);
            add_inplace(&mut x, &ca);

            // MLP
            let ln3_w = self.get(&format!("{}.mlp_ln.weight", pfx))?;
            let ln3_b = self.get(&format!("{}.mlp_ln.bias", pfx))?;
            let m0w = self.get(&format!("{}.mlp.0.weight", pfx))?;
            let m0b = self.get(&format!("{}.mlp.0.bias", pfx))?;
            let m2w = self.get(&format!("{}.mlp.2.weight", pfx))?;
            let m2b = self.get(&format!("{}.mlp.2.bias", pfx))?;

            let normed = layer_norm(&x, ln3_w, ln3_b, t, d);
            let mut h = linear(&normed, m0w, m0b, t, d, 4 * d);
            gelu(&mut h);
            let ff = linear(&h, m2w, m2b, t, 4 * d, d);
            add_inplace(&mut x, &ff);
        }

        // final layernorm
        let ln_w = self.get("decoder.ln.weight")?;
        let ln_b = self.get("decoder.ln.bias")?;
        let x = layer_norm(&x, ln_w, ln_b, t, d);

        // project last token to vocab: logits = emb_w @ last_hidden
        // emb_w: [n_vocab, d], last: [d] → logits: [n_vocab]
        let last = &x[(t - 1) * d..t * d];
        let mut logits = vec![0f32; hp.n_vocab];
        for v in 0..hp.n_vocab {
            let mut s = 0.0f32;
            for j in 0..d {
                s += emb_w[v * d + j] * last[j];
            }
            logits[v] = s;
        }
        Ok(logits)
    }
}
