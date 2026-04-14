//! LEZ (Logos Execution Zone) execution backend.
//!
//! Communicates with the `lez_multisig_module` Logos Core module via its
//! JSON-RPC HTTP bridge. The module is loaded into `logos_host` and exposes
//! wallet operations (sign, submit tx, balance queries) over QtRO — this
//! backend hits the HTTP endpoint that the module bridge publishes.
//!
//! # Architecture
//!
//! ```text
//! LMAO agent (Rust)
//!   └─ LezExecutionBackend ──HTTP/JSON-RPC──▶ lez_multisig_module bridge
//!       (this crate)                            (Logos Core QtRO module)
//!                                                 └─▶ LEZ sequencer
//! ```
//!
//! # Spending threshold
//!
//! The owner can configure a maximum cumulative spend. The backend tracks
//! total tokens sent and rejects `pay()` calls that would exceed the limit.
//!
//! See: <https://github.com/jimmy-claw/lez-multisig-framework>

use async_trait::async_trait;
use logos_messaging_a2a_core::AgentCard;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{AgentId, ExecutionBackend, ExecutionError, TransferDetails, TxHash};

/// Default local endpoint for the LEZ module JSON-RPC bridge.
pub const DEFAULT_LEZ_BRIDGE_URL: &str = "http://127.0.0.1:5292";

/// LEZ execution backend.
///
/// Connects to the `lez_multisig_module` Logos Core module via its local
/// JSON-RPC bridge for agent registration, token transfers, and balance
/// queries on the Logos Execution Zone.
pub struct LezExecutionBackend {
    /// JSON-RPC bridge endpoint URL.
    bridge_url: String,
    /// HTTP client for bridge calls.
    client: reqwest::Client,
    /// Agent wallet address on LEZ (hex with 0x prefix).
    wallet_address: String,
    /// Maximum cumulative spend allowed (0 = unlimited).
    spending_limit: u64,
    /// Cumulative spend so far (atomic for Send+Sync).
    total_spent: AtomicU64,
}

impl LezExecutionBackend {
    /// Create a new LEZ backend.
    ///
    /// # Arguments
    /// * `bridge_url` - URL of the lez_multisig_module JSON-RPC bridge
    /// * `wallet_address` - This agent's LEZ wallet address (hex, 0x-prefixed)
    /// * `spending_limit` - Max cumulative spend in token base units (0 = unlimited)
    pub fn new(bridge_url: &str, wallet_address: &str, spending_limit: u64) -> Self {
        Self {
            bridge_url: bridge_url.to_string(),
            client: reqwest::Client::new(),
            wallet_address: wallet_address.to_string(),
            spending_limit,
            total_spent: AtomicU64::new(0),
        }
    }

    /// Create a backend with the default local bridge endpoint.
    pub fn local(wallet_address: &str, spending_limit: u64) -> Self {
        Self::new(DEFAULT_LEZ_BRIDGE_URL, wallet_address, spending_limit)
    }

    /// Return cumulative tokens spent so far.
    pub fn total_spent(&self) -> u64 {
        self.total_spent.load(Ordering::Relaxed)
    }

    /// Return the configured spending limit (0 = unlimited).
    pub fn spending_limit(&self) -> u64 {
        self.spending_limit
    }

    /// Return remaining spending allowance. Returns `u64::MAX` if unlimited.
    pub fn remaining_allowance(&self) -> u64 {
        if self.spending_limit == 0 {
            return u64::MAX;
        }
        self.spending_limit.saturating_sub(self.total_spent.load(Ordering::Relaxed))
    }

    /// Send a JSON-RPC request to the LEZ module bridge.
    async fn rpc_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ExecutionError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        });

        let resp: serde_json::Value = self
            .client
            .post(&self.bridge_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ExecutionError::Rpc(format!("LEZ bridge unreachable: {}", e)))?
            .json()
            .await
            .map_err(|e| ExecutionError::Rpc(format!("LEZ bridge bad response: {}", e)))?;

        if let Some(error) = resp.get("error") {
            return Err(ExecutionError::Rpc(format!("LEZ module error: {}", error)));
        }

        resp.get("result")
            .cloned()
            .ok_or_else(|| ExecutionError::Rpc("Missing result in LEZ bridge response".into()))
    }

    /// Check and update spending threshold. Returns error if limit exceeded.
    fn check_spending_limit(&self, amount: u64) -> Result<(), ExecutionError> {
        if self.spending_limit == 0 {
            return Ok(());
        }
        let current = self.total_spent.load(Ordering::Relaxed);
        let new_total = current.checked_add(amount).ok_or_else(|| {
            ExecutionError::Other("Spending overflow".into())
        })?;
        if new_total > self.spending_limit {
            return Err(ExecutionError::Other(format!(
                "Spending limit exceeded: {} + {} > {} limit",
                current, amount, self.spending_limit
            )));
        }
        Ok(())
    }

    /// Record a successful spend.
    fn record_spend(&self, amount: u64) {
        self.total_spent.fetch_add(amount, Ordering::Relaxed);
    }
}

#[async_trait]
impl ExecutionBackend for LezExecutionBackend {
    /// Register an agent on LEZ via the multisig module.
    ///
    /// Calls `lez_registerAgent` on the module bridge with the agent's
    /// public key, name, capabilities, and version.
    async fn register_agent(&self, card: &AgentCard) -> Result<TxHash, ExecutionError> {
        let result = self
            .rpc_call(
                "lez_registerAgent",
                serde_json::json!({
                    "publicKey": card.public_key,
                    "name": card.name,
                    "capabilities": card.capabilities,
                    "version": card.version,
                    "wallet": self.wallet_address,
                }),
            )
            .await?;

        parse_tx_hash(&result)
    }

    /// Transfer LEZ tokens to another agent, enforcing spending threshold.
    ///
    /// Calls `lez_sendTransaction` on the module bridge. The multisig module
    /// handles signing and submission to the LEZ sequencer.
    async fn pay(&self, to: &AgentId, amount: u64) -> Result<TxHash, ExecutionError> {
        self.check_spending_limit(amount)?;

        let result = self
            .rpc_call(
                "lez_sendTransaction",
                serde_json::json!({
                    "from": self.wallet_address,
                    "to": to.0,
                    "amount": amount,
                }),
            )
            .await?;

        let tx_hash = parse_tx_hash(&result)?;
        self.record_spend(amount);
        Ok(tx_hash)
    }

    /// Query token balance for an agent on LEZ.
    async fn balance(&self, agent: &AgentId) -> Result<u64, ExecutionError> {
        let result = self
            .rpc_call(
                "lez_getBalance",
                serde_json::json!({ "address": agent.0 }),
            )
            .await?;

        result
            .as_str()
            .and_then(|s| {
                let s = s.trim_start_matches("0x");
                u64::from_str_radix(s, 16).ok()
            })
            .or_else(|| result.as_u64())
            .ok_or_else(|| {
                ExecutionError::Other(format!("Invalid balance response: {}", result))
            })
    }

    /// Verify a transfer on LEZ by querying the module bridge.
    async fn verify_transfer(&self, tx_hash: &str) -> Result<TransferDetails, ExecutionError> {
        let tx_hash = if tx_hash.starts_with("0x") {
            tx_hash.to_string()
        } else {
            format!("0x{}", tx_hash)
        };

        let result = self
            .rpc_call(
                "lez_getTransactionReceipt",
                serde_json::json!({ "hash": tx_hash }),
            )
            .await?;

        if result.is_null() {
            return Err(ExecutionError::Other(format!(
                "Transaction {} not found on LEZ",
                tx_hash
            )));
        }

        let status = result
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("failed");
        if status != "confirmed" && status != "0x1" {
            return Err(ExecutionError::Other(format!(
                "Transaction {} failed (status: {})",
                tx_hash, status
            )));
        }

        let from = result
            .get("from")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let to = result
            .get("to")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let amount = result
            .get("amount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let block_number = result
            .get("blockNumber")
            .and_then(|v| {
                v.as_u64().or_else(|| {
                    v.as_str().and_then(|s| {
                        u64::from_str_radix(s.trim_start_matches("0x"), 16).ok()
                    })
                })
            })
            .unwrap_or(0);

        Ok(TransferDetails {
            from,
            to,
            amount,
            block_number,
        })
    }
}

/// Parse a TxHash from a JSON-RPC result value.
///
/// Accepts either a hex string (with or without 0x prefix) or a JSON object
/// with a `"txHash"` field.
fn parse_tx_hash(value: &serde_json::Value) -> Result<TxHash, ExecutionError> {
    let hex_str = value
        .as_str()
        .or_else(|| value.get("txHash").and_then(|v| v.as_str()))
        .ok_or_else(|| ExecutionError::Other("Missing txHash in LEZ response".into()))?;

    let hex_str = hex_str.trim_start_matches("0x");
    let bytes = hex::decode(hex_str)
        .map_err(|e| ExecutionError::Other(format!("Invalid tx hash hex: {}", e)))?;

    if bytes.len() != 32 {
        return Err(ExecutionError::Other(format!(
            "Expected 32-byte tx hash, got {} bytes",
            bytes.len()
        )));
    }

    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Ok(TxHash(hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_stores_config() {
        let b = LezExecutionBackend::new("http://localhost:9999", "0xwallet", 1000);
        assert_eq!(b.bridge_url, "http://localhost:9999");
        assert_eq!(b.wallet_address, "0xwallet");
        assert_eq!(b.spending_limit, 1000);
        assert_eq!(b.total_spent(), 0);
    }

    #[test]
    fn local_uses_default_url() {
        let b = LezExecutionBackend::local("0xwallet", 500);
        assert_eq!(b.bridge_url, DEFAULT_LEZ_BRIDGE_URL);
        assert_eq!(b.spending_limit, 500);
    }

    #[test]
    fn remaining_allowance_unlimited() {
        let b = LezExecutionBackend::new("http://x", "0xw", 0);
        assert_eq!(b.remaining_allowance(), u64::MAX);
    }

    #[test]
    fn remaining_allowance_with_limit() {
        let b = LezExecutionBackend::new("http://x", "0xw", 1000);
        assert_eq!(b.remaining_allowance(), 1000);
        b.total_spent.store(300, Ordering::Relaxed);
        assert_eq!(b.remaining_allowance(), 700);
    }

    #[test]
    fn remaining_allowance_saturates_at_zero() {
        let b = LezExecutionBackend::new("http://x", "0xw", 100);
        b.total_spent.store(200, Ordering::Relaxed);
        assert_eq!(b.remaining_allowance(), 0);
    }

    #[test]
    fn check_spending_limit_allows_within_budget() {
        let b = LezExecutionBackend::new("http://x", "0xw", 1000);
        assert!(b.check_spending_limit(500).is_ok());
        assert!(b.check_spending_limit(1000).is_ok());
    }

    #[test]
    fn check_spending_limit_rejects_over_budget() {
        let b = LezExecutionBackend::new("http://x", "0xw", 1000);
        b.total_spent.store(800, Ordering::Relaxed);
        let err = b.check_spending_limit(300).unwrap_err();
        assert!(err.to_string().contains("Spending limit exceeded"));
    }

    #[test]
    fn check_spending_limit_unlimited_allows_any() {
        let b = LezExecutionBackend::new("http://x", "0xw", 0);
        assert!(b.check_spending_limit(u64::MAX).is_ok());
    }

    #[test]
    fn record_spend_increments_total() {
        let b = LezExecutionBackend::new("http://x", "0xw", 0);
        b.record_spend(100);
        b.record_spend(200);
        assert_eq!(b.total_spent(), 300);
    }

    #[test]
    fn parse_tx_hash_from_hex_string() {
        let hex = format!("0x{}", "ab".repeat(32));
        let val = serde_json::json!(hex);
        let hash = parse_tx_hash(&val).unwrap();
        assert_eq!(hash, TxHash([0xab; 32]));
    }

    #[test]
    fn parse_tx_hash_without_prefix() {
        let hex = "cd".repeat(32);
        let val = serde_json::json!(hex);
        let hash = parse_tx_hash(&val).unwrap();
        assert_eq!(hash, TxHash([0xcd; 32]));
    }

    #[test]
    fn parse_tx_hash_from_object() {
        let hex = format!("0x{}", "ef".repeat(32));
        let val = serde_json::json!({ "txHash": hex });
        let hash = parse_tx_hash(&val).unwrap();
        assert_eq!(hash, TxHash([0xef; 32]));
    }

    #[test]
    fn parse_tx_hash_wrong_length() {
        let val = serde_json::json!("0xdeadbeef");
        let err = parse_tx_hash(&val).unwrap_err();
        assert!(err.to_string().contains("32-byte"));
    }

    #[test]
    fn parse_tx_hash_invalid_hex() {
        let val = serde_json::json!("0xNOTHEX!");
        let err = parse_tx_hash(&val).unwrap_err();
        assert!(err.to_string().contains("Invalid tx hash hex"));
    }

    #[test]
    fn parse_tx_hash_missing_field() {
        let val = serde_json::json!(42);
        let err = parse_tx_hash(&val).unwrap_err();
        assert!(err.to_string().contains("Missing txHash"));
    }
}
