use anyhow::Result;
use std::time::Duration;
use tokio::time::sleep;

use crate::backend::AiBackend;
use crate::config::Config;
use crate::telegram::TelegramClient;

pub async fn run(cfg: Config, owner_id: i64) -> Result<()> {
    let tg = TelegramClient::new(&cfg.bot_token);
    let backend = crate::backend::from_name(&cfg.backend, &cfg)?;
    let mut offset: i64 = 0;
    let poll_ms = cfg.poll_interval_ms;

    println!("[phonehome] bot live — backend: {}", cfg.backend);

    loop {
        match tg.get_updates(offset).await {
            Ok(updates) => {
                for update in updates {
                    offset = offset.max(update.update_id + 1);

                    let msg = match update.message {
                        Some(m) => m,
                        None => continue,
                    };

                    let from_id = msg.from.as_ref().map(|u| u.id).unwrap_or(0);
                    if from_id != owner_id { continue; }

                    let text = match msg.text {
                        Some(t) => t.trim().to_string(),
                        None => continue,
                    };

                    if text == "/start" {
                        let _ = tg.send(msg.chat.id, "phonehome is live. Send me a message.").await;
                        continue;
                    }

                    let thinking_id = tg.send(msg.chat.id, "...").await.unwrap_or(0);

                    let output = run_prompt(&*backend, &text).await;
                    if thinking_id > 0 {
                        let _ = tg.delete(msg.chat.id, thinking_id).await;
                    }

                    let reply = output.unwrap_or_else(|e| format!("error: {e}"));
                    backend.save_session(&reply);
                    let _ = tg.send(msg.chat.id, &reply).await;
                }
            }
            Err(e) => eprintln!("[phonehome] poll error: {e}"),
        }
        sleep(Duration::from_millis(poll_ms)).await;
    }
}

async fn run_prompt(backend: &dyn AiBackend, prompt: &str) -> Result<String> {
    backend.run(prompt).await
}
