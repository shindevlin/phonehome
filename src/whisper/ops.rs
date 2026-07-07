use rayon::prelude::*;

/// y = x @ W + b   where x:[m,k], W stored GGML column-major [k,n], b:[n]  →  y:[m,n]
/// GGML stores weights with ne0=k (input dim) fastest: W[l,j] = w[j*k + l].
/// Parallelized over output rows.
pub fn linear(x: &[f32], w: &[f32], b: &[f32], m: usize, k: usize, n: usize) -> Vec<f32> {
    let mut y = vec![0f32; m * n];
    y.par_chunks_mut(n).enumerate().for_each(|(i, row)| {
        for j in 0..n {
            let mut s = b[j];
            for l in 0..k {
                s += x[i * k + l] * w[j * k + l];
            }
            row[j] = s;
        }
    });
    y
}

/// Single-row linear: x:[k], w GGML column-major [k,n], b:[n] → y:[n]
pub fn linear1(x: &[f32], w: &[f32], b: &[f32], k: usize, n: usize) -> Vec<f32> {
    (0..n).map(|j| {
        let mut s = b[j];
        for l in 0..k {
            s += x[l] * w[j * k + l];
        }
        s
    }).collect()
}

/// Single-row matmul (no bias): x:[k], w GGML column-major [k,n] → y:[n]
pub fn matmul1(x: &[f32], w: &[f32], k: usize, n: usize) -> Vec<f32> {
    (0..n).map(|j| {
        let mut s = 0.0f32;
        for l in 0..k {
            s += x[l] * w[j * k + l];
        }
        s
    }).collect()
}

/// Like linear but no bias — parallelized over output rows.
pub fn matmul(x: &[f32], w: &[f32], m: usize, k: usize, n: usize) -> Vec<f32> {
    let mut y = vec![0f32; m * n];
    y.par_chunks_mut(n).enumerate().for_each(|(i, row)| {
        for j in 0..n {
            let mut s = 0.0f32;
            for l in 0..k {
                s += x[i * k + l] * w[j * k + l];
            }
            row[j] = s;
        }
    });
    y
}

pub fn layer_norm(x: &[f32], w: &[f32], b: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    const EPS: f32 = 1e-5;
    let mut y = vec![0f32; rows * cols];
    for i in 0..rows {
        let row = &x[i * cols..(i + 1) * cols];
        let mean = row.iter().sum::<f32>() / cols as f32;
        let var = row.iter().map(|&v| (v - mean).powi(2)).sum::<f32>() / cols as f32;
        let inv_std = 1.0 / (var + EPS).sqrt();
        for j in 0..cols {
            y[i * cols + j] = (row[j] - mean) * inv_std * w[j] + b[j];
        }
    }
    y
}

/// Layer norm for a single row.
pub fn layer_norm1(x: &[f32], w: &[f32], b: &[f32], d: usize) -> Vec<f32> {
    const EPS: f32 = 1e-5;
    let mean = x.iter().sum::<f32>() / d as f32;
    let var = x.iter().map(|&v| (v - mean).powi(2)).sum::<f32>() / d as f32;
    let inv_std = 1.0 / (var + EPS).sqrt();
    (0..d).map(|j| (x[j] - mean) * inv_std * w[j] + b[j]).collect()
}

pub fn gelu(x: &mut [f32]) {
    const C: f32 = 0.797_884_6; // sqrt(2/pi)
    for v in x.iter_mut() {
        let x_ = *v;
        *v = 0.5 * x_ * (1.0 + (C * (x_ + 0.044715 * x_.powi(3))).tanh());
    }
}

pub fn softmax_rows(x: &mut [f32], rows: usize, cols: usize) {
    for i in 0..rows {
        let row = &mut x[i * cols..(i + 1) * cols];
        let max = row.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let mut sum = 0.0f32;
        for v in row.iter_mut() {
            *v = (*v - max).exp();
            sum += *v;
        }
        for v in row.iter_mut() {
            *v /= sum;
        }
    }
}

/// 1D convolution: input [in_ch, t_in], weight [out_ch, in_ch, kernel], bias [out_ch]
/// → output [out_ch, t_out]. Parallelized over output channels.
pub fn conv1d(
    input: &[f32], weight: &[f32], bias: &[f32],
    in_ch: usize, out_ch: usize, t_in: usize,
    kernel: usize, stride: usize, pad: usize,
) -> Vec<f32> {
    let t_out = (t_in + 2 * pad - kernel) / stride + 1;
    let mut output = vec![0f32; out_ch * t_out];
    output.par_chunks_mut(t_out).enumerate().for_each(|(oc, row)| {
        for t in 0..t_out {
            let mut s = bias[oc];
            for ic in 0..in_ch {
                for k in 0..kernel {
                    let src = (t * stride + k) as isize - pad as isize;
                    if src >= 0 && (src as usize) < t_in {
                        let w = weight[oc * in_ch * kernel + ic * kernel + k];
                        let x = input[ic * t_in + src as usize];
                        s += w * x;
                    }
                }
            }
            row[t] = s;
        }
    });
    output
}

/// Multi-head attention (encoder self-attention, no mask).
/// x: [t, d], q/k/v weights: [d, d] (no bias for k), out_w: [d, d]
/// Returns [t, d]
pub fn mha(
    x: &[f32],
    q_w: &[f32], q_b: &[f32],
    k_w: &[f32],
    v_w: &[f32], v_b: &[f32],
    out_w: &[f32], out_b: &[f32],
    t: usize, d: usize, n_head: usize,
) -> Vec<f32> {
    let dh = d / n_head;
    let scale = 1.0 / (dh as f32).sqrt();

    let q = linear(x, q_w, q_b, t, d, d);
    let k = matmul(x, k_w, t, d, d);
    let v = linear(x, v_w, v_b, t, d, d);

    // Extract contiguous head slices for cache-friendly attention
    // q_heads[h]: [t, dh], k_heads[h]: [t, dh], v_heads[h]: [t, dh]
    let mut q_heads = vec![vec![0f32; t * dh]; n_head];
    let mut k_heads = vec![vec![0f32; t * dh]; n_head];
    let mut v_heads = vec![vec![0f32; t * dh]; n_head];
    for h in 0..n_head {
        for i in 0..t {
            q_heads[h][i * dh..i * dh + dh].copy_from_slice(&q[i * d + h * dh..i * d + h * dh + dh]);
            k_heads[h][i * dh..i * dh + dh].copy_from_slice(&k[i * d + h * dh..i * d + h * dh + dh]);
            v_heads[h][i * dh..i * dh + dh].copy_from_slice(&v[i * d + h * dh..i * d + h * dh + dh]);
        }
    }

    // Parallel over heads; contiguous slices → cache-friendly matmul
    let head_outs: Vec<Vec<f32>> = (0..n_head).into_par_iter().map(|h| {
        let qh = &q_heads[h]; // [t, dh]
        let kh = &k_heads[h]; // [t, dh]
        let vh = &v_heads[h]; // [t, dh]

        // scores[t,t] = qh @ kh^T * scale
        let mut scores = vec![0f32; t * t];
        for i in 0..t {
            for l in 0..dh {
                let qi = qh[i * dh + l] * scale;
                for j in 0..t {
                    scores[i * t + j] += qi * kh[j * dh + l];
                }
            }
        }
        softmax_rows(&mut scores, t, t);

        // out[t,dh] = scores @ vh
        let mut ho = vec![0f32; t * dh];
        for i in 0..t {
            for j in 0..t {
                let s = scores[i * t + j];
                for l in 0..dh {
                    ho[i * dh + l] += s * vh[j * dh + l];
                }
            }
        }
        ho
    }).collect();

    let mut out = vec![0f32; t * d];
    for h in 0..n_head {
        for i in 0..t {
            out[i * d + h * dh..i * d + h * dh + dh].copy_from_slice(&head_outs[h][i * dh..i * dh + dh]);
        }
    }

    linear(&out, out_w, out_b, t, d, d)
}

/// Cached single-query attention: q:[d], k_cache:[t,d], v_cache:[t,d] → out:[d]
/// Used for both self-attention (causal, attend to all t) and cross-attention.
pub fn attn_single_query(
    q: &[f32],
    k_cache: &[f32], v_cache: &[f32],
    t: usize, d: usize, n_head: usize,
) -> Vec<f32> {
    let dh = d / n_head;
    let scale = 1.0 / (dh as f32).sqrt();
    let mut out = vec![0f32; d];

    for h in 0..n_head {
        let mut scores = vec![0f32; t];
        for j in 0..t {
            let mut s = 0.0f32;
            for l in 0..dh {
                s += q[h * dh + l] * k_cache[j * d + h * dh + l];
            }
            scores[j] = s * scale;
        }
        // softmax
        let max = scores.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let mut sum = 0.0f32;
        for s in &mut scores { *s = (*s - max).exp(); sum += *s; }
        for s in &mut scores { *s /= sum; }

        for l in 0..dh {
            let mut s = 0.0f32;
            for j in 0..t {
                s += scores[j] * v_cache[j * d + h * dh + l];
            }
            out[h * dh + l] = s;
        }
    }
    out
}

pub fn add_inplace(a: &mut [f32], b: &[f32]) {
    for (x, y) in a.iter_mut().zip(b.iter()) { *x += y; }
}
