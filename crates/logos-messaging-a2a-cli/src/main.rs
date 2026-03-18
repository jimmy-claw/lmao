mod agent;
mod cli;
mod common;
mod completion;
mod config;
mod health;
mod info;
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

    // Load configuration file (--config path, ./lmao.toml, or ~/.config/lmao/config.toml)
    let cfg = config::load_config(cli.config.as_deref())?;

    // Merge: CLI flags > config file > defaults
    let (waku_url, keyfile, encrypt, json) =
        config::merge(cli.waku, cli.keyfile, cli.encrypt, cli.json, &cfg);

    let transport = LogosMessagingTransport::new(&waku_url);
    let identity = IdentityConfig { keyfile, encrypt };

    match cli.command {
        Commands::Agent { action } => agent::handle(action, transport, &identity, json).await,
        Commands::Task { action } => task::handle(action, transport, &identity, json).await,
        Commands::Presence { action } => presence::handle(action, transport, &identity, json).await,
        Commands::Session { action } => session::handle(action, transport, &identity, json).await,
        Commands::Health => health::handle(&waku_url, json).await,
        Commands::Metrics => metrics::handle(transport, &identity, json).await,
        Commands::Completion { shell } => {
            completion::handle(shell);
            Ok(())
        }
        Commands::Info => info::handle(transport, &identity, json),
        Commands::Config { action } => match action {
            cli::ConfigAction::Init => config::init_config(json),
            cli::ConfigAction::Show => {
                let effective = config::LmaoConfig {
                    waku_url: Some(waku_url),
                    keyfile: identity.keyfile.clone(),
                    encrypt: Some(encrypt),
                    json: Some(json),
                    agent: cfg.agent,
                    presence: cfg.presence,
                };
                config::show_config(&effective, json)
            }
        },
    }
}
