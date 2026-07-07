use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

pub mod btcpc;
pub mod claude;
pub mod codex;
pub mod gemini;
pub mod openrouter;

#[async_trait]
pub trait AiBackend: Send + Sync {
    async fn run(&self, prompt: &str) -> Result<String>;
    async fn run_in(&self, prompt: &str, cwd: &str) -> Result<String>;
}

pub const KNOWN_BACKENDS: &[&str] = &["claude", "gemini", "codex", "openrouter", "btcpc"];

/// Build a map of every backend that can be instantiated with the given config.
/// Backends with missing credentials are silently skipped.
pub fn build_all(cfg: &crate::config::Config) -> HashMap<String, Arc<dyn AiBackend>> {
    let mut map: HashMap<String, Arc<dyn AiBackend>> = HashMap::new();

    map.insert("claude".into(), Arc::new(claude::ClaudeBackend::new(cfg)));
    map.insert("gemini".into(), Arc::new(gemini::GeminiBackend::new(cfg)));
    map.insert("codex".into(), Arc::new(codex::CodexBackend::new(cfg)));

    match openrouter::OpenRouterBackend::new(cfg) {
        Ok(b) => { map.insert("openrouter".into(), Arc::new(b)); }
        Err(e) => eprintln!("[phonehome] openrouter backend unavailable: {e}"),
    }

    match btcpc::BtcpcBackend::new(cfg) {
        Ok(b) => { map.insert("btcpc".into(), Arc::new(b)); }
        Err(e) => eprintln!("[phonehome] btcpc backend unavailable: {e}"),
    }

    map
}

pub fn from_name(name: &str, cfg: &crate::config::Config) -> Result<Box<dyn AiBackend>> {
    match name {
        "claude" => Ok(Box::new(claude::ClaudeBackend::new(cfg))),
        "gemini" => Ok(Box::new(gemini::GeminiBackend::new(cfg))),
        "codex" => Ok(Box::new(codex::CodexBackend::new(cfg))),
        "openrouter" => Ok(Box::new(openrouter::OpenRouterBackend::new(cfg)?)),
        "btcpc" => Ok(Box::new(btcpc::BtcpcBackend::new(cfg)?)),
        other => anyhow::bail!("unknown backend: {other}. Supported: {}", KNOWN_BACKENDS.join(", ")),
    }
}
