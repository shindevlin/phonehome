use anyhow::Result;
use dirs::home_dir;
use std::fs;
use std::path::PathBuf;

pub fn run(bot_token: String, chat_id: i64, backend: String) -> Result<()> {
    let dir = home_dir()
        .unwrap_or_else(|| PathBuf::from("/root"))
        .join(".phonehome");
    fs::create_dir_all(&dir)?;

    let config_path = dir.join("config.toml");
    if config_path.exists() {
        println!("config already exists at {}", config_path.display());
        println!("edit it directly or delete and re-run init.");
        return Ok(());
    }

    let content = format!(
        r#"# phonehome config
bot_token = "{bot_token}"
chat_id = {chat_id}
backend = "{backend}"
# cwd = "/home/you"        # working directory for AI commands (default: $HOME)
# session_file = ""        # path to session file (default: ~/.phonehome_session)
# poll_interval_ms = 1500  # Telegram polling interval
# max_send_chars = 20000   # max chars to send per response
"#
    );

    fs::write(&config_path, content)?;
    println!("created {}", config_path.display());
    println!("run 'phonehome bot --owner-id <your-telegram-user-id>' to start.");
    Ok(())
}
