use clap::Parser;
use magus_protocol::DEFAULT_PORT;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

/// Hosts Magus games. Players who join the same room are paired up.
#[derive(Parser)]
struct Args {
    /// Address to listen on.
    #[arg(long, default_value_t = format!("0.0.0.0:{DEFAULT_PORT}"))]
    bind: String,
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
    let listener = TcpListener::bind(&args.bind).await?;
    tracing::info!(addr = %listener.local_addr()?, "magus server listening");
    magus_server::serve(listener).await?;
    Ok(())
}
