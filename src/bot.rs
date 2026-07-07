use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

use crate::backend::AiBackend;
use crate::chat_log;
use crate::config::Config;
use crate::routes;
use crate::telegram::TelegramClient;
use crate::transcribe;

pub async fn run(cfg: Config, owner_id: i64) -> Result<()> {
    let tg = Arc::new(TelegramClient::new(&cfg.bot_token));
    let backends: Arc<HashMap<String, Arc<dyn AiBackend>>> =
        Arc::new(crate::backend::build_all(&cfg));
    let default_backend_name = cfg.backend.clone();
    let mut offset: i64 = 0;
    let poll_ms = cfg.poll_interval_ms;
    // Pending /phonehome confirmations: (chat_id, thread_id) → resolved_path
    let mut pending_confirms: HashMap<(i64, Option<i64>), String> = HashMap::new();

    // Pre-load whisper model so first audio message isn't slow
    if let Err(e) = transcribe::ensure_model().await {
        eprintln!("[phonehome] whisper model unavailable: {e}");
    }

    let available: Vec<String> = backends.keys().cloned().collect();
    println!("[phonehome] bot live — backends available: {}", available.join(", "));

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

                    let tg2 = Arc::clone(&tg);

                    // --- audio / voice message ---
                    let audio_file = msg.voice.as_ref().or(msg.audio.as_ref());
                    if let Some(af) = audio_file {
                        let file_id = af.file_id.clone();
                        let chat_id = msg.chat.id;
                        let thread_id = msg.message_thread_id;
                        let reply_thread = if msg.chat.id != from_id { thread_id } else { None };
                        let msg_id = msg.message_id;
                        let default_backend_for_audio = default_backend_name.clone();
                        let backends_for_audio = Arc::clone(&backends);
                        eprintln!("[bot] audio from={} chat={} thread={:?}", from_id, chat_id, thread_id);
                        tokio::spawn(async move {
                            let b = backends_for_audio.get(&default_backend_for_audio).cloned();
                            if let Some(backend) = b {
                                handle_audio(&*tg2, &*backend, chat_id, reply_thread, msg_id, &file_id).await;
                            } else {
                                let _ = tg2.send_to(chat_id, reply_thread, "no backend available for audio").await;
                            }
                        });
                        continue;
                    }

                    // --- text message ---
                    let text = match msg.text {
                        Some(t) => t.trim().to_string(),
                        None => continue,
                    };
                    let chat_id = msg.chat.id;
                    eprintln!("[bot] text from={} chat={} text={:?}", from_id, chat_id, &text[..text.len().min(40)]);

                    // Log every incoming user message to the chat panel log
                    if !text.starts_with('/') {
                        chat_log::append("user", &text);
                    }

                    if text == "/start" {
                        let _ = tg.send(chat_id, "phonehome is live. Send me a message.").await;
                        continue;
                    }

                    // yes/no confirmation for pending /phonehome bare-name lookup
                    {
                        let thread_id = msg.message_thread_id;
                        let reply_thread = if msg.chat.id != from_id { thread_id } else { None };
                        let key = (chat_id, reply_thread);
                        let lc = text.to_lowercase();
                        if (lc == "yes" || lc == "y") && pending_confirms.contains_key(&key) {
                            let repo_path = pending_confirms.remove(&key).unwrap();
                            let route = routes::Route {
                                cwd: repo_path.clone(),
                                chat_id,
                                thread_id: reply_thread,
                                label: String::new(),
                                backend: None,
                            };
                            let reply = match routes::upsert(route) {
                                Ok(()) => format!("Mapped `{}` → this topic.", repo_path),
                                Err(e) => format!("Failed to save route: {e}"),
                            };
                            let _ = tg.send_to(chat_id, reply_thread, &reply).await;
                            continue;
                        }
                        if (lc == "no" || lc == "n") && pending_confirms.remove(&key).is_some() {
                            let _ = tg.send_to(chat_id, reply_thread, "Cancelled.").await;
                            continue;
                        }
                    }

                    // /phonehome REPO_PATH https://t.me/c/XXXXXXXX/Y  (private chat)
                    // /phonehome REPO_PATH                             (inside a group topic — auto-detects thread)
                    // /phonehome BARE_NAME                             (bare name — resolves from known roots)
                    if text.starts_with("/phonehome") {
                        let thread_id = msg.message_thread_id;
                        let reply_thread = if msg.chat.id != from_id { thread_id } else { None };
                        let group_chat_id = if msg.chat.id != from_id { Some(msg.chat.id) } else { None };
                        let arg = text["/phonehome".len()..].trim();

                        // Bare name (no slash, no URL) — try to resolve to a real path
                        if !arg.is_empty() && !arg.starts_with('/') && !arg.starts_with("https://") {
                            let all_routes = routes::load();
                            let resolved = resolve_repo_name(arg, &cfg.repos_dir(), &all_routes);
                            let reply = match resolved {
                                Some(path) => {
                                    let key = (chat_id, reply_thread);
                                    pending_confirms.insert(key, path.clone());
                                    format!("Do you mean `/phonehome {path}`?\nReply `yes` to confirm or `no` to cancel.")
                                }
                                None => format!("No repo found matching `{arg}`. Use the full path instead."),
                            };
                            let _ = tg.send_to(chat_id, reply_thread, &reply).await;
                            continue;
                        }

                        let reply = handle_phonehome_cmd(&text, &cfg, group_chat_id, thread_id);
                        let _ = tg.send_to(chat_id, reply_thread, &reply).await;
                        continue;
                    }
                    if text == "/routes" {
                        let reply = handle_routes_cmd();
                        let _ = tg.send(chat_id, &reply).await;
                        continue;
                    }
                    if text.starts_with("/model") {
                        let thread_id = msg.message_thread_id;
                        let reply_thread = if msg.chat.id != from_id { thread_id } else { None };
                        let reply = handle_model_cmd(
                            &text,
                            msg.chat.id,
                            thread_id,
                            &default_backend_name,
                            &backends,
                        );
                        let _ = tg.send_to(chat_id, reply_thread, &reply).await;
                        continue;
                    }

                    // Check if this came from a mapped group topic
                    let thread_id = msg.message_thread_id;
                    let all_routes = routes::load();
                    let reply_thread = if msg.chat.id != from_id { thread_id } else { None };

                    let (repo_cwd, route_backend_name) = if msg.chat.id != from_id {
                        let route = all_routes.iter()
                            .find(|r| r.chat_id == msg.chat.id && r.thread_id == thread_id);
                        let cwd = route.map(|r| r.cwd.clone());
                        let b = route.and_then(|r| r.backend.clone());
                        (cwd, b)
                    } else {
                        (None, None)
                    };

                    // Private chat: try to resume the active Claude Code session first.
                    // If the session is still alive the response comes back to Telegram
                    // and we skip the normal backend routing entirely.
                    if msg.chat.id == from_id && repo_cwd.is_none() {
                        if let Some(session_id) = cfg.read_session() {
                            let text2 = text.clone();
                            let session_id2 = session_id.clone();
                            let tg3 = Arc::clone(&tg);
                            tokio::spawn(async move {
                                eprintln!("[bot] resuming session {session_id2} with reply");
                                let thinking_id = tg3.send(chat_id, "...").await.unwrap_or(0);
                                let output = resume_session(&session_id2, &text2).await;
                                if thinking_id > 0 { let _ = tg3.delete(chat_id, thinking_id).await; }
                                match output {
                                    Some(reply) => {
                                        chat_log::append("claude", &reply);
                                        let _ = tg3.send(chat_id, &reply).await;
                                    }
                                    None => { let _ = tg3.send(chat_id, "⚠ Session expired — start a new Claude Code session.").await; }
                                }
                            });
                            continue;
                        }
                    }

                    let backend_name = route_backend_name
                        .unwrap_or_else(|| default_backend_name.clone());
                    let backend2 = Arc::clone(&backends);
                    // Use the route CWD, or fall back to the last active Ludicrous repo
                    let effective_cwd = repo_cwd.clone().unwrap_or_else(|| cfg.effective_cwd());

                    tokio::spawn(async move {
                        let thinking_id = tg2.send_to(chat_id, reply_thread, "...").await.unwrap_or(0);
                        let backend = backend2.get(&backend_name);
                        let output = match backend {
                            None => Err(anyhow::anyhow!("backend '{backend_name}' not available")),
                            Some(b) => b.run_in(&text, &effective_cwd).await,
                        };
                        if thinking_id > 0 {
                            let _ = tg2.delete(chat_id, thinking_id).await;
                        }
                        let reply = output.unwrap_or_else(|e| { eprintln!("[bot] run_prompt error: {e}"); format!("error: {e}") });
                        chat_log::append("claude", &reply);
                        let _ = tg2.send_to(chat_id, reply_thread, &reply).await;
                    });
                }
            }
            Err(e) => eprintln!("[phonehome] poll error: {e}"),
        }
        sleep(Duration::from_millis(poll_ms)).await;
    }
}

async fn handle_audio(tg: &TelegramClient, backend: &dyn AiBackend, chat_id: i64, thread_id: Option<i64>, msg_id: i64, file_id: &str) {
    let status_id = tg.send_to(chat_id, thread_id, "🎤 Transcribing...").await.unwrap_or(0);

    let transcript = async {
        let file_path = tg.get_file_path(file_id).await?;
        let audio_bytes = tg.download_file(&file_path).await?;

        let tmp_in = format!("/tmp/ph-audio-{msg_id}.ogg");
        let tmp_wav = format!("/tmp/ph-audio-{msg_id}.wav");
        std::fs::write(&tmp_in, &audio_bytes)?;

        let status = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-i", &tmp_in, "-ar", "16000", "-ac", "1", "-acodec", "pcm_s16le", "-f", "wav", &tmp_wav])
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
        Ok(_) => { let _ = tg.send_to(chat_id, thread_id, "⚠️ Could not transcribe audio.").await; return; }
        Err(e) => { let _ = tg.send_to(chat_id, thread_id, &format!("⚠️ Transcription error: {e}")).await; return; }
    };

    let _ = tg.send_to(chat_id, thread_id, &format!("🎤 You said: \"{}\"", transcript)).await;

    let thinking_id = tg.send_to(chat_id, thread_id, "...").await.unwrap_or(0);
    let output = run_prompt(backend, &transcript).await;
    if thinking_id > 0 { let _ = tg.delete(chat_id, thinking_id).await; }
    let reply = output.unwrap_or_else(|e| format!("error: {e}"));
    let _ = tg.send_to(chat_id, thread_id, &reply).await;
}

async fn run_prompt(backend: &dyn AiBackend, prompt: &str) -> Result<String> {
    backend.run(prompt).await
}

/// Resume the active Claude Code terminal session with a user reply.
/// Returns None if the session has expired or claude is not found.
async fn resume_session(session_id: &str, prompt: &str) -> Option<String> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;

    let mut child = tokio::process::Command::new("claude")
        .args(["--resume", session_id, "-p", prompt, "--print"])
        .env("PHONEHOME_BOT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut o) = child.stdout.take() { o.read_to_string(&mut stdout).await.ok(); }
    if let Some(mut e) = child.stderr.take() { e.read_to_string(&mut stderr).await.ok(); }
    let _ = child.wait().await;

    let combined = format!("{}{}", stdout, stderr).trim().to_string();
    if combined.is_empty() || combined.contains("No conversation found") {
        eprintln!("[bot] session resume failed: {combined}");
        return None;
    }
    Some(combined)
}

fn handle_phonehome_cmd(
    text: &str,
    cfg: &Config,
    group_chat_id: Option<i64>,
    msg_thread_id: Option<i64>,
) -> String {
    let args: Vec<&str> = text["/phonehome".len()..].split_whitespace().collect();

    // Sent from inside a group topic: /phonehome REPO_PATH
    if let (Some(gchat), Some(tid)) = (group_chat_id, msg_thread_id) {
        let repo_path = match args.as_slice() {
            [path] => *path,
            [] => cfg.cwd.as_str(),
            _ => return "Usage (in topic): /phonehome REPO_PATH".to_string(),
        };
        if !std::path::Path::new(repo_path).is_dir() {
            return format!("Directory not found: {repo_path}");
        }
        let route = routes::Route {
            cwd: repo_path.to_string(),
            chat_id: gchat,
            thread_id: Some(tid),
            label: String::new(),
            backend: None,
        };
        return match routes::upsert(route) {
            Ok(()) => format!("Mapped\n{}\n-> this topic (thread {})", repo_path, tid),
            Err(e) => format!("Failed to save route: {e}"),
        };
    }

    // Sent from private chat: /phonehome [REPO_PATH] https://t.me/c/XXXXXXXXXX/Y
    let (repo_path, url) = match args.as_slice() {
        [url] if url.starts_with("https://") => (cfg.cwd.as_str(), *url),
        [path, url] if url.starts_with("https://") => (*path, *url),
        _ => return "Usage: /phonehome [REPO_PATH] https://t.me/c/XXXXXXXXXX/Y\nOr send /phonehome REPO_PATH inside a group topic.".to_string(),
    };

    let (chat_id, thread_id) = match routes::parse_tme_url(url) {
        Some(v) => v,
        None => return format!("Could not parse URL: {url}"),
    };

    if !std::path::Path::new(repo_path).is_dir() {
        return format!("Directory not found: {repo_path}");
    }

    let route = routes::Route {
        cwd: repo_path.to_string(),
        chat_id,
        thread_id,
        label: String::new(),
        backend: None,
    };

    match routes::upsert(route) {
        Ok(()) => format!("Mapped\n{}\n-> chat {} thread {:?}", repo_path, chat_id, thread_id),
        Err(e) => format!("Failed to save route: {e}"),
    }
}

fn handle_model_cmd(
    text: &str,
    chat_id: i64,
    thread_id: Option<i64>,
    default_backend: &str,
    backends: &std::collections::HashMap<String, Arc<dyn AiBackend>>,
) -> String {
    let routes = routes::load();
    let current_route = routes.iter()
        .find(|r| r.chat_id == chat_id && r.thread_id == thread_id);

    let arg = text["/model".len()..].trim();

    if arg.is_empty() {
        let current = current_route
            .and_then(|r| r.backend.as_deref())
            .unwrap_or(default_backend);
        let mut out = format!("*Current model:* `{current}`\n\n*Available:*\n");
        for name in crate::backend::KNOWN_BACKENDS {
            let status = if backends.contains_key(*name) { "✓" } else { "✗ (not configured)" };
            let marker = if *name == current { " ◀" } else { "" };
            out.push_str(&format!("  `{name}` {status}{marker}\n"));
        }
        out.push_str("\nUse `/model <name>` to switch. `/model default` to reset.");
        return out;
    }

    if arg != "default" && !crate::backend::KNOWN_BACKENDS.contains(&arg) {
        return format!("Unknown backend: `{arg}`\nKnown: {}", crate::backend::KNOWN_BACKENDS.join(", "));
    }

    if arg != "default" && !backends.contains_key(arg) {
        return format!("`{arg}` is not available (missing credentials). Check config.toml.");
    }

    if current_route.is_none() {
        return "No route configured for this topic. Run /phonehome first.".to_string();
    }

    match routes::set_backend(chat_id, thread_id, arg) {
        Ok(true) => {
            let label = if arg == "default" { format!("reset to default (`{default_backend}`)") } else { format!("set to `{arg}`") };
            format!("Model {label} for this topic.")
        }
        Ok(false) => "Route not found — run /phonehome first.".to_string(),
        Err(e) => format!("Failed to update route: {e}"),
    }
}

/// Resolve a bare repo name to a canonical full path.
/// Search order: configured repos_dir first, then parents of existing routes.
fn resolve_repo_name(name: &str, repos_dir: &str, routes: &[routes::Route]) -> Option<String> {
    use std::collections::HashSet;

    let mut roots: Vec<String> = vec![repos_dir.to_string()];

    // Also scan parents of existing routes (in case repos live in multiple places)
    let extra: HashSet<String> = routes.iter()
        .filter_map(|r| std::path::Path::new(&r.cwd).parent()
            .and_then(|p| p.to_str())
            .map(|s| s.to_string()))
        .collect();
    for r in extra {
        if !roots.contains(&r) { roots.push(r); }
    }

    for root in &roots {
        let candidate = format!("{root}/{name}");
        if std::path::Path::new(&candidate).is_dir() {
            return std::fs::canonicalize(&candidate)
                .ok()
                .and_then(|p| p.to_str().map(|s| s.to_string()))
                .or(Some(candidate));
        }
    }
    None
}

fn handle_routes_cmd() -> String {
    let rs = routes::load();
    if rs.is_empty() {
        return "No routes configured. Use /phonehome REPO_PATH URL to add one.".to_string();
    }
    let mut out = String::from("*Configured routes:*\n\n");
    for r in &rs {
        let thread = r.thread_id.map(|t| format!("topic {t}")).unwrap_or_else(|| "main".to_string());
        out.push_str(&format!("`{}` → chat {} ({})\n", r.cwd, r.chat_id, thread));
    }
    out
}
