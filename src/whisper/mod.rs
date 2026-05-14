mod ggml;
mod mel;
mod model;
mod ops;

use anyhow::Result;
use dirs::home_dir;
use std::path::PathBuf;

const GGML_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";

pub fn model_path() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("/root"))
        .join(".phonehome")
        .join("ggml-base.en.bin")
}

pub async fn ensure_model() -> Result<()> {
    let path = model_path();
    if path.exists() {
        return Ok(());
    }
    println!("[whisper] downloading base.en model (~145 MB)...");
    let resp = reqwest::get(GGML_URL).await?;
    let bytes = resp.bytes().await?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, &bytes)?;
    println!("[whisper] model saved to {}", path.display());
    Ok(())
}

/// Transcribe a 16 kHz mono WAV file. Returns the recognized text.
pub fn transcribe(wav_path: &std::path::Path) -> Result<String> {
    // Load weights (single pass)
    let g = ggml::load(&model_path())?;
    let n_mels     = g.hparams.n_mels;
    let n_fft_bins = g.n_fft_bins;
    let n_audio_ctx = g.hparams.n_audio_ctx;
    let vocab      = g.vocab.clone();
    let filters    = g.mel_filters.clone();
    let model      = model::Whisper::from_ggml(g);

    // Read audio
    let reader = hound::WavReader::open(wav_path)?;
    let samples: Vec<f32> = reader
        .into_samples::<i16>()
        .filter_map(|s| s.ok())
        .map(|s| s as f32 / 32768.0)
        .collect();

    // Mel spectrogram
    let mel = mel::log_mel(&samples, &filters, n_mels, n_fft_bins);

    // Encode
    let enc = model.encode(&mel)?;

    // Decode (greedy)
    decode_greedy(&model, &enc, n_audio_ctx, &vocab)
}

fn decode_greedy(
    model: &model::Whisper,
    enc: &[f32],
    t_enc: usize,
    vocab: &[String],
) -> Result<String> {
    // Special token IDs for base.en
    const SOT: u32 = 50258;        // <|startoftranscript|>
    const TRANSCRIBE: u32 = 50359; // <|transcribe|>
    const NO_TIMESTAMPS: u32 = 50363;
    const EOT: u32 = 50257;        // <|endoftext|>
    const MAX_TOKENS: usize = 224;

    let mut tokens: Vec<u32> = vec![SOT, TRANSCRIBE, NO_TIMESTAMPS];
    let mut text = String::new();

    for _ in 0..MAX_TOKENS {
        let logits = model.decode_step(&tokens, enc, t_enc)?;

        // Suppress special/timestamp tokens
        let next = logits.iter().enumerate()
            .filter(|&(i, _)| i < 50257) // only real text tokens
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i as u32)
            .unwrap_or(EOT);

        if next == EOT { break; }
        tokens.push(next);

        if let Some(word) = vocab.get(next as usize) {
            text.push_str(word);
        }

        if tokens.len() >= MAX_TOKENS + 3 { break; }
    }

    Ok(text.trim().to_string())
}
