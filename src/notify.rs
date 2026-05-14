use anyhow::Result;
use dirs::home_dir;
use glob::glob;
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use crate::config::Config;
use crate::telegram::TelegramClient;

pub async fn run(cfg: Config) -> Result<()> {
    // read stdin JSON from Claude Code Stop hook
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;

    let data: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let session_id = data["session_id"].as_str().unwrap_or("").trim().to_string();
    if session_id.is_empty() {
        return Ok(());
    }

    cfg.write_session(&session_id);

    let projects_dir = home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/root"))
        .join(".claude")
        .join("projects");

    let pattern = projects_dir.join(format!("*/{session_id}.jsonl"));
    let pattern_str = pattern.to_string_lossy();

    let session_path = glob(&pattern_str)?.next().and_then(|r| r.ok());
    let session_path = match session_path {
        Some(p) => p,
        None => return Ok(()),
    };

    let last_text = extract_last_assistant(&session_path)?;
    let last_text = match last_text {
        Some(t) => t,
        None => return Ok(()),
    };

    let tg = TelegramClient::new(&cfg.bot_token);
    tg.send(cfg.chat_id, &last_text).await?;
    Ok(())
}

fn extract_last_assistant(path: &std::path::Path) -> Result<Option<String>> {
    let mut f = File::open(path)?;
    let size = f.seek(SeekFrom::End(0))?;
    f.seek(SeekFrom::Start(size.saturating_sub(65536)))?;

    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    let tail = String::from_utf8_lossy(&buf);

    for line in tail.lines().rev() {
        if let Ok(entry) = serde_json::from_str::<Value>(line) {
            if entry["type"].as_str() == Some("assistant") {
                let content = &entry["message"]["content"];
                if let Some(parts) = content.as_array() {
                    let texts: Vec<&str> = parts.iter()
                        .filter_map(|p| {
                            if p["type"].as_str() == Some("text") {
                                p["text"].as_str().filter(|s| !s.trim().is_empty())
                            } else {
                                None
                            }
                        })
                        .collect();
                    if !texts.is_empty() {
                        return Ok(Some(texts.join("\n").trim().to_string()));
                    }
                }
            }
        }
    }
    Ok(None)
}
