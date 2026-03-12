mod agent;
mod cli;
mod common;
mod presence;
mod task;

use anyhow::Result;
use clap::Parser;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let transport = LogosMessagingTransport::new(&cli.waku);

    match cli.command {
        Commands::Agent { action } => agent::handle(action, transport).await,
        Commands::Task { action } => task::handle(action, transport).await,
        Commands::Presence { action } => presence::handle(action, transport).await,
    }
}
