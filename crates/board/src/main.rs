//! Board: a planning canvas for client website projects.
//!
//! See `planning-board-tech-spec-rust.md` for the design and `plan.md` for
//! the build order.

mod app;
mod assets;
mod config;
// The Typeset mapper is wired in during phase 7; until then only its tests use it.
#[allow(dead_code)]
mod export;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use tracing_subscriber::EnvFilter;

use crate::app::AppState;
use crate::config::Config;

#[derive(Parser)]
#[command(
    name = "board",
    version,
    about = "A planning canvas for client website projects"
)]
struct Cli {
    /// Path to the config file. Created with defaults if it doesn't exist.
    #[arg(
        long,
        env = "BOARD_CONFIG",
        default_value = "board.toml",
        global = true
    )]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server.
    Serve,
    /// Print a one-time login link for the web UI.
    Login,
    /// Manage API tokens.
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    /// Write a board's model (`board-model.json`) into a directory.
    Pull {
        /// The board to pull.
        board_id: String,
        /// Directory to write into.
        #[arg(long, default_value = ".")]
        into: PathBuf,
    },
}

#[derive(Subcommand)]
enum TokenCommand {
    /// Create a token. It is shown once.
    Create {
        /// A name to recognise the token by, e.g. "claude-mcp" or "design".
        #[arg(long)]
        name: String,
        #[arg(long, value_enum)]
        scope: Scope,
        #[arg(long, value_enum)]
        actor: Actor,
    },
    /// List tokens (names and scopes, never the tokens themselves).
    List,
    /// Revoke a token by id.
    Revoke { id: String },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Scope {
    Admin,
    Read,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Actor {
    Claude,
    Api,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,board=debug")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Serve => serve(&cli.config).await,
        Command::Login => bail!("`board login` arrives in phase 1"),
        Command::Token { command } => match command {
            TokenCommand::Create { name, scope, actor } => bail!(
                "`board token create` (name {name:?}, scope {scope:?}, actor {actor:?}) arrives in phase 1"
            ),
            TokenCommand::List => bail!("`board token list` arrives in phase 1"),
            TokenCommand::Revoke { id } => bail!("`board token revoke {id}` arrives in phase 1"),
        },
        Command::Pull { board_id, into } => bail!(
            "`board pull {board_id} --into {}` arrives in phase 6",
            into.display()
        ),
    }
}

async fn serve(config_path: &std::path::Path) -> Result<()> {
    let config = Config::load_or_init(config_path)?;
    let bind = config.bind;

    let state = AppState {
        config: Arc::new(config),
    };
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(
        %bind,
        public_origin = %state.config.public_origin,
        database = %state.config.database.display(),
        typeset = state.config.typeset_url.as_deref().unwrap_or("not configured"),
        "board listening"
    );

    axum::serve(listener, app::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")
}

/// Resolve on Ctrl+C or SIGTERM (systemd stop).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                tracing::warn!(%err, "can't listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutting down");
}
