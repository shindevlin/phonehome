use std::collections::HashMap;
use anyhow::{Context, Result};
use rayon::prelude::*;
use super::ops::*;
use super::ggml::{GgmlFile, HParams};

pub struct Whisper {
    pub hp: HParams,
    t: HashMap<String, Vec<f32>>,
}

/// Per-layer K/V cache for the decoder.
pub struct KVCache {
    /// Self-attention: accumulated [t, d] for K and V (grows each step).
    pub self_k: Vec<Vec<f32>>,
    pub self_v: Vec<Vec<f32>>,
    /// Cross-attention: pre-computed [t_enc, d] from encoder output (constant).
    pub cross_k: Vec<Vec<f32>>,
    pub cross_v: Vec<Vec<f32>>,
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

    /// Audio encoder: mel [n_mels, 3000] → [n_audio_ctx, n_audio_state]
    pub fn encode(&self, mel: &[f32]) -> Result<Vec<f32>> {
        let hp = &self.hp;
        let (d, t_in) = (hp.n_audio_state, 3000usize);

        let c1w = self.get("encoder.conv1.weight")?;
        let c1b = self.get("encoder.conv1.bias")?;
        let mut x = conv1d(mel, c1w, c1b, hp.n_mels, d, t_in, 3, 1, 1);
        gelu(&mut x);

        let c2w = self.get("encoder.conv2.weight")?;
        let c2b = self.get("encoder.conv2.bias")?;
        let t_enc = hp.n_audio_ctx;
        let mut x = conv1d(&x, c2w, c2b, d, d, t_in, 3, 2, 1);
        gelu(&mut x);

        // [d, t_enc] → [t_enc, d]
        let mut xt = vec![0f32; t_enc * d];
        for i in 0..t_enc {
            for j in 0..d {
                xt[i * d + j] = x[j * t_enc + i];
            }
        }

        let pe = self.get("encoder.positional_embedding")?;
        add_inplace(&mut xt, pe);

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

        let lnp_w = self.get("encoder.ln_post.weight")?;
        let lnp_b = self.get("encoder.ln_post.bias")?;
        Ok(layer_norm(&xt, lnp_w, lnp_b, t_enc, d))
    }

    /// Pre-compute cross-attention K/V for every decoder layer from encoder output.
    /// Call once before decoding; pass the result to each decode_step_cached call.
    pub fn init_kv_cache(&self, enc: &[f32]) -> Result<KVCache> {
        let hp = &self.hp;
        let d = hp.n_text_state;
        let t_enc = hp.n_audio_ctx;
        let n_layers = hp.n_text_layer;

        let mut cross_k = Vec::with_capacity(n_layers);
        let mut cross_v = Vec::with_capacity(n_layers);

        for layer in 0..n_layers {
            let pfx = format!("decoder.blocks.{}", layer);
            let kw = self.get(&format!("{}.cross_attn.key.weight", pfx))?;
            let vw = self.get(&format!("{}.cross_attn.value.weight", pfx))?;
            let vb = self.get(&format!("{}.cross_attn.value.bias", pfx))?;
            cross_k.push(matmul(enc, kw, t_enc, d, d));
            cross_v.push(linear(enc, vw, vb, t_enc, d, d));
        }

        Ok(KVCache {
            self_k: vec![Vec::new(); n_layers],
            self_v: vec![Vec::new(); n_layers],
            cross_k,
            cross_v,
        })
    }

    /// Process a single new token, update the KV cache, return logits [n_vocab].
    /// `pos` is the 0-based position of this token in the sequence.
    pub fn decode_step_cached(&self, token: u32, pos: usize, cache: &mut KVCache) -> Result<Vec<f32>> {
        let hp = &self.hp;
        let d = hp.n_text_state;
        let t_enc = hp.n_audio_ctx;
        let n_head = hp.n_text_head;

        let emb_w = self.get("decoder.token_embedding.weight")?;
        let pos_emb = self.get("decoder.positional_embedding")?;

        // Embed this token + positional encoding → x: [d]
        let tok = token as usize;
        let mut x: Vec<f32> = (0..d)
            .map(|j| emb_w[tok * d + j] + pos_emb[pos * d + j])
            .collect();

        for layer in 0..hp.n_text_layer {
            let pfx = format!("decoder.blocks.{}", layer);

            // --- self-attention ---
            let ln1_w = self.get(&format!("{}.attn_ln.weight", pfx))?;
            let ln1_b = self.get(&format!("{}.attn_ln.bias", pfx))?;
            let qw = self.get(&format!("{}.attn.query.weight", pfx))?;
            let qb = self.get(&format!("{}.attn.query.bias", pfx))?;
            let kw = self.get(&format!("{}.attn.key.weight", pfx))?;
            let vw = self.get(&format!("{}.attn.value.weight", pfx))?;
            let vb = self.get(&format!("{}.attn.value.bias", pfx))?;
            let ow = self.get(&format!("{}.attn.out.weight", pfx))?;
            let ob = self.get(&format!("{}.attn.out.bias", pfx))?;

            let xn = layer_norm1(&x, ln1_w, ln1_b, d);
            let q = linear1(&xn, qw, qb, d, d);
            let k_new = matmul1(&xn, kw, d, d);
            let v_new = linear1(&xn, vw, vb, d, d);

            // Append to self-attention cache
            cache.self_k[layer].extend_from_slice(&k_new);
            cache.self_v[layer].extend_from_slice(&v_new);
            let t_self = pos + 1;

            let sa_out = attn_single_query(&q, &cache.self_k[layer], &cache.self_v[layer], t_self, d, n_head);
            let sa_proj = linear1(&sa_out, ow, ob, d, d);
            add_inplace(&mut x, &sa_proj);

            // --- cross-attention (uses pre-computed K/V) ---
            let ln2_w = self.get(&format!("{}.cross_attn_ln.weight", pfx))?;
            let ln2_b = self.get(&format!("{}.cross_attn_ln.bias", pfx))?;
            let xqw = self.get(&format!("{}.cross_attn.query.weight", pfx))?;
            let xqb = self.get(&format!("{}.cross_attn.query.bias", pfx))?;
            let xow = self.get(&format!("{}.cross_attn.out.weight", pfx))?;
            let xob = self.get(&format!("{}.cross_attn.out.bias", pfx))?;

            let xn = layer_norm1(&x, ln2_w, ln2_b, d);
            let xq = linear1(&xn, xqw, xqb, d, d);
            let ca_out = attn_single_query(&xq, &cache.cross_k[layer], &cache.cross_v[layer], t_enc, d, n_head);
            let ca_proj = linear1(&ca_out, xow, xob, d, d);
            add_inplace(&mut x, &ca_proj);

            // --- MLP ---
            let ln3_w = self.get(&format!("{}.mlp_ln.weight", pfx))?;
            let ln3_b = self.get(&format!("{}.mlp_ln.bias", pfx))?;
            let m0w = self.get(&format!("{}.mlp.0.weight", pfx))?;
            let m0b = self.get(&format!("{}.mlp.0.bias", pfx))?;
            let m2w = self.get(&format!("{}.mlp.2.weight", pfx))?;
            let m2b = self.get(&format!("{}.mlp.2.bias", pfx))?;

            let xn = layer_norm1(&x, ln3_w, ln3_b, d);
            let mut h = linear1(&xn, m0w, m0b, d, 4 * d);
            gelu(&mut h);
            let ff = linear1(&h, m2w, m2b, 4 * d, d);
            add_inplace(&mut x, &ff);
        }

        let ln_w = self.get("decoder.ln.weight")?;
        let ln_b = self.get("decoder.ln.bias")?;
        let x = layer_norm1(&x, ln_w, ln_b, d);

        // Project to vocab in parallel (n_vocab=51864 rows × d cols)
        let mut logits = vec![0f32; hp.n_vocab];
        logits.par_chunks_mut(128).enumerate().for_each(|(ci, chunk)| {
            for (k, lv) in chunk.iter_mut().enumerate() {
                let v = ci * 128 + k;
                let base = v * d;
                let mut s = 0.0f32;
                for j in 0..d {
                    s += emb_w[base + j] * x[j];
                }
                *lv = s;
            }
        });
        Ok(logits)
    }
}
