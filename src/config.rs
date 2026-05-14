use anyhow::{Context, Result};
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub bot_token: String,
    pub chat_id: i64,
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default = "default_cwd")]
    pub cwd: String,
    #[serde(default)]
    pub session_file: Option<String>,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_max_send_chars")]
    pub max_send_chars: usize,
}

fn default_backend() -> String { "claude".to_string() }
fn default_cwd() -> String { home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default() }
fn default_poll_interval_ms() -> u64 { 1500 }
fn default_max_send_chars() -> usize { 20000 }

impl Config {
    pub fn config_path() -> PathBuf {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("/root"))
            .join(".phonehome")
            .join("config.toml")
    }

    pub fn session_file_path(&self) -> PathBuf {
        if let Some(ref p) = self.session_file {
            PathBuf::from(p)
        } else {
            home_dir()
                .unwrap_or_else(|| PathBuf::from("/root"))
                .join(".phonehome_session")
        }
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("config not found at {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&raw).context("invalid config.toml")?;

        // env overrides
        if let Ok(v) = std::env::var("PHONEHOME_BOT_TOKEN") { cfg.bot_token = v; }
        if let Ok(v) = std::env::var("PHONEHOME_CHAT_ID") {
            cfg.chat_id = v.parse().context("PHONEHOME_CHAT_ID must be an integer")?;
        }
        if let Ok(v) = std::env::var("PHONEHOME_BACKEND") { cfg.backend = v; }

        Ok(cfg)
    }

    pub fn read_session(&self) -> Option<String> {
        fs::read_to_string(self.session_file_path())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn write_session(&self, id: &str) {
        let _ = fs::write(self.session_file_path(), id);
    }
}
