//! # ExecutionBackend
//!
//! Abstracts on-chain execution for LMAO agents. Agents can register their
//! [`AgentCard`] on-chain, check balances, and make payments — without coupling
//! to a specific blockchain.
//!
//! Two backends are planned:
//! - **Status Network** (`status-network` feature, default) — EVM-compatible, gasless
//! - **LEZ** (`lez` feature) — ZK-verified execution with privacy guarantees

use async_trait::async_trait;
use logos_messaging_a2a_core::AgentCard;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Newtype wrapper for an agent identity (secp256k1 compressed public key hex).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&AgentCard> for AgentId {
    fn from(card: &AgentCard) -> Self {
        AgentId(card.public_key.clone())
    }
}

/// A 32-byte transaction hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TxHash(pub [u8; 32]);

impl fmt::Display for TxHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}

/// Details of a verified on-chain token transfer.
///
/// Returned by [`ExecutionBackend::verify_transfer`] after querying the chain
/// to confirm a payment transaction is valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferDetails {
    /// Sender address (lowercase hex with 0x prefix).
    pub from: String,
    /// Recipient address (lowercase hex with 0x prefix).
    pub to: String,
    /// Amount transferred (in token base units).
    pub amount: u64,
    /// Block number the transfer was included in.
    pub block_number: u64,
}

/// Trait for on-chain agent operations.
///
/// Implementations abstract over different execution layers (Status Network,
/// LEZ, etc.) so agents can register, pay, and query balances without knowing
/// the underlying chain.
#[async_trait]
pub trait ExecutionBackend: Send + Sync {
    /// Register an agent's card on-chain for permanent, discoverable identity.
    async fn register_agent(&self, card: &AgentCard) -> anyhow::Result<TxHash>;

    /// Transfer `amount` tokens to another agent.
    async fn pay(&self, to: &AgentId, amount: u64) -> anyhow::Result<TxHash>;

    /// Query the token balance for an agent.
    async fn balance(&self, agent: &AgentId) -> anyhow::Result<u64>;

    /// Verify a transaction hash corresponds to a valid ERC-20 transfer.
    ///
    /// Queries the chain for the transaction receipt, decodes the `Transfer`
    /// event log, and returns the transfer details. Returns an error if the
    /// transaction doesn't exist, failed, or contains no valid transfer event.
    async fn verify_transfer(&self, tx_hash: &str) -> anyhow::Result<TransferDetails>;
}

#[cfg(feature = "status-network")]
pub mod status_network;

#[cfg(feature = "lez")]
pub mod lez;

// Re-exports for convenience
#[cfg(feature = "status-network")]
pub use status_network::StatusNetworkBackend;

#[cfg(feature = "lez")]
pub use lez::LezExecutionBackend;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_id_from_card() {
        let card = AgentCard {
            name: "test".into(),
            description: "test agent".into(),
            version: "0.1.0".into(),
            capabilities: vec![],
            public_key: "0x1234abcd".into(),
            intro_bundle: None,
        };
        let id = AgentId::from(&card);
        assert_eq!(id.0, "0x1234abcd");
    }

    #[test]
    fn tx_hash_display() {
        let hash = TxHash([0xab; 32]);
        assert_eq!(hash.to_string().len(), 64);
        assert!(hash.to_string().starts_with("abab"));
    }

    #[test]
    fn transfer_details_equality() {
        let d1 = TransferDetails {
            from: "0xaaa".into(),
            to: "0xbbb".into(),
            amount: 100,
            block_number: 42,
        };
        let d2 = d1.clone();
        assert_eq!(d1, d2);
    }

    struct MockBackend;

    #[async_trait]
    impl ExecutionBackend for MockBackend {
        async fn register_agent(&self, _card: &AgentCard) -> anyhow::Result<TxHash> {
            Ok(TxHash([0; 32]))
        }
        async fn pay(&self, _to: &AgentId, _amount: u64) -> anyhow::Result<TxHash> {
            Ok(TxHash([1; 32]))
        }
        async fn balance(&self, _agent: &AgentId) -> anyhow::Result<u64> {
            Ok(42)
        }
        async fn verify_transfer(&self, _tx_hash: &str) -> anyhow::Result<TransferDetails> {
            Ok(TransferDetails {
                from: "0xsender".into(),
                to: "0xrecipient".into(),
                amount: 100,
                block_number: 1,
            })
        }
    }

    #[tokio::test]
    async fn mock_backend_register() {
        let backend = MockBackend;
        let card = AgentCard {
            name: "a".into(),
            description: "b".into(),
            version: "0.1.0".into(),
            capabilities: vec![],
            public_key: "0xdead".into(),
            intro_bundle: None,
        };
        let tx = backend.register_agent(&card).await.unwrap();
        assert_eq!(tx, TxHash([0; 32]));
    }

    #[tokio::test]
    async fn mock_backend_pay() {
        let backend = MockBackend;
        let tx = backend.pay(&AgentId("0xbeef".into()), 100).await.unwrap();
        assert_eq!(tx, TxHash([1; 32]));
    }

    #[tokio::test]
    async fn mock_backend_balance() {
        let backend = MockBackend;
        let bal = backend.balance(&AgentId("0xcafe".into())).await.unwrap();
        assert_eq!(bal, 42);
    }

    #[tokio::test]
    async fn mock_backend_verify_transfer() {
        let backend = MockBackend;
        let details = backend.verify_transfer("0xdeadbeef").await.unwrap();
        assert_eq!(details.amount, 100);
        assert_eq!(details.from, "0xsender");
        assert_eq!(details.to, "0xrecipient");
    }
}
