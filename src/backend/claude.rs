use anyhow::Result;
use async_trait::async_trait;
use std::process::Stdio;
use tokio::process::Command;
use tokio::io::AsyncReadExt;

use crate::config::Config;

pub struct ClaudeBackend {
    cwd: String,
    cfg: Config,
}

impl ClaudeBackend {
    pub fn new(cfg: &Config) -> Self {
        Self { cwd: cfg.cwd.clone(), cfg: cfg.clone() }
    }

    fn session_args(&self) -> Vec<String> {
        match self.cfg.read_session() {
            Some(id) => vec!["--resume".to_string(), id],
            None => vec!["--continue".to_string()],
        }
    }
}

#[async_trait]
impl super::AiBackend for ClaudeBackend {
    async fn run(&self, prompt: &str) -> Result<String> {
        let mut args = self.session_args();
        args.extend(["-p".to_string(), prompt.to_string(), "--print".to_string()]);

        let mut child = Command::new("claude")
            .args(&args)
            .current_dir(&self.cwd)
            .env("PHONEHOME_BOT", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut stdout_buf = String::new();
        let mut stderr_buf = String::new();

        if let Some(mut out) = child.stdout.take() {
            out.read_to_string(&mut stdout_buf).await?;
        }
        if let Some(mut err) = child.stderr.take() {
            err.read_to_string(&mut stderr_buf).await?;
        }

        child.wait().await?;

        let combined = format!("{stdout_buf}{stderr_buf}");
        Ok(combined.trim().to_string())
    }

    fn save_session(&self, output: &str) {
        let re = regex::Regex::new(r"(?i)Session[- ]ID[:\s]+([a-f0-9-]{36})").unwrap();
        if let Some(cap) = re.captures(output) {
            self.cfg.write_session(&cap[1]);
        }
    }
}
