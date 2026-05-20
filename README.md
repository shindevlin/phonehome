# phonehome

**BYOB Telegram remote for any AI CLI backend.**

Send messages to Claude Code (or any AI CLI) from your phone via Telegram, and get Claude's responses pushed back automatically — all without any central server. You bring your own bot token; your messages go directly through Telegram's API to your machine.

---

## How it works

```
You (Telegram) ──▶ Bot token ──▶ Your machine ──▶ Claude Code ──▶ Telegram ──▶ You
```

No cloud routing. No data collection. The bot token goes from your Telegram bot directly to your server — phonehome never sees it.

---

## Quick start

### 1. Create a Telegram bot

1. Message [@BotFather](https://t.me/BotFather) on Telegram
2. Send `/newbot` and follow the prompts
3. Copy the bot token (looks like `1234567890:AAF...`)
4. Find your own Telegram user ID: message [@userinfobot](https://t.me/userinfobot)

### 2. Install phonehome

```bash
cargo install phonehome
```

Or build from source:

```bash
git clone https://github.com/shindevlin/phonehome
cd phonehome
cargo install --path .
```

### 3. Configure

```bash
phonehome init --bot-token "YOUR_TOKEN" --chat-id YOUR_USER_ID
```

This creates `~/.phonehome/config.toml`. Edit it to set your working directory and backend:

```toml
bot_token = "1234567890:AAFabc..."
chat_id   = 9876543210
backend   = "claude"
cwd       = "/home/you"
```

### 4. Start the bot

```bash
phonehome bot --owner-id YOUR_USER_ID
```

Or run it as a systemd service (see [systemd setup](#systemd-setup)).

### 5. Wire the Stop hook

Add this to your `~/.claude/settings.json` so Claude pushes every response to Telegram automatically:

```json
{
  "hooks": {
    "Stop": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "phonehome notify",
        "timeout": 15,
        "async": true
      }]
    }]
  }
}
```

Restart Claude Code once. From then on, every response is forwarded to Telegram — no polling required.

---

## Commands

| Subcommand | Purpose |
|---|---|
| `phonehome bot --owner-id <id>` | Start the Telegram polling bot |
| `phonehome notify` | Stop hook — reads stdin, sends last response to Telegram |
| `phonehome init --bot-token ... --chat-id ...` | Create config |

---

## Configuration reference

`~/.phonehome/config.toml`:

```toml
bot_token        = ""      # Required: Telegram bot token
chat_id          = 0       # Required: your Telegram user ID
backend          = "claude" # AI backend: only "claude" today
cwd              = ""      # Working directory for AI commands (default: $HOME)
session_file     = ""      # Path to session resume file (default: ~/.phonehome_session)
poll_interval_ms = 1500    # Telegram polling interval in milliseconds
max_send_chars   = 20000   # Max characters forwarded per response
```

All fields except `bot_token` and `chat_id` have sensible defaults.

**Environment variable overrides** (useful for containers / secrets managers):

| Variable | Overrides |
|---|---|
| `PHONEHOME_BOT_TOKEN` | `bot_token` |
| `PHONEHOME_CHAT_ID` | `chat_id` |
| `PHONEHOME_BACKEND` | `backend` |

---

## Systemd setup

```ini
# ~/.config/systemd/user/phonehome.service
[Unit]
Description=phonehome Telegram bot
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/home/YOU/.cargo/bin/phonehome bot --owner-id YOUR_USER_ID
Restart=always
RestartSec=10
Environment=HOME=/home/YOU

[Install]
WantedBy=default.target
```

```bash
systemctl --user daemon-reload
systemctl --user enable --now phonehome
```

---

## Voice messages

phonehome accepts Telegram voice messages and audio files. It transcribes them locally using a bundled Whisper model (no cloud API key needed) and forwards the transcript to Claude as a text prompt.

**Requirements:** `ffmpeg` must be in your PATH for audio conversion.

The Whisper model is downloaded on first use and cached at `~/.phonehome/whisper/`.

---

## Adding a new AI backend

Implement the `AiBackend` trait in `src/backend/`:

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

Then register it in `src/backend/mod.rs`:

```rust
"mybackend" => Ok(Box::new(MyBackend::new(cfg))),
```

Set `backend = "mybackend"` in your config and restart.

---

## Security

- **Only your user ID is accepted.** Messages from other Telegram users are silently dropped.
- **No central routing.** All traffic goes directly between your bot token and Telegram's API.
- **PHONEHOME_BOT=1** is set in the environment of spawned AI processes, preventing Stop hook loops.
- The config file contains your bot token — set `chmod 600 ~/.phonehome/config.toml`.

---

## License

MIT
