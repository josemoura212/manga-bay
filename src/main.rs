#![allow(unused)]
mod api;
mod node;
mod p2p;
mod storage;
mod utils;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value_t = 3000)]
    port: u16,

    /// Path to data directory
    #[arg(short, long, default_value = "./data")]
    data_dir: String,

    /// Bootstrap peers (Multiaddrs)
    #[arg(short, long)]
    bootstrap_peers: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "manga_bay=debug,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();
    tracing::info!("Starting Manga Bay Node on port {}", args.port);

    let storage = storage::engine::Storage::new(&args.data_dir).await?;
    node::app::run(args.port, storage, args.bootstrap_peers).await?;

    Ok(())
}
