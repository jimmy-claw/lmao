//! LEZ (Logos Execution Zone) execution backend — stub.
//!
//! LEZ provides ZK-verified execution with privacy guarantees.
//! This module is a placeholder for when the LEZ toolchain stabilises.
//!
//! See: <https://github.com/jimmy-claw/lez-registry> (issue #4)

use async_trait::async_trait;
use logos_messaging_a2a_core::AgentCard;

use crate::{AgentId, ExecutionBackend, TxHash};

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
        // TODO: Implement LEZ agent registration via ZK program (issue #4)
        unimplemented!("LEZ execution backend not yet available — tracking in issue #4")
    }

    /// Pay another agent via LEZ tokens. (Not yet implemented — see issue #4.)
    async fn pay(&self, _to: &AgentId, _amount: u64) -> anyhow::Result<TxHash> {
        // TODO: Implement LEZ token transfer (issue #4)
        unimplemented!("LEZ execution backend not yet available — tracking in issue #4")
    }

    /// Query agent balance on LEZ. (Not yet implemented — see issue #4.)
    async fn balance(&self, _agent: &AgentId) -> anyhow::Result<u64> {
        // TODO: Implement LEZ balance query (issue #4)
        unimplemented!("LEZ execution backend not yet available — tracking in issue #4")
    }
}
