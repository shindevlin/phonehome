mod backend;
mod bot;
mod chat_log;
mod whisper;
mod config;
mod echo;
mod init;
mod link;
mod notify;
mod routes;
mod telegram;
mod transcribe;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "phonehome", about = "BYOB Telegram remote for any AI CLI backend", version)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start the Telegram bot (polls for messages, runs AI backend)
    Bot {
        /// Your Telegram user ID (messages from other users are ignored)
        #[arg(long)]
        owner_id: i64,
    },
    /// Stop hook handler — reads Claude Code JSON from stdin, sends last response to Telegram
    Notify,
    /// UserPromptSubmit hook — echoes your message to Telegram
    Echo,
    /// Create ~/.phonehome/config.toml interactively
    Init {
        #[arg(long)]
        bot_token: String,
        #[arg(long)]
        chat_id: i64,
        #[arg(long, default_value = "claude")]
        backend: String,
    },
    /// Register a directory so phonehome watches it and routes sessions from it
    Link {
        /// Path to register (defaults to current directory)
        path: Option<std::path::PathBuf>,
    },
    /// Remove a directory from the watch list
    Unlink {
        /// Path to remove (defaults to current directory)
        path: Option<std::path::PathBuf>,
    },
    /// List all linked directories
    Links,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Skip notify when we ourselves triggered the AI (prevents loops)
    if std::env::var("PHONEHOME_BOT").is_ok() {
        return Ok(());
    }

    let cli = Cli::parse();
    match cli.command {
        Cmd::Bot { owner_id } => {
            let cfg = config::Config::load()?;
            bot::run(cfg, owner_id).await?;
        }
        Cmd::Notify => {
            let cfg = config::Config::load()?;
            notify::run(cfg).await?;
        }
        Cmd::Echo => {
            let cfg = config::Config::load()?;
            echo::run(cfg).await?;
        }
        Cmd::Init { bot_token, chat_id, backend } => {
            init::run(bot_token, chat_id, backend)?;
        }
        Cmd::Link { path } => link::link(path)?,
        Cmd::Unlink { path } => link::unlink(path)?,
        Cmd::Links => link::list()?,
    }
    Ok(())
}
