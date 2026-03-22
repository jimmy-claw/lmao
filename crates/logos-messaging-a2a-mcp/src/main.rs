//! MCP Bridge Server for Logos Messaging A2A
//!
//! Exposes discovered Waku A2A agents as MCP tools.
//! Each agent on the network becomes a callable tool in Claude Desktop, Cursor, etc.
//!
//! Architecture:
//!   MCP Host (Claude) → stdio → logos-messaging-a2a-mcp → Waku → Agent Fleet
//!
//! Usage:
//!   logos-messaging-a2a-mcp --waku-url http://localhost:8645
//!
//! In Claude Desktop's config:
//!   { "mcpServers": { "logos-agents": { "command": "logos-messaging-a2a-mcp", "args": ["--waku-url", "http://..."] } } }

mod config;
mod server;
mod state;

use anyhow::Result;
use clap::Parser;
use logos_messaging_a2a_transport::nwaku_rest::LogosMessagingTransport;
use rmcp::{transport::stdio, ServiceExt};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::Cli;
use crate::server::LogosA2ABridge;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    tracing::info!(
        "Starting Logos A2A MCP bridge (waku: {}, timeout: {}s)",
        cli.waku_url,
        cli.timeout
    );

    let bridge = LogosA2ABridge::new(&cli.waku_url, cli.timeout);

    {
        let node: tokio::sync::RwLockReadGuard<
            logos_messaging_a2a_node::WakuA2ANode<LogosMessagingTransport>,
        > = bridge.node.read().await;
        if let Err(e) = node.announce().await {
            tracing::warn!("Failed to announce bridge on network: {e}");
        }
    }

    let service = bridge.serve(stdio()).await?;
    service.waiting().await?;

    Ok(())
}
