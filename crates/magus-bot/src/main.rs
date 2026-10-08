use std::time::Duration;

use clap::Parser;
use magus_protocol::DEFAULT_PORT;
use tracing_subscriber::EnvFilter;

/// A computer opponent: joins a room and plays one game.
#[derive(Parser)]
struct Args {
    /// Server address.
    #[arg(long, default_value_t = format!("127.0.0.1:{DEFAULT_PORT}"))]
    server: String,
    /// Room to join; join the same room from `magus` to play against the bot.
    #[arg(long, default_value = "practice")]
    room: String,
    /// Deck to play.
    #[arg(long, default_value = "tide-ash")]
    deck: String,
    #[arg(long, default_value = "Bot")]
    name: String,
    /// Pause before each move so you can follow along.
    #[arg(long, default_value_t = 600)]
    delay_ms: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let outcome = magus_bot::run(
        &args.server,
        &args.name,
        &args.room,
        &args.deck,
        Duration::from_millis(args.delay_ms),
    )
    .await?;
    match outcome {
        Some(outcome) => tracing::info!(name = %args.name, ?outcome, "game over"),
        None => tracing::warn!(name = %args.name, "disconnected before the game ended"),
    }
    Ok(())
}
