use anyhow::Result;
use std::time::Duration;
use tokio::time::sleep;

use crate::backend::AiBackend;
use crate::config::Config;
use crate::telegram::TelegramClient;
use crate::transcribe;

pub async fn run(cfg: Config, owner_id: i64) -> Result<()> {
    let tg = TelegramClient::new(&cfg.bot_token);
    let backend = crate::backend::from_name(&cfg.backend, &cfg)?;
    let mut offset: i64 = 0;
    let poll_ms = cfg.poll_interval_ms;

    // Pre-load whisper model so first audio message isn't slow
    if let Err(e) = transcribe::ensure_model().await {
        eprintln!("[phonehome] whisper model unavailable: {e}");
    }

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

                    // --- audio / voice message ---
                    let audio_file = msg.voice.as_ref().or(msg.audio.as_ref());
                    if let Some(af) = audio_file {
                        handle_audio(&tg, &*backend, msg.chat.id, msg.message_id, &af.file_id).await;
                        continue;
                    }

                    // --- text message ---
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

async fn handle_audio(tg: &TelegramClient, backend: &dyn AiBackend, chat_id: i64, msg_id: i64, file_id: &str) {
    let status_id = tg.send(chat_id, "🎤 Transcribing...").await.unwrap_or(0);

    let transcript = async {
        let file_path = tg.get_file_path(file_id).await?;
        let audio_bytes = tg.download_file(&file_path).await?;

        let tmp_in = format!("/tmp/ph-audio-{msg_id}.ogg");
        let tmp_wav = format!("/tmp/ph-audio-{msg_id}.wav");
        std::fs::write(&tmp_in, &audio_bytes)?;

        // Convert to 16 kHz mono WAV
        let status = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-i", &tmp_in, "-ar", "16000", "-ac", "1", "-f", "wav", &tmp_wav])
            .output().await?
            .status;
        let _ = std::fs::remove_file(&tmp_in);

        if !status.success() {
            anyhow::bail!("ffmpeg conversion failed");
        }

        let wav_path = std::path::PathBuf::from(&tmp_wav);
        let text = tokio::task::spawn_blocking(move || transcribe::transcribe(&wav_path))
            .await??;
        let _ = std::fs::remove_file(&tmp_wav);
        Ok::<String, anyhow::Error>(text)
    }.await;

    if status_id > 0 { let _ = tg.delete(chat_id, status_id).await; }

    let transcript = match transcript {
        Ok(t) if !t.is_empty() => t,
        Ok(_) => { let _ = tg.send(chat_id, "⚠️ Could not transcribe audio.").await; return; }
        Err(e) => { let _ = tg.send(chat_id, &format!("⚠️ Transcription error: {e}")).await; return; }
    };

    let _ = tg.send(chat_id, &format!("🎤 You said: \"{}\"", transcript)).await;

    let thinking_id = tg.send(chat_id, "...").await.unwrap_or(0);
    let output = run_prompt(backend, &transcript).await;
    if thinking_id > 0 { let _ = tg.delete(chat_id, thinking_id).await; }
    let reply = output.unwrap_or_else(|e| format!("error: {e}"));
    backend.save_session(&reply);
    let _ = tg.send(chat_id, &reply).await;
}

async fn run_prompt(backend: &dyn AiBackend, prompt: &str) -> Result<String> {
    backend.run(prompt).await
}
