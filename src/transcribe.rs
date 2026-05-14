use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn model_path() -> PathBuf {
    crate::whisper::model_path()
}

pub async fn ensure_model() -> Result<()> {
    crate::whisper::ensure_model().await
}

pub fn transcribe(wav_path: &Path) -> Result<String> {
    crate::whisper::transcribe(wav_path)
}
