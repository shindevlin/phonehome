use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use dirs::home_dir;
use ed25519_dalek::{Signer, SigningKey};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;

#[derive(Deserialize)]
struct KeyFile {
    private_key_hex: String,
    public_key_hex: Option<String>,
}

pub struct BtcpcBackend {
    public_key_hex: String,
    signing_key: SigningKey,
    model: String,
    client: reqwest::Client,
}

impl BtcpcBackend {
    pub fn new(cfg: &Config) -> Result<Self> {
        let key_path = home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/root"))
            .join(".btcpc")
            .join("key.json");

        let raw = std::fs::read_to_string(&key_path)
            .with_context(|| format!("btcpc key not found at {} — run `btcpc init` first", key_path.display()))?;

        let kf: KeyFile = serde_json::from_str(&raw)
            .context("invalid ~/.btcpc/key.json — expected {\"public_key\":\"...\",\"secret_key\":\"...\"}")?;

        let secret_bytes = decode_key_bytes(&kf.private_key_hex)
            .context("could not decode private_key_hex in ~/.btcpc/key.json")?;

        let seed: [u8; 32] = secret_bytes[..32]
            .try_into()
            .context("private_key_hex must be at least 32 bytes")?;

        let signing_key = SigningKey::from_bytes(&seed);
        let public_key_hex = kf.public_key_hex
            .unwrap_or_else(|| hex::encode(signing_key.verifying_key().to_bytes()));

        Ok(Self {
            public_key_hex,
            signing_key,
            model: cfg.backends.btcpc_model.clone(),
            client: reqwest::Client::new(),
        })
    }

    /// Bearer token = "<pubkey_hex>.<sig_hex>"
    /// Challenge signed: "phonehome:<timestamp_rounded_to_30s>"
    fn bearer_token(&self) -> String {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let window = ts - (ts % 30);
        let challenge = format!("phonehome:{window}");

        let sig = self.signing_key.sign(challenge.as_bytes());
        let sig_hex = hex::encode(sig.to_bytes());

        format!("{}.{sig_hex}", self.public_key_hex)
    }

    async fn call(&self, prompt: &str) -> Result<String> {
        let body = json!({
            "model": self.model,
            "messages": [{ "role": "user", "content": prompt }]
        });

        let resp = self.client
            .post("https://btcpc.net/v1/chat/completions")
            .bearer_auth(self.bearer_token())
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            anyhow::bail!("btcpc API error {status}: {text}");
        }

        let json: Value = serde_json::from_str(&text)
            .with_context(|| format!("invalid JSON from btcpc: {text}"))?;

        json["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .with_context(|| format!("unexpected btcpc response shape: {text}"))
    }
}

fn decode_key_bytes(s: &str) -> Result<Vec<u8>> {
    if let Ok(b) = hex::decode(s) { return Ok(b); }
    if let Ok(b) = general_purpose::STANDARD.decode(s) { return Ok(b); }
    if let Ok(b) = general_purpose::URL_SAFE_NO_PAD.decode(s) { return Ok(b); }
    anyhow::bail!("key must be hex or base64 encoded")
}

#[async_trait]
impl super::AiBackend for BtcpcBackend {
    async fn run(&self, prompt: &str) -> Result<String> {
        self.call(prompt).await
    }

    async fn run_in(&self, prompt: &str, _cwd: &str) -> Result<String> {
        self.call(prompt).await
    }
}
