//! x402-style payment flow configuration.

use logos_messaging_a2a_execution::ExecutionBackend;
use std::sync::Arc;

/// Configuration for x402-style payment flow via [`ExecutionBackend`].
///
/// When configured on a node:
/// - **Sending**: if `auto_pay` is true, `backend.pay()` is called before
///   sending and the TX hash is attached to the outgoing task.
/// - **Receiving**: if `required_amount > 0`, incoming tasks without a valid
///   `payment_tx` (or with insufficient `payment_amount`) are rejected.
pub struct PaymentConfig {
    /// Execution backend used for `pay()` / `balance()` / `verify_transfer()` calls.
    pub backend: Arc<dyn ExecutionBackend>,
    /// Minimum payment required to accept an incoming task. 0 = no requirement.
    pub required_amount: u64,
    /// Automatically pay when sending tasks.
    pub auto_pay: bool,
    /// Amount to auto-pay per outgoing task (only used when `auto_pay` is true).
    pub auto_pay_amount: u64,
    /// When true, verify payment tx hashes on-chain via `backend.verify_transfer()`.
    /// When false, only check that `payment_tx` and `payment_amount` are present.
    pub verify_on_chain: bool,
    /// Expected recipient address for on-chain verification (lowercase hex with 0x).
    /// If empty, recipient is not checked (only amount is validated).
    pub receiving_account: String,
}
