use anyhow::{Context, Result};
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackendsConfig {
    pub openrouter_api_key: Option<String>,
    #[serde(default = "default_openrouter_model")]
    pub openrouter_model: String,
    #[serde(default = "default_btcpc_model")]
    pub btcpc_model: String,
}

fn default_openrouter_model() -> String { "openai/gpt-4o".to_string() }
fn default_btcpc_model() -> String { "auto".to_string() }

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
    #[serde(default)]
    pub watch_dirs: Vec<String>,
    #[serde(default)]
    pub backends: BackendsConfig,
    /// Directory to scan when resolving bare repo names (e.g. /phonehome brutus11).
    /// Defaults to ~/repos if it exists, otherwise ~/projects, otherwise ~.
    pub repos_dir: Option<String>,
}

impl Config {
    /// Returns true if the given hook cwd should be forwarded to Telegram.
    pub fn should_watch(&self, hook_cwd: &str) -> bool {
        if self.watch_dirs.is_empty() {
            hook_cwd == self.cwd
        } else {
            self.watch_dirs.iter().any(|d| d == hook_cwd)
        }
    }
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

    fn active_cwd_path() -> PathBuf {
        home_dir()
            .unwrap_or_else(|| PathBuf::from("/root"))
            .join(".phonehome")
            .join("active_cwd")
    }

    pub fn read_active_cwd(&self) -> Option<String> {
        fs::read_to_string(Self::active_cwd_path())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn write_active_cwd(&self, cwd: &str) {
        let _ = fs::write(Self::active_cwd_path(), cwd);
    }

    /// Effective working directory: active_cwd (set by last Ludicrous session) → cwd from config.
    pub fn effective_cwd(&self) -> String {
        self.read_active_cwd().unwrap_or_else(|| self.cwd.clone())
    }

    /// Directory to scan for bare repo name resolution.
    /// Uses repos_dir from config if set, otherwise tries ~/repos, ~/projects, ~/code, ~.
    pub fn repos_dir(&self) -> String {
        if let Some(ref d) = self.repos_dir {
            return d.clone();
        }
        let home = home_dir().unwrap_or_else(|| PathBuf::from("/root"));
        for candidate in &["repos", "projects", "code"] {
            let p = home.join(candidate);
            if p.is_dir() {
                return p.to_string_lossy().to_string();
            }
        }
        home.to_string_lossy().to_string()
    }
}
