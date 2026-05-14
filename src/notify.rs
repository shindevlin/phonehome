use anyhow::Result;
use dirs::home_dir;
use glob::glob;
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use tokio::time::{sleep, Duration, Instant};

use crate::config::Config;
use crate::telegram::TelegramClient;

pub async fn run(cfg: Config) -> Result<()> {
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

    // Stop fires before Claude writes the final text to JSONL.
    // Wait until the file grows (new data written), then read the final entry.
    let initial_size = std::fs::metadata(&session_path).map(|m| m.len()).unwrap_or(0);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        sleep(Duration::from_millis(300)).await;
        let current_size = std::fs::metadata(&session_path).map(|m| m.len()).unwrap_or(0);
        if current_size > initial_size || Instant::now() >= deadline {
            break;
        }
    }

    let last_text = match extract_last_assistant(&session_path)? {
        Some(t) => t,
        None => return Ok(()),
    };

    let log_path = home_dir()
        .unwrap_or_else(|| PathBuf::from("/root"))
        .join(".phonehome")
        .join("conversation.log");

    append_log(&log_path, "Claude", &last_text);

    let tg = TelegramClient::new(&cfg.bot_token);
    tg.send(cfg.chat_id, &last_text).await?;
    Ok(())
}

fn append_log(path: &std::path::Path, role: &str, text: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(f, "[{ts}] {role}: {text}\n---");
    }
}

fn extract_last_assistant(path: &std::path::Path) -> Result<Option<String>> {
    let mut f = File::open(path)?;
    let size = f.seek(SeekFrom::End(0))?;
    f.seek(SeekFrom::Start(size.saturating_sub(524288)))?;

    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    let tail = String::from_utf8_lossy(&buf);

    // Scan backwards: return the single last text entry, stop at a human user message.
    // The polling in run() ensures we wait until the final text is written before reading.
    for line in tail.lines().rev() {
        if let Ok(entry) = serde_json::from_str::<Value>(line) {
            match entry["type"].as_str() {
                Some("assistant") => {
                    if let Some(parts) = entry["message"]["content"].as_array() {
                        let text: String = parts.iter()
                            .filter(|p| p["type"].as_str() == Some("text"))
                            .filter_map(|p| p["text"].as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !text.is_empty() {
                            return Ok(Some(text));
                        }
                    }
                }
                Some("user") => {
                    let content = &entry["message"]["content"];
                    let is_human = content.is_string() || content.as_array().map(|parts| {
                        parts.iter().any(|p| matches!(p["type"].as_str(), Some("text") | Some("image_url")))
                        && !parts.iter().any(|p| p["type"].as_str() == Some("tool_result"))
                    }).unwrap_or(false);
                    if is_human {
                        return Ok(None);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(None)
}
