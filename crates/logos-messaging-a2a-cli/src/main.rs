mod agent;
mod cli;
mod common;
mod completion;
mod health;
mod metrics;
mod presence;
mod session;
mod task;

use anyhow::Result;
use clap::Parser;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Commands};
use common::IdentityConfig;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let transport = LogosMessagingTransport::new(&cli.waku);
    let json = cli.json;
    let identity = IdentityConfig {
        keyfile: cli.keyfile,
        encrypt: cli.encrypt,
    };

    match cli.command {
        Commands::Agent { action } => agent::handle(action, transport, &identity, json).await,
        Commands::Task { action } => task::handle(action, transport, &identity, json).await,
        Commands::Presence { action } => presence::handle(action, transport, &identity, json).await,
        Commands::Session { action } => session::handle(action, transport, &identity, json).await,
        Commands::Health => health::handle(&cli.waku, json).await,
        Commands::Metrics => metrics::handle(transport, &identity, json).await,
        Commands::Completion { shell } => {
            completion::handle(shell);
            Ok(())
        }
    }
}
