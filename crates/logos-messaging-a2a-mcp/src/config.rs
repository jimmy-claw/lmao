use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "logos-messaging-a2a-mcp",
    about = "MCP bridge for Logos A2A agents"
)]
pub(crate) struct Cli {
    /// nwaku REST API URL
    #[arg(long, default_value = "http://localhost:8645")]
    pub waku_url: String,

    /// How long (seconds) to wait for agent responses
    #[arg(long, default_value_t = 30)]
    pub timeout: u64,
}
