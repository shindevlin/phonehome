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
    let vocab      = g.vocab.clone();
    let filters    = g.mel_filters.clone();
    let model      = model::Whisper::from_ggml(g);

    // Read audio
    let reader = hound::WavReader::open(wav_path)?;
    let samples: Vec<f32> = reader
        .into_samples::<i16>()
        .collect::<std::result::Result<Vec<i16>, _>>()?
        .into_iter()
        .map(|s| s as f32 / 32768.0)
        .collect();

    let audio_secs = (samples.len() as f32 / 16000.0).min(30.0);
    eprintln!("[whisper] {:.1}s audio", audio_secs);

    // Mel spectrogram → encoder
    let mel = mel::log_mel(&samples, &filters, n_mels, n_fft_bins);
    let enc = model.encode(&mel)?;
    let mut cache = model.init_kv_cache(&enc)?;
    decode_greedy(&model, &mut cache, &vocab, audio_secs)
}

fn tok(vocab: &[String], name: &str) -> Option<u32> {
    vocab.iter().position(|s| s == name).map(|i| i as u32)
}

fn decode_greedy(
    model: &model::Whisper,
    cache: &mut model::KVCache,
    vocab: &[String],
    audio_secs: f32,
) -> Result<String> {
    // Look up special tokens by name so we work with any Whisper variant
    let eot  = tok(vocab, "<|endoftext|>").unwrap_or(50256);
    let sot  = tok(vocab, "<|startoftranscript|>").unwrap_or(50257);
    let task = tok(vocab, "<|transcribe|>").unwrap_or(50359);
    let nots = tok(vocab, "<|notimestamps|>").unwrap_or(50363);

    // Cap at ~10 tokens/sec. EOT is the primary stop; this cuts hallucination on silence padding.
    let max_tokens = ((audio_secs * 10.0) as usize + 16).min(224);

    // Feed prompt tokens; logits from the last one start generation.
    let prompt = [sot, task, nots];
    let mut logits = Vec::new();
    for (pos, &t) in prompt.iter().enumerate() {
        logits = model.decode_step_cached(t, pos, cache)?;
    }

    let mut text = String::new();
    let mut pos = prompt.len();
    let mut generated: Vec<u32> = Vec::new();

    for _ in 0..max_tokens {
        // Suppress tokens seen in the last 64 generated tokens
        let suppress_start = generated.len().saturating_sub(64);
        let recent: std::collections::HashSet<u32> = generated[suppress_start..].iter().cloned().collect();

        // Allow EOT in the pool so the model can signal end-of-speech naturally.
        // Any token >= sot is a special token; allow only EOT through, stop on anything else.
        let next = logits.iter().enumerate()
            .filter(|&(i, _)| (i as u32) <= eot)
            .filter(|&(i, _)| !recent.contains(&(i as u32)))
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .or_else(|| logits.iter().enumerate()
                .filter(|&(i, _)| (i as u32) <= eot)
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap()))
            .map(|(i, _)| i as u32)
            .unwrap_or(eot);

        if next == eot { break; }

        // Hard stop: last 4 tokens identical
        if generated.len() >= 4 && generated[generated.len()-4..].iter().all(|&t| t == next) {
            break;
        }

        // Cycle detection: if the last 8 tokens reappear as a sequence, we're looping
        if generated.len() >= 16 {
            let tail = &generated[generated.len()-8..];
            if generated[..generated.len()-8].windows(8).any(|w| w == tail) {
                break;
            }
        }

        if let Some(word) = vocab.get(next as usize) {
            text.push_str(word);
        }

        generated.push(next);
        logits = model.decode_step_cached(next, pos, cache)?;
        pos += 1;
    }

    Ok(text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transcribe_tone() {
        let p = std::path::Path::new("/tmp/ph_test.wav");
        if !p.exists() { eprintln!("skip: no test wav"); return; }
        let result = transcribe(p);
        println!("transcribe result: {:?}", result);
        assert!(result.is_ok(), "transcribe panicked: {:?}", result);
    }
}
