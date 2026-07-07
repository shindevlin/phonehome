use dirs::home_dir;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

pub fn path() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("/root"))
        .join(".phonehome")
        .join("chat.ndjson")
}

/// Append one message to ~/.phonehome/chat.ndjson.
/// `from` is "user" or "claude".
pub fn append(from: &str, text: &str) {
    let ts = chrono::Local::now().format("%H:%M").to_string();
    let entry = serde_json::json!({
        "ts": ts,
        "from": from,
        "text": text,
    });
    let line = serde_json::to_string(&entry).unwrap_or_default();
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path())
    {
        let _ = writeln!(f, "{line}");
    }
}
