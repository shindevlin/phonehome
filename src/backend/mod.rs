use anyhow::Result;
use async_trait::async_trait;

pub mod claude;

#[async_trait]
pub trait AiBackend: Send + Sync {
    /// Run a prompt and return the full output string.
    async fn run(&self, prompt: &str) -> Result<String>;
    /// Called after run() completes — extract and persist session ID from output.
    fn save_session(&self, output: &str);
}

pub fn from_name(name: &str, cfg: &crate::config::Config) -> Result<Box<dyn AiBackend>> {
    match name {
        "claude" => Ok(Box::new(claude::ClaudeBackend::new(cfg))),
        other => anyhow::bail!("unknown backend: {other}. Supported: claude"),
    }
}
