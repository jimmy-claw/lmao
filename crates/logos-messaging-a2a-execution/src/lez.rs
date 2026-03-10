//! LEZ (Logos Execution Zone) execution backend — stub.
//!
//! LEZ provides ZK-verified execution with privacy guarantees.
//! This module is a placeholder for when the LEZ toolchain stabilises.
//!
//! See: <https://github.com/jimmy-claw/lez-registry> (issue #4)

use async_trait::async_trait;
use logos_messaging_a2a_core::AgentCard;

use crate::{AgentId, ExecutionBackend, TransferDetails, TxHash};

/// LEZ execution backend (stub).
///
/// Will provide ZK-verified agent registration, payments, and balance
/// queries once the LEZ runtime and SDK are production-ready.
pub struct LezExecutionBackend {
    _private: (),
}

impl LezExecutionBackend {
    /// Create a new LEZ backend.
    ///
    /// Currently a stub — all methods return `unimplemented!()`.
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for LezExecutionBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExecutionBackend for LezExecutionBackend {
    /// Register an agent on the LEZ chain. (Not yet implemented — see issue #4.)
    async fn register_agent(&self, _card: &AgentCard) -> anyhow::Result<TxHash> {
        unimplemented!("LEZ execution backend not yet available — tracking in issue #4")
    }

    /// Pay another agent via LEZ tokens. (Not yet implemented — see issue #4.)
    async fn pay(&self, _to: &AgentId, _amount: u64) -> anyhow::Result<TxHash> {
        unimplemented!("LEZ execution backend not yet available — tracking in issue #4")
    }

    /// Query agent balance on LEZ. (Not yet implemented — see issue #4.)
    async fn balance(&self, _agent: &AgentId) -> anyhow::Result<u64> {
        unimplemented!("LEZ execution backend not yet available — tracking in issue #4")
    }

    /// Verify a transfer on the LEZ chain. (Not yet implemented — see issue #4.)
    async fn verify_transfer(&self, _tx_hash: &str) -> anyhow::Result<TransferDetails> {
        unimplemented!("LEZ execution backend not yet available — tracking in issue #4")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lez_new_creates_instance() {
        let _backend = LezExecutionBackend::new();
    }

    #[test]
    fn lez_default_creates_instance() {
        let _backend = LezExecutionBackend::default();
    }

    #[test]
    fn lez_new_and_default_are_equivalent() {
        // Both constructors should work without panic
        let _a = LezExecutionBackend::new();
        let _b = LezExecutionBackend::default();
    }

    #[tokio::test]
    #[should_panic(expected = "LEZ execution backend not yet available")]
    async fn lez_register_agent_panics() {
        let backend = LezExecutionBackend::new();
        let card = AgentCard {
            name: "test".into(),
            description: "test agent".into(),
            version: "0.1.0".into(),
            capabilities: vec![],
            public_key: "0xdead".into(),
            intro_bundle: None,
        };
        let _ = backend.register_agent(&card).await;
    }

    #[tokio::test]
    #[should_panic(expected = "LEZ execution backend not yet available")]
    async fn lez_pay_panics() {
        let backend = LezExecutionBackend::new();
        let _ = backend.pay(&AgentId("0x1234".into()), 100).await;
    }

    #[tokio::test]
    #[should_panic(expected = "LEZ execution backend not yet available")]
    async fn lez_balance_panics() {
        let backend = LezExecutionBackend::new();
        let _ = backend.balance(&AgentId("0x1234".into())).await;
    }

    #[tokio::test]
    #[should_panic(expected = "LEZ execution backend not yet available")]
    async fn lez_verify_transfer_panics() {
        let backend = LezExecutionBackend::new();
        let _ = backend.verify_transfer("0xdeadbeef").await;
    }
}
