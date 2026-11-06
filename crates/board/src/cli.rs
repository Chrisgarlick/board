//! The `board` command line.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};

use crate::app::{self, AppState};
use crate::auth;
use crate::clock::{Clock, SystemClock};
use crate::config::Config;
use crate::db;
use crate::service::housekeeping;
use crate::types;

#[derive(Parser)]
#[command(
    name = "board",
    version,
    about = "A planning canvas for client website projects"
)]
pub struct Cli {
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

pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Serve => serve(&cli.config).await,
        Command::Login => login(&cli.config).await,
        Command::Token { command } => token(&cli.config, command).await,
        Command::Pull { board_id, into } => bail!(
            "`board pull {board_id} --into {}` arrives in phase 6",
            into.display()
        ),
    }
}

async fn open(config_path: &Path) -> Result<(Config, sqlx::SqlitePool)> {
    let config = Config::load_or_init(config_path)?;
    let pool = db::connect(&config.database).await?;
    Ok((config, pool))
}

async fn serve(config_path: &Path) -> Result<()> {
    let (config, pool) = open(config_path).await?;
    let bind = config.bind;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);

    housekeeping::spawn(pool.clone(), clock.clone());

    let state = AppState {
        config: Arc::new(config),
        pool,
        clock,
        hub: Default::default(),
        renders: Default::default(),
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

async fn login(config_path: &Path) -> Result<()> {
    let (config, pool) = open(config_path).await?;
    let code = auth::create_login_code(&pool, SystemClock.now()).await?;
    println!("{}/login/{code}", config.public_origin);
    eprintln!("This link works once, within 10 minutes.");
    Ok(())
}

async fn token(config_path: &Path, command: TokenCommand) -> Result<()> {
    let (_, pool) = open(config_path).await?;
    match command {
        TokenCommand::Create { name, scope, actor } => {
            let scope = match scope {
                Scope::Admin => types::Scope::Admin,
                Scope::Read => types::Scope::Read,
            };
            let actor = match actor {
                Actor::Claude => types::Actor::Claude,
                Actor::Api => types::Actor::Api,
            };
            let token = auth::create_token(&pool, &name, scope, actor, SystemClock.now()).await?;
            println!("{}", token.token);
            eprintln!(
                "Token {} created ({}, acting as {}). This is the only time it's shown.",
                token.id,
                scope.as_str(),
                actor
            );
        }
        TokenCommand::List => {
            let tokens = auth::list_tokens(&pool).await?;
            if tokens.is_empty() {
                eprintln!("No tokens yet. Create one with `board token create`.");
            }
            for t in tokens {
                println!(
                    "{}  {:<16} {:<6} {:<7} created {}  last used {}",
                    t.id,
                    t.name,
                    t.scope,
                    t.actor,
                    t.created_at,
                    t.last_used_at.as_deref().unwrap_or("never")
                );
            }
        }
        TokenCommand::Revoke { id } => {
            if auth::revoke_token(&pool, &id).await? {
                eprintln!("Revoked {id}.");
            } else {
                bail!("no token with id {id}");
            }
        }
    }
    Ok(())
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
