use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::config::Config;

pub struct OpenRouterBackend {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl OpenRouterBackend {
    pub fn new(cfg: &Config) -> Result<Self> {
        let api_key = cfg.backends.openrouter_api_key.clone()
            .context("openrouter backend requires backends.openrouter_api_key in config.toml")?;
        Ok(Self {
            api_key,
            model: cfg.backends.openrouter_model.clone(),
            client: reqwest::Client::new(),
        })
    }

    async fn call(&self, prompt: &str) -> Result<String> {
        let body = json!({
            "model": self.model,
            "messages": [{ "role": "user", "content": prompt }]
        });

        let resp = self.client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", "https://github.com/phonehome")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            anyhow::bail!("OpenRouter error {status}: {text}");
        }

        let json: Value = serde_json::from_str(&text)
            .with_context(|| format!("invalid JSON from OpenRouter: {text}"))?;

        json["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .with_context(|| format!("unexpected OpenRouter response shape: {text}"))
    }
}

#[async_trait]
impl super::AiBackend for OpenRouterBackend {
    async fn run(&self, prompt: &str) -> Result<String> {
        self.call(prompt).await
    }

    async fn run_in(&self, prompt: &str, _cwd: &str) -> Result<String> {
        self.call(prompt).await
    }
}
