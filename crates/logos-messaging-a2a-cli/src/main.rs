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

    // Load config file (explicit --config path, or auto-discovered)
    let (file_config, config_path) = config::load_config(cli.config.as_deref())?;

    // Merge: CLI flags > config file > built-in defaults
    let resolved = config::merge(&file_config, &cli);

    let transport = LogosMessagingTransport::new(&resolved.waku_url);
    let json = resolved.json;
    let identity = IdentityConfig {
        keyfile: resolved.keyfile,
        encrypt: resolved.encrypt,
    };

    match cli.command {
        Commands::Agent { action } => agent::handle(action, transport, &identity, json).await,
        Commands::Task { action } => task::handle(action, transport, &identity, json).await,
        Commands::Presence { action } => presence::handle(action, transport, &identity, json).await,
        Commands::Session { action } => session::handle(action, transport, &identity, json).await,
        Commands::Health => health::handle(&resolved.waku_url, json).await,
        Commands::Metrics => metrics::handle(transport, &identity, json).await,
        Commands::Completion { shell } => {
            completion::handle(shell);
            Ok(())
        }
        Commands::Info => info::handle(transport, &identity, json),
        Commands::Config { action } => {
            handle_config(action, &file_config, config_path.as_deref(), json)
        }
    }
}

fn handle_config(
    action: cli::ConfigAction,
    file_config: &config::Config,
    config_path: Option<&std::path::Path>,
    json: bool,
) -> Result<()> {
    match action {
        cli::ConfigAction::Init { path } => {
            let target = path
                .or_else(config::default_config_path)
                .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;
            config::init_config(&target)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "event": "config_created", "path": target.display().to_string() })
                );
            } else {
                println!("Config file created at: {}", target.display());
            }
            Ok(())
        }
        cli::ConfigAction::Show => {
            if json {
                println!("{}", serde_json::to_string_pretty(file_config)?);
            } else {
                if let Some(p) = config_path {
                    println!("# Loaded from: {}\n", p.display());
                } else {
                    println!("# No config file found (using defaults)\n");
                }
                println!("{}", toml::to_string_pretty(file_config)?);
            }
            Ok(())
        }
    }
}
