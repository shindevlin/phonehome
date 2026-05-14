/// y = x @ W + b   where x:[m,k], W:[k,n], b:[n]  →  y:[m,n]
pub fn linear(x: &[f32], w: &[f32], b: &[f32], m: usize, k: usize, n: usize) -> Vec<f32> {
    let mut y = vec![0f32; m * n];
    for i in 0..m {
        for j in 0..n {
            let mut s = b[j];
            for l in 0..k {
                s += x[i * k + l] * w[l * n + j];
            }
            y[i * n + j] = s;
        }
    }
    y
}

/// Like linear but no bias
pub fn matmul(x: &[f32], w: &[f32], m: usize, k: usize, n: usize) -> Vec<f32> {
    let mut y = vec![0f32; m * n];
    for i in 0..m {
        for j in 0..n {
            let mut s = 0.0f32;
            for l in 0..k {
                s += x[i * k + l] * w[l * n + j];
            }
            y[i * n + j] = s;
        }
    }
    y
}

/// x:[m,k], w_T:[n,k] (already transposed storage)  → y:[m,n]
pub fn matmul_bt(x: &[f32], wt: &[f32], m: usize, k: usize, n: usize) -> Vec<f32> {
    let mut y = vec![0f32; m * n];
    for i in 0..m {
        for j in 0..n {
            let mut s = 0.0f32;
            for l in 0..k {
                s += x[i * k + l] * wt[j * k + l];
            }
            y[i * n + j] = s;
        }
    }
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

/// Causal masked softmax (sets upper triangle to -inf before softmax)
pub fn softmax_causal(x: &mut [f32], rows: usize, cols: usize) {
    for i in 0..rows {
        for j in (i + 1)..cols {
            x[i * cols + j] = f32::NEG_INFINITY;
        }
        let row = &mut x[i * cols..(i + 1) * cols];
        let max = row.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let mut sum = 0.0f32;
        for v in row.iter_mut() {
            *v = if v.is_finite() { (*v - max).exp() } else { 0.0 };
            sum += *v;
        }
        for v in row.iter_mut() {
            *v /= sum;
        }
    }
}

/// 1D convolution: input [in_ch, t_in], weight [out_ch, in_ch, kernel], bias [out_ch]
/// → output [out_ch, t_out], t_out = (t_in + 2*pad - kernel)/stride + 1
pub fn conv1d(
    input: &[f32], weight: &[f32], bias: &[f32],
    in_ch: usize, out_ch: usize, t_in: usize,
    kernel: usize, stride: usize, pad: usize,
) -> Vec<f32> {
    let t_out = (t_in + 2 * pad - kernel) / stride + 1;
    let mut output = vec![0f32; out_ch * t_out];
    for oc in 0..out_ch {
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
            output[oc * t_out + t] = s;
        }
    }
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

    let mut out = vec![0f32; t * d];

    for h in 0..n_head {
        // scores [t, t]
        let mut scores = vec![0f32; t * t];
        for i in 0..t {
            for j in 0..t {
                let mut s = 0.0f32;
                for l in 0..dh {
                    s += q[i * d + h * dh + l] * k[j * d + h * dh + l];
                }
                scores[i * t + j] = s * scale;
            }
        }
        softmax_rows(&mut scores, t, t);

        // weighted sum of v
        for i in 0..t {
            for l in 0..dh {
                let mut s = 0.0f32;
                for j in 0..t {
                    s += scores[i * t + j] * v[j * d + h * dh + l];
                }
                out[i * d + h * dh + l] = s;
            }
        }
    }

    // output projection: out = out @ out_w + out_b
    linear(&out, out_w, out_b, t, d, d)
}

/// Cross-attention: query from x, key/value from enc.
/// x: [t_dec, d], enc: [t_enc, d]
pub fn cross_attn(
    x: &[f32], enc: &[f32],
    q_w: &[f32], q_b: &[f32],
    k_w: &[f32],
    v_w: &[f32], v_b: &[f32],
    out_w: &[f32], out_b: &[f32],
    t_dec: usize, t_enc: usize, d: usize, n_head: usize,
) -> Vec<f32> {
    let dh = d / n_head;
    let scale = 1.0 / (dh as f32).sqrt();

    let q = linear(x, q_w, q_b, t_dec, d, d);
    let k = matmul(enc, k_w, t_enc, d, d);
    let v = linear(enc, v_w, v_b, t_enc, d, d);

    let mut out = vec![0f32; t_dec * d];

    for h in 0..n_head {
        let mut scores = vec![0f32; t_dec * t_enc];
        for i in 0..t_dec {
            for j in 0..t_enc {
                let mut s = 0.0f32;
                for l in 0..dh {
                    s += q[i * d + h * dh + l] * k[j * d + h * dh + l];
                }
                scores[i * t_enc + j] = s * scale;
            }
        }
        softmax_rows(&mut scores, t_dec, t_enc);

        for i in 0..t_dec {
            for l in 0..dh {
                let mut s = 0.0f32;
                for j in 0..t_enc {
                    s += scores[i * t_enc + j] * v[j * d + h * dh + l];
                }
                out[i * d + h * dh + l] = s;
            }
        }
    }

    linear(&out, out_w, out_b, t_dec, d, d)
}

/// Causal self-attention for decoder (token sequence grows step by step).
/// x: [t, d] (full sequence so far)
pub fn causal_mha(
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

    let mut out = vec![0f32; t * d];

    for h in 0..n_head {
        let mut scores = vec![0f32; t * t];
        for i in 0..t {
            for j in 0..=i {
                let mut s = 0.0f32;
                for l in 0..dh {
                    s += q[i * d + h * dh + l] * k[j * d + h * dh + l];
                }
                scores[i * t + j] = s * scale;
            }
            // upper triangle stays 0 → after softmax over 0..=i it's ignored
        }
        // softmax over valid positions only
        for i in 0..t {
            let row = &mut scores[i * t..i * t + i + 1];
            let max = row.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            let mut sum = 0.0f32;
            for v in row.iter_mut() { *v = (*v - max).exp(); sum += *v; }
            for v in row.iter_mut() { *v /= sum; }
        }

        for i in 0..t {
            for l in 0..dh {
                let mut s = 0.0f32;
                for j in 0..=i {
                    s += scores[i * t + j] * v[j * d + h * dh + l];
                }
                out[i * d + h * dh + l] = s;
            }
        }
    }

    linear(&out, out_w, out_b, t, d, d)
}

pub fn add_inplace(a: &mut [f32], b: &[f32]) {
    for (x, y) in a.iter_mut().zip(b.iter()) { *x += y; }
}
