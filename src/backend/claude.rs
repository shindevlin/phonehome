use anyhow::Result;
use async_trait::async_trait;
use dirs::home_dir;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::config::Config;

pub struct ClaudeBackend {
    default_cwd: String,
}

impl ClaudeBackend {
    pub fn new(cfg: &Config) -> Self {
        Self { default_cwd: cfg.cwd.clone() }
    }

    /// ~/.phonehome/sessions/<sanitized_cwd>.claude.session
    fn session_path(cwd: &str) -> PathBuf {
        let safe: String = cwd.chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
            .collect();
        home_dir()
            .unwrap_or_else(|| PathBuf::from("/root"))
            .join(".phonehome")
            .join("sessions")
            .join(format!("{safe}.claude.session"))
    }

    fn read_session(path: &Path) -> Option<String> {
        std::fs::read_to_string(path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn write_session(path: &Path, id: &str) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, id);
    }

    fn extract_session_id(output: &str) -> Option<String> {
        let re = regex::Regex::new(r"(?i)Session[- ]ID[:\s]+([a-f0-9-]{36})").unwrap();
        re.captures(output).map(|c| c[1].to_string())
    }

    async fn exec(&self, args: Vec<String>, prompt: &str, cwd: &str) -> Result<String> {
        let mut full_args = args;
        full_args.extend(["-p".to_string(), prompt.to_string(), "--print".to_string()]);

        let mut child = Command::new("claude")
            .args(&full_args)
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

        Ok(format!("{stdout}{stderr}").trim().to_string())
    }

    async fn run_with_session(&self, prompt: &str, cwd: &str) -> Result<String> {
        let session_path = Self::session_path(cwd);
        let session_args = match Self::read_session(&session_path) {
            Some(id) => vec!["--resume".to_string(), id],
            None => vec![],
        };

        let output = self.exec(session_args, prompt, cwd).await?;

        if output.contains("No conversation found with session ID") {
            Self::write_session(&session_path, "");
            let fresh = self.exec(vec![], prompt, cwd).await?;
            if let Some(id) = Self::extract_session_id(&fresh) {
                Self::write_session(&session_path, &id);
            }
            return Ok(fresh);
        }

        if let Some(id) = Self::extract_session_id(&output) {
            Self::write_session(&session_path, &id);
        }

        Ok(output)
    }
}

#[async_trait]
impl super::AiBackend for ClaudeBackend {
    async fn run(&self, prompt: &str) -> Result<String> {
        self.run_with_session(prompt, &self.default_cwd.clone()).await
    }

    async fn run_in(&self, prompt: &str, cwd: &str) -> Result<String> {
        self.run_with_session(prompt, cwd).await
    }
}
