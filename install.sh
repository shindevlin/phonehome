#!/usr/bin/env bash
set -euo pipefail

# PhoneHome installer
# Installs the phonehome binary, configures ~/.phonehome/config.toml,
# wires Claude Code hooks, and sets up a systemd user service.

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

say()  { echo -e "${CYAN}${BOLD}==>${RESET} $*"; }
ok()   { echo -e "${GREEN}✓${RESET} $*"; }
warn() { echo -e "${YELLOW}⚠${RESET}  $*"; }
die()  { echo -e "${RED}✗${RESET}  $*" >&2; exit 1; }
ask()  { echo -e "${BOLD}$*${RESET}"; }

CONFIG_DIR="$HOME/.phonehome"
CONFIG_FILE="$CONFIG_DIR/config.toml"
HOOKS_FILE="$HOME/.claude/settings.json"

# ─── Rust / Cargo ────────────────────────────────────────────────────────────
install_cargo() {
    if command -v cargo &>/dev/null; then
        ok "cargo $(cargo --version 2>/dev/null | awk '{print $2}') already installed"
        return
    fi
    say "cargo not found — installing Rust via rustup"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env" 2>/dev/null || true
    export PATH="$HOME/.cargo/bin:$PATH"
    ok "Rust installed"
}

install_binary() {
    say "Building and installing phonehome binary (this may take a minute)"
    cargo install --path "$(dirname "$0")" --quiet
    ok "phonehome installed to $(which phonehome 2>/dev/null || echo "~/.cargo/bin/phonehome")"
}

# ─── Bot token ───────────────────────────────────────────────────────────────
get_bot_token() {
    echo
    ask "Step 1: Telegram Bot Token"
    echo "  Create a bot via @BotFather on Telegram (send /newbot)."
    echo "  Paste the token below (format: 123456789:ABC-DEF...)."
    echo
    while true; do
        read -rp "  Bot token: " BOT_TOKEN
        BOT_TOKEN="${BOT_TOKEN// /}"
        if [[ "$BOT_TOKEN" =~ ^[0-9]+:[A-Za-z0-9_-]{35,}$ ]]; then
            break
        fi
        warn "That doesn't look like a valid bot token. Try again."
    done
    ok "Token accepted"
}

# ─── Owner chat_id ───────────────────────────────────────────────────────────
get_owner_id() {
    echo
    ask "Step 2: Your Telegram user ID"
    echo "  We'll auto-detect it. Send any message to your bot now,"
    echo "  then press Enter."
    read -rp "  (press Enter after messaging your bot) "

    local updates
    updates=$(curl -sf "https://api.telegram.org/bot${BOT_TOKEN}/getUpdates?limit=10&allowed_updates=message" 2>/dev/null || true)
    OWNER_ID=$(echo "$updates" | python3 -c "
import json, sys
data = json.load(sys.stdin)
results = data.get('result', [])
for r in reversed(results):
    uid = r.get('message', {}).get('from', {}).get('id')
    if uid:
        print(uid)
        break
" 2>/dev/null || true)

    if [[ -z "$OWNER_ID" ]]; then
        warn "Couldn't auto-detect your user ID."
        echo "  You can find it by sending /start to @userinfobot on Telegram."
        read -rp "  Enter your Telegram user ID manually: " OWNER_ID
    fi
    ok "Owner ID: $OWNER_ID"
}

# ─── Group / topic ───────────────────────────────────────────────────────────
get_group() {
    echo
    ask "Step 3: Telegram group URL"
    echo "  Open your Telegram group in the browser and copy the URL."
    echo "  Format: https://t.me/c/1234567890/1  (the trailing /1 is the topic)"
    echo "  (Leave blank to use private chat with the bot instead)"
    echo
    read -rp "  Group URL (or blank for private chat): " GROUP_URL
    GROUP_URL="${GROUP_URL// /}"

    CHAT_ID="$OWNER_ID"
    if [[ -n "$GROUP_URL" ]]; then
        # Parse t.me/c/XXXXXXXXXX/Y  →  chat_id = -100XXXXXXXXXX
        local raw_id
        raw_id=$(echo "$GROUP_URL" | grep -oP '(?<=t\.me/c/)\d+' || true)
        if [[ -n "$raw_id" ]]; then
            CHAT_ID="-100${raw_id}"
            ok "Group chat ID: $CHAT_ID"
        else
            warn "Couldn't parse that URL — defaulting to private chat."
            CHAT_ID="$OWNER_ID"
        fi
    else
        ok "Using private chat (chat ID = $CHAT_ID)"
    fi
}

# ─── repos_dir ───────────────────────────────────────────────────────────────
get_repos_dir() {
    echo
    ask "Step 4: Your repos directory"
    echo "  PhoneHome uses this to resolve short repo names (e.g. /phonehome brutus11)."

    local default_dir=""
    for candidate in "$HOME/repos" "$HOME/projects" "$HOME/code"; do
        if [[ -d "$candidate" ]]; then
            default_dir="$candidate"
            break
        fi
    done
    default_dir="${default_dir:-$HOME}"

    read -rp "  Repos directory [${default_dir}]: " REPOS_DIR
    REPOS_DIR="${REPOS_DIR:-$default_dir}"
    REPOS_DIR="${REPOS_DIR%/}"
    ok "Repos dir: $REPOS_DIR"
}

# ─── Backend ─────────────────────────────────────────────────────────────────
get_backend() {
    echo
    ask "Step 5: Default AI backend"
    echo
    echo "  PhoneHome supports multiple AI backends. You can set a different"
    echo "  backend per Telegram topic with /model, so you're not locked in."
    echo
    echo "  Available backends:"
    echo "    claude      — Claude Code CLI (uses your existing Claude session)"
    echo "    gemini      — Google Gemini (needs GEMINI_API_KEY in environment)"
    echo "    openrouter  — OpenRouter (GPT-4o, Llama, etc.) — needs API key"
    echo "    btcpc       — Local model on this machine"
    echo "    codex       — OpenAI Codex"
    echo
    read -rp "  Default backend [claude]: " BACKEND
    BACKEND="${BACKEND:-claude}"
    OPENROUTER_KEY=""

    if [[ "$BACKEND" == "openrouter" ]]; then
        read -rp "  OpenRouter API key: " OPENROUTER_KEY
    fi
    ok "Default backend: $BACKEND"
}

# ─── Write config ─────────────────────────────────────────────────────────────
write_config() {
    mkdir -p "$CONFIG_DIR"

    cat > "$CONFIG_FILE" <<TOML
bot_token = "${BOT_TOKEN}"
chat_id = ${CHAT_ID}
backend = "${BACKEND}"
cwd = "${HOME}"
repos_dir = "${REPOS_DIR}"

[backends]
TOML

    if [[ -n "$OPENROUTER_KEY" ]]; then
        echo "openrouter_api_key = \"${OPENROUTER_KEY}\"" >> "$CONFIG_FILE"
    fi

    ok "Config written to $CONFIG_FILE"
}

# ─── Claude Code hooks ────────────────────────────────────────────────────────
wire_hooks() {
    mkdir -p "$HOME/.claude"

    if [[ ! -f "$HOOKS_FILE" ]]; then
        echo '{}' > "$HOOKS_FILE"
    fi

    python3 - <<'PYTHON'
import json, os, sys

hooks_file = os.path.expanduser("~/.claude/settings.json")
with open(hooks_file) as f:
    cfg = json.load(f)

hooks = cfg.setdefault("hooks", {})

def ensure_hook(event, matcher, cmd):
    group = hooks.setdefault(event, [])
    # Check if our command is already present
    for entry in group:
        for hook in entry.get("hooks", []):
            if hook.get("command", "") == cmd:
                return
    group.append({"matcher": matcher, "hooks": [{"type": "command", "command": cmd}]})

ensure_hook("Stop",              "", "phonehome notify")
ensure_hook("UserPromptSubmit",  "", "phonehome echo")

with open(hooks_file, "w") as f:
    json.dump(cfg, f, indent=2)
print("Hooks wired.")
PYTHON

    ok "Claude Code hooks wired in $HOOKS_FILE"
}

# ─── systemd user service ─────────────────────────────────────────────────────
write_service() {
    local service_dir="$HOME/.config/systemd/user"
    local service_file="$service_dir/phonehome.service"
    mkdir -p "$service_dir"

    # phonehome daemon is only needed for the bot polling loop
    # (the hooks call phonehome notify/echo as one-shot commands)
    cat > "$service_file" <<SERVICE
[Unit]
Description=PhoneHome Telegram bot
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=${HOME}/.cargo/bin/phonehome bot
Restart=on-failure
RestartSec=5
Environment=PATH=${HOME}/.cargo/bin:/usr/local/bin:/usr/bin:/bin

[Install]
WantedBy=default.target
SERVICE

    systemctl --user daemon-reload 2>/dev/null || true
    systemctl --user enable --now phonehome 2>/dev/null && ok "phonehome service enabled and started" || \
        warn "Couldn't start systemd service. Start it manually: systemctl --user enable --now phonehome"
}

# ─── Next steps ──────────────────────────────────────────────────────────────
print_next_steps() {
    echo
    echo -e "${BOLD}${GREEN}Installation complete!${RESET}"
    echo
    echo -e "${BOLD}Next steps:${RESET}"
    echo
    echo "  1. In each Telegram topic where you want a repo routed, send:"
    echo "       /phonehome REPONAME"
    echo "     PhoneHome will find the repo and confirm the mapping."
    echo
    echo "  2. Each topic can use a different AI backend. To switch:"
    echo "       /model gemini"
    echo "       /model openrouter"
    echo "       /model claude"
    echo "     The change is per-topic — other repos keep their backend."
    echo
    echo "  3. List active routes:"
    echo "       /routes"
    echo
    echo "  4. View the service log:"
    echo "       journalctl --user -u phonehome -f"
    echo
}

# ─── Main ─────────────────────────────────────────────────────────────────────
main() {
    echo
    echo -e "${BOLD}PhoneHome Installer${RESET}"
    echo "──────────────────────────────────────────"
    echo "  Bridges Claude Code sessions → Telegram"
    echo "──────────────────────────────────────────"
    echo

    install_cargo
    get_bot_token
    get_owner_id
    get_group
    get_repos_dir
    get_backend
    write_config
    install_binary
    wire_hooks
    write_service
    print_next_steps
}

main "$@"
