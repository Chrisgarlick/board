use clap::Parser as _;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Logs go to stderr so stdout carries only a command's output (a token,
    // a login link) and can be piped or captured safely.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,board=debug")),
        )
        .init();

    board::cli::run(board::cli::Cli::parse()).await
}
