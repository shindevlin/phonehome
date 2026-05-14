use anyhow::Result;
use dirs::home_dir;
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::PathBuf;

use crate::config::Config;
use crate::telegram::TelegramClient;

pub async fn run(cfg: Config) -> Result<()> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;

    let data: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);

    let msg = data["message"].as_str()
        .or_else(|| data["prompt"].as_str())
        .or_else(|| data["user_message"].as_str())
        .unwrap_or("")
        .trim();

    if msg.is_empty() {
        return Ok(());
    }

    let log_path = home_dir()
        .unwrap_or_else(|| PathBuf::from("/root"))
        .join(".phonehome")
        .join("conversation.log");

    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(f, "[{ts}] You: {msg}\n---");
    }

    let tg = TelegramClient::new(&cfg.bot_token);
    tg.send(cfg.chat_id, &format!("You: {msg}")).await?;
    Ok(())
}
