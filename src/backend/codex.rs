use anyhow::Result;
use async_trait::async_trait;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::config::Config;

pub struct CodexBackend {
    default_cwd: String,
}

impl CodexBackend {
    pub fn new(cfg: &Config) -> Self {
        Self { default_cwd: cfg.cwd.clone() }
    }

    async fn exec(&self, prompt: &str, cwd: &str) -> Result<String> {
        let mut child = Command::new("codex")
            .args(["exec", prompt, "--ask-for-approval", "never", "--sandbox", "workspace-write"])
            .current_dir(cwd)
            .env("PHONEHOME_BOT", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut o) = child.stdout.take() { o.read_to_string(&mut stdout).await?; }
        if let Some(mut e) = child.stderr.take() { e.read_to_string(&mut stderr).await?; }
        child.wait().await?;

        let combined = format!("{stdout}{stderr}").trim().to_string();
        if combined.is_empty() {
            anyhow::bail!("codex returned no output — is it installed? (npm install -g @openai/codex)");
        }
        Ok(combined)
    }
}

#[async_trait]
impl super::AiBackend for CodexBackend {
    async fn run(&self, prompt: &str) -> Result<String> {
        self.exec(prompt, &self.default_cwd).await
    }

    async fn run_in(&self, prompt: &str, cwd: &str) -> Result<String> {
        self.exec(prompt, cwd).await
    }
}
