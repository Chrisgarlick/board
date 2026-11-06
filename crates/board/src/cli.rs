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
    /// Write a board's model (`board-model.json`) into a directory: from
    /// this machine's database, or from a remote Board with `--from`.
    Pull {
        /// The board to pull.
        board_id: String,
        /// Directory to write into.
        #[arg(long, default_value = ".")]
        into: PathBuf,
        /// A remote Board, e.g. https://board.example.com. Needs a token.
        #[arg(long, env = "BOARD_URL")]
        from: Option<String>,
        /// A read token for the remote Board.
        #[arg(long, env = "BOARD_TOKEN", hide_env_values = true)]
        token: Option<String>,
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
        Command::Pull {
            board_id,
            into,
            from,
            token,
        } => pull(&cli.config, &board_id, &into, from, token).await,
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

/// `board pull`: write `board-model.json`, pretty-printed in a stable order
/// so changes show up as clean diffs. Leaves the file alone when nothing
/// changed.
async fn pull(
    config_path: &Path,
    board_id: &str,
    into: &Path,
    from: Option<String>,
    token: Option<String>,
) -> Result<()> {
    let model: board_model::BoardModel = match from {
        Some(origin) => {
            let token = token.context("--from needs a token: pass --token or set BOARD_TOKEN")?;
            let url = format!(
                "{}/api/boards/{board_id}/model",
                origin.trim_end_matches('/')
            );
            let body = tokio::task::spawn_blocking(move || fetch(&url, &token)).await??;
            serde_json::from_str(&body).context("the server's reply isn't a board model")?
        }
        None => {
            let (_config, pool) = open(config_path).await?;
            let mut conn = pool.acquire().await?;
            crate::model::load(&mut conn, board_id)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?
        }
    };
    let mut json = serde_json::to_string_pretty(&model)?;
    json.push('\n');
    std::fs::create_dir_all(into).with_context(|| format!("creating {}", into.display()))?;
    let path = into.join("board-model.json");
    if std::fs::read_to_string(&path).is_ok_and(|old| old == json) {
        println!("{} is up to date.", path.display());
        return Ok(());
    }
    std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
    println!(
        "Wrote {} ({} tables, {} pages, {} features, {} flows).",
        path.display(),
        model.tables.len(),
        model.pages.iter().map(|p| p.walk().len()).sum::<usize>(),
        model.features.len(),
        model.flows.len()
    );
    Ok(())
}

fn fetch(url: &str, token: &str) -> Result<String> {
    let mut res = ureq::get(url)
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(401) => anyhow::anyhow!("the token was refused"),
            ureq::Error::StatusCode(404) => anyhow::anyhow!("no such board on that server"),
            other => anyhow::anyhow!("fetching {url}: {other}"),
        })?;
    Ok(res.body_mut().read_to_string()?)
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
