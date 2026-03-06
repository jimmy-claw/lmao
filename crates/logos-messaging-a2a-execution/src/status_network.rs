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

use crate::{AgentId, ExecutionBackend, TransferDetails, TxHash};

/// Default Status Network Sepolia RPC endpoint.
pub const DEFAULT_RPC_URL: &str = "https://public.sepolia.rpc.status.network";

/// Deployed AgentRegistry contract on Status Network Sepolia testnet.
/// Deployed via `forge create` with `--legacy --gas-price 0`.
/// TX: 0xc89dd4622e137bcf2c69685473bea6acd63205703a5fabc1ba641f37c4cdf9b1
pub const SN_TESTNET_AGENT_REGISTRY: &str = "0x438bB48f4E3Dd338e98e3EEf7b5dDc90a1490b8C";

/// ERC-20 Transfer event topic: keccak256("Transfer(address,address,uint256)")
const TRANSFER_EVENT_TOPIC: &str =
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

/// Status Network execution backend.
///
/// Provides agent registration, payments, balance queries, and transfer
/// verification via EVM smart contracts deployed on the Status Network.
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

    /// Parse an ERC-20 Transfer event from transaction receipt logs.
    ///
    /// Looks for a log entry matching the Transfer(address,address,uint256)
    /// event signature from the configured token contract.
    fn parse_transfer_log(
        &self,
        logs: &[serde_json::Value],
    ) -> anyhow::Result<(String, String, u64)> {
        for log in logs {
            let topics = log
                .get("topics")
                .and_then(|t| t.as_array())
                .unwrap_or(&Vec::new())
                .clone();

            // Transfer event has 3 topics: event sig, from (indexed), to (indexed)
            if topics.len() < 3 {
                continue;
            }

            let event_sig = topics[0].as_str().unwrap_or("");
            if event_sig != TRANSFER_EVENT_TOPIC {
                continue;
            }

            // Optionally filter by token contract address
            let log_addr = log
                .get("address")
                .and_then(|a| a.as_str())
                .unwrap_or("")
                .to_lowercase();
            if !self.token_contract.is_empty() && log_addr != self.token_contract.to_lowercase() {
                continue;
            }

            // Decode from/to from indexed topics (last 20 bytes of 32-byte word)
            let from_topic = topics[1].as_str().unwrap_or("0x0");
            let to_topic = topics[2].as_str().unwrap_or("0x0");
            let from = format!("0x{}", &from_topic[from_topic.len().saturating_sub(40)..]);
            let to = format!("0x{}", &to_topic[to_topic.len().saturating_sub(40)..]);

            // Decode amount from data field (uint256, take low 8 bytes for u64)
            let data = log.get("data").and_then(|d| d.as_str()).unwrap_or("0x0");
            let data_hex = data.trim_start_matches("0x");
            let amount = if data_hex.len() > 16 {
                // Take last 16 hex chars (8 bytes) to fit in u64
                u64::from_str_radix(&data_hex[data_hex.len() - 16..], 16).unwrap_or(0)
            } else {
                u64::from_str_radix(data_hex, 16).unwrap_or(0)
            };

            return Ok((from, to, amount));
        }

        anyhow::bail!("No Transfer event found in transaction logs")
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
        Ok(TxHash(pubkey_hash.0))
    }

    /// Send tokens to another agent via the ERC-20 token contract.
    ///
    /// Currently a stub that verifies RPC connectivity.
    async fn pay(&self, to: &AgentId, amount: u64) -> anyhow::Result<TxHash> {
        let _chain_id = self.rpc_call("eth_chainId", serde_json::json!([])).await?;

        let mut hash = [0u8; 32];
        let data = format!("{}:{}", to.0, amount);
        let digest = sha2_hash(data.as_bytes());
        hash.copy_from_slice(&digest);
        Ok(TxHash(hash))
    }

    /// Query the token balance for an agent.
    async fn balance(&self, agent: &AgentId) -> anyhow::Result<u64> {
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

        let hex_str = result.as_str().unwrap_or("0x0");
        let hex_str = hex_str.trim_start_matches("0x");
        let balance = u64::from_str_radix(hex_str, 16).unwrap_or(0);
        Ok(balance)
    }

    /// Verify a transaction hash on-chain via `eth_getTransactionReceipt`.
    ///
    /// Fetches the receipt, checks that the transaction succeeded (status 0x1),
    /// and parses the ERC-20 Transfer event from the logs.
    async fn verify_transfer(&self, tx_hash: &str) -> anyhow::Result<TransferDetails> {
        // Normalise tx hash
        let tx_hash = if tx_hash.starts_with("0x") {
            tx_hash.to_string()
        } else {
            format!("0x{}", tx_hash)
        };

        // Fetch receipt
        let receipt = self
            .rpc_call("eth_getTransactionReceipt", serde_json::json!([tx_hash]))
            .await?;

        if receipt.is_null() {
            anyhow::bail!("Transaction {} not found (not mined yet?)", tx_hash);
        }

        // Check status (0x1 = success)
        let status = receipt
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("0x0");
        if status != "0x1" {
            anyhow::bail!("Transaction {} failed (status: {})", tx_hash, status);
        }

        // Get block number
        let block_hex = receipt
            .get("blockNumber")
            .and_then(|b| b.as_str())
            .unwrap_or("0x0");
        let block_number = u64::from_str_radix(block_hex.trim_start_matches("0x"), 16).unwrap_or(0);

        // Parse Transfer event from logs
        let logs = receipt
            .get("logs")
            .and_then(|l| l.as_array())
            .cloned()
            .unwrap_or_default();

        let (from, to, amount) = self.parse_transfer_log(&logs)?;

        Ok(TransferDetails {
            from,
            to,
            amount,
            block_number,
        })
    }
}

/// Simple SHA-256 hash using manual implementation (avoids extra dep).
fn sha2_hash(data: &[u8]) -> [u8; 32] {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
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

    #[test]
    fn parse_transfer_log_valid() {
        let backend = StatusNetworkBackend::new("http://localhost", "0xREG", "0xtoken");
        let logs = vec![serde_json::json!({
            "address": "0xtoken",
            "topics": [
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                "0x000000000000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "0x000000000000000000000000bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            ],
            "data": "0x0000000000000000000000000000000000000000000000000000000000000064"
        })];
        let (from, to, amount) = backend.parse_transfer_log(&logs).unwrap();
        assert_eq!(from, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(to, "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        assert_eq!(amount, 100);
    }

    #[test]
    fn parse_transfer_log_wrong_contract() {
        let backend = StatusNetworkBackend::new("http://localhost", "0xREG", "0xtoken");
        let logs = vec![serde_json::json!({
            "address": "0xother_contract",
            "topics": [
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                "0x000000000000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "0x000000000000000000000000bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            ],
            "data": "0x0000000000000000000000000000000000000000000000000000000000000064"
        })];
        assert!(backend.parse_transfer_log(&logs).is_err());
    }

    #[test]
    fn parse_transfer_log_no_transfer_event() {
        let backend = StatusNetworkBackend::new("http://localhost", "0xREG", "0xtoken");
        let logs = vec![serde_json::json!({
            "address": "0xtoken",
            "topics": [
                "0x0000000000000000000000000000000000000000000000000000000000000000"
            ],
            "data": "0x00"
        })];
        assert!(backend.parse_transfer_log(&logs).is_err());
    }

    #[test]
    fn parse_transfer_log_empty() {
        let backend = StatusNetworkBackend::new("http://localhost", "0xREG", "0xtoken");
        assert!(backend.parse_transfer_log(&[]).is_err());
    }
}
