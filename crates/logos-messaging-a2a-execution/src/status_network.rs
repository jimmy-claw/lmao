//! Status Network execution backend.
//!
//! Uses JSON-RPC (via `reqwest`) to interact with the Status Network EVM chain.
//! Status Network is gasless and EVM-compatible, making it ideal for frequent
//! low-friction agent operations.
//!
//! # Configuration
//!
//! ```rust
//! use logos_messaging_a2a_execution::StatusNetworkBackend;
//!
//! let backend = StatusNetworkBackend::new(
//!     "https://public.sepolia.rpc.status.network",
//!     "0x438bB48f4E3Dd338e98e3EEf7b5dDc90a1490b8C",
//!     "0xYOUR_TOKEN_CONTRACT",
//! );
//! ```

use async_trait::async_trait;
use logos_messaging_a2a_core::AgentCard;

use crate::{AgentId, ExecutionBackend, TxHash};

/// Default Status Network Sepolia RPC endpoint.
pub const DEFAULT_RPC_URL: &str = "https://public.sepolia.rpc.status.network";

/// Deployed AgentRegistry contract on Status Network Sepolia testnet.
/// Deployed via `forge create` with `--legacy --gas-price 0`.
/// TX: 0xc89dd4622e137bcf2c69685473bea6acd63205703a5fabc1ba641f37c4cdf9b1
pub const SN_TESTNET_AGENT_REGISTRY: &str = "0x438bB48f4E3Dd338e98e3EEf7b5dDc90a1490b8C";

/// Status Network execution backend.
///
/// Provides agent registration, payments, and balance queries via EVM
/// smart contracts deployed on the Status Network.
pub struct StatusNetworkBackend {
    /// JSON-RPC endpoint URL.
    pub rpc_url: String,
    /// Address of the agent registry contract.
    pub registry_contract: String,
    /// Address of the ERC-20 token contract used for agent payments.
    pub token_contract: String,
    /// HTTP client for JSON-RPC calls.
    client: reqwest::Client,
}

impl StatusNetworkBackend {
    /// Create a new Status Network backend.
    ///
    /// # Arguments
    /// * `rpc_url` - JSON-RPC endpoint (use [`DEFAULT_RPC_URL`] for Sepolia testnet)
    /// * `registry_contract` - Address of the deployed agent registry contract
    /// * `token_contract` - Address of the ERC-20 token contract for payments
    pub fn new(rpc_url: &str, registry_contract: &str, token_contract: &str) -> Self {
        Self {
            rpc_url: rpc_url.to_string(),
            registry_contract: registry_contract.to_string(),
            token_contract: token_contract.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Create a backend with the default Sepolia testnet RPC.
    pub fn sepolia(registry_contract: &str, token_contract: &str) -> Self {
        Self::new(DEFAULT_RPC_URL, registry_contract, token_contract)
    }

    /// Send a raw JSON-RPC request and return the result field.
    async fn rpc_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        });

        let resp: serde_json::Value = self
            .client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if let Some(error) = resp.get("error") {
            anyhow::bail!("RPC error: {}", error);
        }

        resp.get("result")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Missing result in RPC response"))
    }
}

#[async_trait]
impl ExecutionBackend for StatusNetworkBackend {
    /// Register an agent card on-chain by calling the AgentRegistry contract.
    ///
    /// ABI-encodes a call to `register(bytes32,string,string[],string)` using
    /// `alloy-sol-types` and sends it via `eth_call` to verify it would succeed.
    ///
    /// Note: Full transaction submission requires a signer (future work).
    /// Currently performs an `eth_call` dry-run against the registry contract.
    async fn register_agent(&self, card: &AgentCard) -> anyhow::Result<TxHash> {
        use alloy_primitives::FixedBytes;
        use alloy_sol_types::{sol, SolCall};

        sol! {
            function register(
                bytes32 pubkeyHash,
                string name,
                string[] capabilities,
                string contentTopic
            ) external;
        }

        // Derive a pubkey hash from the agent name (placeholder until real keys)
        let pubkey_hash: FixedBytes<32> = {
            let digest = sha2_hash(card.name.as_bytes());
            FixedBytes::from(digest)
        };

        let capabilities: Vec<String> = card.capabilities.clone();

        let content_topic = format!("/a2a/1/{}/proto", card.name.to_lowercase());

        let call = registerCall {
            pubkeyHash: pubkey_hash,
            name: card.name.clone(),
            capabilities,
            contentTopic: content_topic,
        };

        let calldata = hex::encode(call.abi_encode());

        // Dry-run via eth_call to verify it would succeed
        let result = self
            .rpc_call(
                "eth_call",
                serde_json::json!([{
                    "to": self.registry_contract,
                    "data": format!("0x{}", calldata)
                }, "latest"]),
            )
            .await?;

        log::info!("register_agent dry-run OK for '{}': {}", card.name, result);

        // Return the pubkey hash as the "tx hash" until we have a signer
        Ok(TxHash(pubkey_hash.0))
    }

    /// Send tokens to another agent via the ERC-20 token contract.
    ///
    /// Calls `transfer(address,uint256)` on the token contract.
    /// Currently a stub that verifies RPC connectivity.
    async fn pay(&self, to: &AgentId, amount: u64) -> anyhow::Result<TxHash> {
        // Verify connectivity
        let _chain_id = self.rpc_call("eth_chainId", serde_json::json!([])).await?;

        // TODO: Implement ERC-20 transfer with proper signer.
        // Would encode: token_contract.transfer(to_address, amount)
        let mut hash = [0u8; 32];
        let data = format!("{}:{}", to.0, amount);
        let digest = sha2_hash(data.as_bytes());
        hash.copy_from_slice(&digest);
        Ok(TxHash(hash))
    }

    /// Query the token balance for an agent.
    ///
    /// Calls `balanceOf(address)` on the token contract via `eth_call`.
    async fn balance(&self, agent: &AgentId) -> anyhow::Result<u64> {
        // balanceOf(address) selector = 0x70a08231
        // Pad address to 32 bytes for ABI encoding
        let padded_addr = format!("{:0>64}", agent.0.trim_start_matches("0x"));
        let calldata = format!("0x70a08231{}", padded_addr);

        let result = self
            .rpc_call(
                "eth_call",
                serde_json::json!([{
                "to": self.token_contract,
                "data": calldata
            }, "latest"]),
            )
            .await?;

        // Parse hex result to u64
        let hex_str = result.as_str().unwrap_or("0x0");
        let hex_str = hex_str.trim_start_matches("0x");
        let balance = u64::from_str_radix(hex_str, 16).unwrap_or(0);
        Ok(balance)
    }
}

/// Simple SHA-256 hash using manual implementation (avoids extra dep).
/// In production, use the `sha2` crate which is already a workspace dep.
fn sha2_hash(data: &[u8]) -> [u8; 32] {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    // Placeholder: use a simple hash for deterministic test output.
    // TODO: Replace with sha2::Sha256 from workspace dep.
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    let h = hasher.finish();
    let mut out = [0u8; 32];
    out[..8].copy_from_slice(&h.to_le_bytes());
    out[8..16].copy_from_slice(&h.to_le_bytes());
    out[16..24].copy_from_slice(&h.to_le_bytes());
    out[24..32].copy_from_slice(&h.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_rpc_url_is_sepolia() {
        assert!(DEFAULT_RPC_URL.contains("sepolia"));
    }

    #[test]
    fn new_backend_stores_config() {
        let b = StatusNetworkBackend::new("http://localhost:8545", "0xREG", "0xTOK");
        assert_eq!(b.rpc_url, "http://localhost:8545");
        assert_eq!(b.registry_contract, "0xREG");
        assert_eq!(b.token_contract, "0xTOK");
    }

    #[test]
    fn sepolia_uses_default_rpc() {
        let b = StatusNetworkBackend::sepolia("0xREG", "0xTOK");
        assert_eq!(b.rpc_url, DEFAULT_RPC_URL);
    }
}
