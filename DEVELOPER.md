# Developer Notes — phonehome

## Architecture

```
src/
├── main.rs         — CLI entry point (clap), subcommand routing
├── config.rs       — Config loading (TOML + env overrides), session file I/O
├── backend/
│   ├── mod.rs      — AiBackend trait, backend factory
│   └── claude.rs   — Claude Code backend (--resume/--continue, stale session retry)
├── bot.rs          — Telegram polling loop, text + voice message dispatch
├── notify.rs       — Stop hook handler (reads JSONL tail, sends last assistant turn)
├── echo.rs         — UserPromptSubmit hook (echoes prompt to Telegram)
├── telegram.rs     — Thin Telegram HTTP client (send, delete, getUpdates, file download)
├── transcribe.rs   — Whisper transcription entry point
└── whisper/        — Whisper inference (pure Rust, no C FFI)
    ├── mod.rs
    ├── model.rs
    ├── ops.rs
    ├── mel.rs
    └── ggml.rs
```

## Key design decisions

### BYOB (Bring Your Own Bot)
There is no central routing server. All Telegram traffic flows directly between the user's bot token and Telegram's API. This is a hard invariant — don't add any intermediate hop.

### Loop prevention
When `phonehome bot` spawns Claude, it sets `PHONEHOME_BOT=1` in the child environment. `main.rs` exits immediately if this var is set, so Stop hooks triggered by bot-spawned sessions are no-ops.

### Session continuity
`config.rs` tracks the active Claude session in `session_file` (default `~/.phonehome_session`). The `notify` subcommand updates this file every time the Stop hook fires with a valid `session_id`. The `bot` subcommand reads it to call `--resume <id>`, falling back to `--continue`. If Claude returns "No conversation found with session ID", `claude.rs` wipes the stale ID and retries with `--continue`.

### Stop hook timing
The Stop hook fires before Claude finishes writing the last assistant message to the JSONL. `notify.rs` polls the session file size with a 300ms tick for up to 5 seconds, then reads the tail when the file grows. This means `notify` blocks for up to 5 seconds; the hook is set `async: true` so it doesn't hold up Claude Code.

### JSONL tail read
`notify.rs` reads the last 512 KB of the session JSONL. It scans backwards, returning the first `assistant` type entry with non-empty `text` content blocks, stopping if it encounters a human `user` message first.

### Telegram client
`telegram.rs` is a hand-rolled thin client. No `teloxide` or other Telegram framework — the only external HTTP dep is `reqwest`. This keeps binary size small and avoids pulling in tokio runtime duplicates.

Chunk size is 4000 characters (Telegram limit is 4096, but we leave headroom for Markdown escaping). Character boundary–safe slicing is used throughout to avoid panics on multi-byte Unicode.

`send()` tries Markdown parse mode first; falls back to plain text if the API rejects it (malformed Markdown in Claude output is common).

## Adding a backend

1. Create `src/backend/<name>.rs` implementing `AiBackend`:
   ```rust
   #[async_trait]
   impl AiBackend for MyBackend {
       async fn run(&self, prompt: &str) -> Result<String> {
           // spawn your CLI tool, return its stdout
       }
       fn save_session(&self, output: &str) {
           // parse and persist session ID from output, if applicable
       }
   }
   ```
2. Register in `src/backend/mod.rs`:
   ```rust
   "<name>" => Ok(Box::new(MyBackend::new(cfg))),
   ```
3. Set `backend = "<name>"` in `~/.phonehome/config.toml`.

The `save_session` method is called after every `run()`. For backends without session IDs (Gemini, OpenCode), implement it as a no-op.

## Whisper model

The whisper implementation in `src/whisper/` is a pure-Rust port. It:
1. Downloads the `ggml-base.en` model to `~/.phonehome/whisper/` on first use
2. Computes a log-mel spectrogram from 16 kHz mono PCM (via `src/whisper/mel.rs`)
3. Runs the encoder/decoder against the model weights (via `src/whisper/model.rs`)

`ffmpeg` is required for OGG→WAV conversion before transcription. The bot calls ffmpeg with `-ar 16000 -ac 1 -f wav` to normalize the input.

## Config file security

The config file contains the bot token in plaintext. On creation via `phonehome init`, the file is written with mode 0600. If you're deploying in a shared environment, use `PHONEHOME_BOT_TOKEN` env var instead and leave the config field empty.

## Running in development

```bash
# Run the bot directly (Ctrl-C to stop)
cargo run -- bot --owner-id YOUR_TELEGRAM_ID

# Test the notify path manually
echo '{"session_id":"your-session-id"}' | cargo run -- notify

# Echo hook test
echo '{"prompt":"hello"}' | cargo run -- echo
```

## Deployment checklist

- [ ] `~/.phonehome/config.toml` exists with correct token and chat ID
- [ ] `phonehome bot` starts cleanly (check `[phonehome] bot live` in logs)
- [ ] Stop hook wired in `~/.claude/settings.json`
- [ ] `PHONEHOME_BOT=1` is NOT set in the shell where you run Claude manually
- [ ] `chmod 600 ~/.phonehome/config.toml`
- [ ] If using systemd: `systemctl --user status phonehome` shows active

## Known limitations

- Only one AI session at a time (last session ID wins in the session file)
- Voice transcription requires `ffmpeg` and ~150 MB disk for the Whisper model
- Responses over 20 000 characters are truncated before sending
- The Stop hook fires even on `/clear` and `/compact` — those sessions produce no assistant text, so `notify` exits cleanly with no Telegram message sent
