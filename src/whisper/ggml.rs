use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use anyhow::{bail, Result};

pub const MAGIC: u32 = 0x67676d6c;

#[derive(Debug, Clone)]
pub struct HParams {
    pub n_vocab: usize,
    pub n_audio_ctx: usize,
    pub n_audio_state: usize,
    pub n_audio_head: usize,
    pub n_audio_layer: usize,
    pub n_text_ctx: usize,
    pub n_text_state: usize,
    pub n_text_head: usize,
    pub n_text_layer: usize,
    pub n_mels: usize,
}

pub struct GgmlFile {
    pub hparams: HParams,
    /// mel filter bank [n_mel, n_fft_bins]
    pub mel_filters: Vec<f32>,
    pub n_fft_bins: usize,
    pub vocab: Vec<String>,
    pub tensors: HashMap<String, Vec<f32>>,
}

pub fn load(path: &std::path::Path) -> Result<GgmlFile> {
    let data = std::fs::read(path)?;
    let mut cur = 0usize;

    macro_rules! read_u32 {
        () => {{
            let v = u32::from_le_bytes(data[cur..cur+4].try_into()?);
            cur += 4;
            v
        }};
    }
    macro_rules! read_i32 {
        () => {{
            let v = i32::from_le_bytes(data[cur..cur+4].try_into()?);
            cur += 4;
            v as usize
        }};
    }

    // magic
    let magic = read_u32!();
    if magic != MAGIC {
        bail!("bad GGML magic: {:#x}", magic);
    }

    // hparams
    let n_vocab       = read_i32!();
    let n_audio_ctx   = read_i32!();
    let n_audio_state = read_i32!();
    let n_audio_head  = read_i32!();
    let n_audio_layer = read_i32!();
    let n_text_ctx    = read_i32!();
    let n_text_state  = read_i32!();
    let n_text_head   = read_i32!();
    let n_text_layer  = read_i32!();
    let n_mels        = read_i32!();
    let _ftype        = read_i32!();

    let hparams = HParams {
        n_vocab, n_audio_ctx, n_audio_state, n_audio_head, n_audio_layer,
        n_text_ctx, n_text_state, n_text_head, n_text_layer, n_mels,
    };

    // mel filters
    let n_mel  = read_i32!();
    let n_fft  = read_i32!(); // 201 = N_FFT/2+1 where N_FFT=400
    let nf = n_mel * n_fft;
    let mel_filters: Vec<f32> = (0..nf)
        .map(|_| { let v = f32::from_le_bytes(data[cur..cur+4].try_into().unwrap()); cur += 4; v })
        .collect();

    // vocab
    let n_vocab_file = read_i32!();
    let mut vocab = Vec::with_capacity(n_vocab_file);
    for _ in 0..n_vocab_file {
        let len = read_u32!() as usize;
        let s = String::from_utf8_lossy(&data[cur..cur+len]).into_owned();
        cur += len;
        vocab.push(s);
    }

    // tensors
    let mut tensors: HashMap<String, Vec<f32>> = HashMap::new();
    while cur + 12 <= data.len() {
        let n_dims  = read_i32!();
        let name_len = read_i32!();
        let ftype   = read_i32!();

        let mut shape = vec![1usize; n_dims.max(1)];
        if cur + n_dims.saturating_mul(4) > data.len() { break; }
        for i in 0..n_dims {
            shape[i] = read_i32!();
        }

        if cur + name_len > data.len() { break; }
        let name = String::from_utf8_lossy(&data[cur..cur+name_len]).into_owned();
        cur += name_len;

        let nelements: usize = if n_dims == 0 { 0 } else { shape.iter().product() };
        let bytes_per = match ftype { 1 => 2usize, _ => 4usize };
        let nbytes = nelements * bytes_per;

        if cur + nbytes > data.len() {
            eprintln!("[ggml] tensor '{}' shape {:?} ftype {} at offset {}: needs {} bytes, {} remain — stopping",
                name, shape, ftype, cur, nbytes, data.len().saturating_sub(cur));
            break;
        }

        let floats: Vec<f32> = match ftype {
            0 => { // f32
                (0..nelements).map(|_| {
                    let x = f32::from_le_bytes(data[cur..cur+4].try_into().unwrap());
                    cur += 4; x
                }).collect()
            }
            1 => { // f16
                (0..nelements).map(|_| {
                    let raw = u16::from_le_bytes(data[cur..cur+2].try_into().unwrap());
                    cur += 2;
                    half::f16::from_bits(raw).to_f32()
                }).collect()
            }
            _ => bail!("unsupported ftype {}", ftype),
        };

        if !name.is_empty() {
            tensors.insert(name, floats);
        }
    }

    Ok(GgmlFile { hparams, mel_filters, n_fft_bins: n_fft, vocab, tensors })
}
