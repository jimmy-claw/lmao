use anyhow::{Context, Result};
use logos_messaging_a2a_core::Task;
use logos_messaging_a2a_execution::AgentId;
use logos_messaging_a2a_transport::Transport;

use crate::WakuA2ANode;

impl<T: Transport> WakuA2ANode<T> {
    /// If auto-pay is enabled, call `backend.pay()` and attach proof to the task.
    pub async fn maybe_auto_pay(&self, task: &Task) -> Result<Task> {
        if let Some(ref pay_cfg) = self.payment {
            if pay_cfg.auto_pay && pay_cfg.auto_pay_amount > 0 {
                let recipient = AgentId(task.to.clone());
                let tx_hash = pay_cfg
                    .backend
                    .pay(&recipient, pay_cfg.auto_pay_amount)
                    .await
                    .context("auto-pay failed")?;
                let mut task = task.clone();
                task.payment_tx = Some(tx_hash.to_string());
                task.payment_amount = Some(pay_cfg.auto_pay_amount);
                return Ok(task);
            }
        }
        Ok(task.clone())
    }

    /// Check that an incoming task satisfies the payment requirement.
    ///
    /// When `verify_on_chain` is true, queries the chain via
    /// `backend.verify_transfer()` to confirm:
    /// 1. The tx hash exists and succeeded
    /// 2. The transfer amount meets the minimum requirement
    /// 3. The recipient matches `receiving_account` (if configured)
    /// 4. The tx hash hasn't been seen before (replay protection)
    ///
    /// Returns `true` if the task is accepted, `false` if rejected.
    pub(crate) async fn verify_payment(&self, task: &Task) -> bool {
        let pay_cfg = match &self.payment {
            Some(cfg) => cfg,
            None => return true,
        };

        if pay_cfg.required_amount == 0 {
            return true;
        }

        let tx_hash = match &task.payment_tx {
            Some(tx) if !tx.is_empty() => tx.clone(),
            _ => {
                tracing::warn!(
                    task_id = %task.id,
                    required_amount = pay_cfg.required_amount,
                    "Rejecting task — no payment tx hash provided"
                );
                return false;
            }
        };

        // Check for replayed tx hashes
        {
            let seen = self.seen_tx_hashes.lock().unwrap();
            if seen.contains(&tx_hash) {
                tracing::warn!(
                    task_id = %task.id,
                    tx_hash = %tx_hash,
                    "Rejecting task — replayed payment tx"
                );
                return false;
            }
        }

        if !pay_cfg.verify_on_chain {
            // Offline check: trust the claimed amount
            match task.payment_amount {
                Some(amount) if amount >= pay_cfg.required_amount => {
                    self.seen_tx_hashes.lock().unwrap().insert(tx_hash);
                    true
                }
                _ => {
                    tracing::warn!(
                        task_id = %task.id,
                        required = pay_cfg.required_amount,
                        got = ?task.payment_amount,
                        "Rejecting task — insufficient payment"
                    );
                    false
                }
            }
        } else {
            // On-chain verification
            match pay_cfg.backend.verify_transfer(&tx_hash).await {
                Ok(details) => {
                    if details.amount < pay_cfg.required_amount {
                        tracing::warn!(
                            task_id = %task.id,
                            on_chain_amount = details.amount,
                            required = pay_cfg.required_amount,
                            "Rejecting task — on-chain amount below required"
                        );
                        return false;
                    }
                    if !pay_cfg.receiving_account.is_empty()
                        && details.to.to_lowercase() != pay_cfg.receiving_account.to_lowercase()
                    {
                        tracing::warn!(
                            task_id = %task.id,
                            actual_recipient = %details.to,
                            expected_recipient = %pay_cfg.receiving_account,
                            "Rejecting task — payment sent to wrong address"
                        );
                        return false;
                    }
                    // Mark as seen to prevent replay
                    self.seen_tx_hashes.lock().unwrap().insert(tx_hash);
                    true
                }
                Err(e) => {
                    tracing::error!(
                        task_id = %task.id,
                        error = %e,
                        "Rejecting task — on-chain verification failed"
                    );
                    false
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::payment::PaymentConfig;
    use crate::tasks::test_support::{
        fast_config, FailingVerifyBackend, MockExecutionBackend, MockTransport, VerifyingBackend,
    };
    use crate::WakuA2ANode;
    use logos_messaging_a2a_core::Task;
    use logos_messaging_a2a_execution::{ExecutionBackend, TransferDetails};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_task_with_payment_attached() {
        let mut task = Task::new("02aa", "03bb", "pay me");
        task.payment_tx = Some("abcd1234".to_string());
        task.payment_amount = Some(100);

        // Serialize and deserialize to verify payment fields survive the wire
        let json = serde_json::to_string(&task).unwrap();
        let deserialized: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.payment_tx, Some("abcd1234".to_string()));
        assert_eq!(deserialized.payment_amount, Some(100));
    }

    #[tokio::test]
    async fn test_task_rejected_without_payment() {
        let transport = MockTransport::new();
        let backend = Arc::new(MockExecutionBackend);

        // Receiver requires payment of 50
        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver agent",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend: backend.clone(),
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: false,
            receiving_account: String::new(),
        });
        let recipient_pubkey = receiver.pubkey().to_string();

        // Lazy-subscribe
        let _ = receiver.poll_tasks().await.unwrap();

        // Sender does NOT pay
        let sender = WakuA2ANode::with_config(
            "sender",
            "sender agent",
            vec![],
            transport.clone(),
            fast_config(),
        );

        let task = Task::new(sender.pubkey(), &recipient_pubkey, "free ride");
        sender.send_task(&task).await.unwrap();

        // Receiver should reject the unpaid task
        let received = receiver.poll_tasks().await.unwrap();
        assert!(received.is_empty(), "unpaid task should be rejected");
    }

    #[tokio::test]
    async fn test_auto_pay_on_send() {
        let transport = MockTransport::new();
        let backend = Arc::new(MockExecutionBackend);

        // Receiver requires payment and listens
        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver agent",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend: backend.clone(),
            required_amount: 100,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: false,
            receiving_account: String::new(),
        });
        let recipient_pubkey = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        // Sender with auto-pay enabled
        let sender = WakuA2ANode::with_config(
            "sender",
            "sender agent",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend: backend.clone(),
            required_amount: 0,
            auto_pay: true,
            auto_pay_amount: 100,
            verify_on_chain: false,
            receiving_account: String::new(),
        });

        let task = Task::new(sender.pubkey(), &recipient_pubkey, "paid task");
        sender.send_task(&task).await.unwrap();

        // Receiver should accept the auto-paid task
        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(received.len(), 1);
        assert!(received[0].payment_tx.is_some(), "should have TX hash");
        assert_eq!(received[0].payment_amount, Some(100));
        assert_eq!(received[0].text(), Some("paid task"));
    }

    #[tokio::test]
    async fn test_replay_protection_rejects_duplicate_tx() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(VerifyingBackend {
            details: TransferDetails {
                from: "0xsender".into(),
                to: "0xrecipient".into(),
                amount: 100,
                block_number: 1,
            },
        });

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver agent",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend: backend.clone(),
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: false,
            receiving_account: String::new(),
        });
        let recipient_pubkey = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender = WakuA2ANode::with_config(
            "sender",
            "sender agent",
            vec![],
            transport.clone(),
            fast_config(),
        );

        // First task with tx hash — accepted
        let mut task1 = Task::new(sender.pubkey(), &recipient_pubkey, "first payment");
        task1.payment_tx = Some("0xabc123".to_string());
        task1.payment_amount = Some(100);
        sender.send_task(&task1).await.unwrap();
        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(received.len(), 1, "first use of tx should be accepted");

        // Same tx hash again — rejected (replay)
        let mut task2 = Task::new(sender.pubkey(), &recipient_pubkey, "replay attempt");
        task2.payment_tx = Some("0xabc123".to_string());
        task2.payment_amount = Some(100);
        sender.send_task(&task2).await.unwrap();
        let received = receiver.poll_tasks().await.unwrap();
        assert!(received.is_empty(), "replayed tx should be rejected");
    }

    #[tokio::test]
    async fn test_on_chain_verify_rejects_insufficient_amount() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(VerifyingBackend {
            details: TransferDetails {
                from: "0xsender".into(),
                to: "0xrecipient".into(),
                amount: 10, // less than required
                block_number: 1,
            },
        });

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: true,
            receiving_account: String::new(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());
        let mut task = Task::new(sender.pubkey(), &rpk, "underpaid");
        task.payment_tx = Some("0xunderpaid".to_string());
        task.payment_amount = Some(10);
        sender.send_task(&task).await.unwrap();
        assert!(receiver.poll_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_on_chain_verify_rejects_wrong_recipient() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(VerifyingBackend {
            details: TransferDetails {
                from: "0xsender".into(),
                to: "0xwrong".into(),
                amount: 100,
                block_number: 1,
            },
        });

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: true,
            receiving_account: "0xcorrect".to_string(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());
        let mut task = Task::new(sender.pubkey(), &rpk, "wrong dest");
        task.payment_tx = Some("0xwrongdest".to_string());
        task.payment_amount = Some(100);
        sender.send_task(&task).await.unwrap();
        assert!(receiver.poll_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_on_chain_verify_rejects_failed_tx() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(FailingVerifyBackend);

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: true,
            receiving_account: String::new(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());
        let mut task = Task::new(sender.pubkey(), &rpk, "bad tx");
        task.payment_tx = Some("0xnonexistent".to_string());
        task.payment_amount = Some(100);
        sender.send_task(&task).await.unwrap();
        assert!(receiver.poll_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_on_chain_verify_accepts_valid_payment() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(VerifyingBackend {
            details: TransferDetails {
                from: "0xsender".into(),
                to: "0xmy_wallet".into(),
                amount: 200,
                block_number: 42,
            },
        });

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 100,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: true,
            receiving_account: "0xmy_wallet".to_string(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());
        let mut task = Task::new(sender.pubkey(), &rpk, "valid payment");
        task.payment_tx = Some("0xgoodtx".to_string());
        task.payment_amount = Some(200);
        sender.send_task(&task).await.unwrap();
        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].text(), Some("valid payment"));
    }

    #[tokio::test]
    async fn test_payment_with_empty_tx_hash_rejected() {
        let transport = MockTransport::new();
        let backend = Arc::new(MockExecutionBackend);

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend: backend.clone(),
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: false,
            receiving_account: String::new(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());

        // Task with empty tx hash
        let mut task = Task::new(sender.pubkey(), &rpk, "empty tx");
        task.payment_tx = Some(String::new());
        task.payment_amount = Some(100);
        sender.send_task(&task).await.unwrap();

        let received = receiver.poll_tasks().await.unwrap();
        assert!(received.is_empty(), "empty tx hash should be rejected");
    }

    #[tokio::test]
    async fn test_payment_zero_required_accepts_all() {
        let transport = MockTransport::new();
        let backend = Arc::new(MockExecutionBackend);

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 0,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: false,
            receiving_account: String::new(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());

        // No payment at all
        let task = Task::new(sender.pubkey(), &rpk, "free task");
        sender.send_task(&task).await.unwrap();

        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(
            received.len(),
            1,
            "zero required_amount should accept all tasks"
        );
    }

    #[tokio::test]
    async fn test_node_without_payment_config_accepts_all() {
        let transport = MockTransport::new();
        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        );
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());

        let task = Task::new(sender.pubkey(), &rpk, "no payment config");
        sender.send_task(&task).await.unwrap();

        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(
            received.len(),
            1,
            "node without payment config should accept all tasks"
        );
    }

    #[tokio::test]
    async fn test_maybe_auto_pay_disabled() {
        let backend = Arc::new(MockExecutionBackend);
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config())
            .with_payment(PaymentConfig {
                backend,
                required_amount: 0,
                auto_pay: false, // disabled
                auto_pay_amount: 100,
                verify_on_chain: false,
                receiving_account: String::new(),
            });

        let task = Task::new(node.pubkey(), "02aa", "no auto pay");
        let result = node.maybe_auto_pay(&task).await.unwrap();
        // With auto_pay disabled, task should NOT have payment info
        assert!(result.payment_tx.is_none());
        assert!(result.payment_amount.is_none());
    }

    #[tokio::test]
    async fn test_maybe_auto_pay_zero_amount() {
        let backend = Arc::new(MockExecutionBackend);
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config())
            .with_payment(PaymentConfig {
                backend,
                required_amount: 0,
                auto_pay: true,
                auto_pay_amount: 0, // zero amount
                verify_on_chain: false,
                receiving_account: String::new(),
            });

        let task = Task::new(node.pubkey(), "02aa", "zero amount");
        let result = node.maybe_auto_pay(&task).await.unwrap();
        // With zero auto_pay_amount, should not actually pay
        assert!(result.payment_tx.is_none());
    }

    #[tokio::test]
    async fn test_maybe_auto_pay_attaches_tx_hash() {
        let backend = Arc::new(MockExecutionBackend);
        let transport = MockTransport::new();
        let node = WakuA2ANode::with_config("test", "test", vec![], transport, fast_config())
            .with_payment(PaymentConfig {
                backend,
                required_amount: 0,
                auto_pay: true,
                auto_pay_amount: 50,
                verify_on_chain: false,
                receiving_account: String::new(),
            });

        let task = Task::new(node.pubkey(), "02aa", "pay me");
        let result = node.maybe_auto_pay(&task).await.unwrap();
        assert!(result.payment_tx.is_some());
        assert_eq!(result.payment_amount, Some(50));
    }

    #[tokio::test]
    async fn test_maybe_auto_pay_no_payment_config() {
        let transport = MockTransport::new();
        let node = WakuA2ANode::new("test", "test", vec![], transport);

        let task = Task::new(node.pubkey(), "02aa", "no config");
        let result = node.maybe_auto_pay(&task).await.unwrap();
        assert!(result.payment_tx.is_none());
        assert!(result.payment_amount.is_none());
    }

    #[tokio::test]
    async fn test_on_chain_verify_case_insensitive_recipient() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(VerifyingBackend {
            details: TransferDetails {
                from: "0xsender".into(),
                to: "0xMyWallet".into(), // mixed case
                amount: 200,
                block_number: 1,
            },
        });

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 100,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: true,
            receiving_account: "0xmywallet".to_string(), // lowercase
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());
        let mut task = Task::new(sender.pubkey(), &rpk, "case test");
        task.payment_tx = Some("0xcasetx".to_string());
        task.payment_amount = Some(200);
        sender.send_task(&task).await.unwrap();

        let received = receiver.poll_tasks().await.unwrap();
        assert_eq!(
            received.len(),
            1,
            "case-insensitive recipient match should accept"
        );
    }

    #[tokio::test]
    async fn test_replay_protection_with_on_chain_verify() {
        let transport = MockTransport::new();
        let backend: Arc<dyn ExecutionBackend> = Arc::new(VerifyingBackend {
            details: TransferDetails {
                from: "0xsender".into(),
                to: "0xrecipient".into(),
                amount: 200,
                block_number: 1,
            },
        });

        let receiver = WakuA2ANode::with_config(
            "receiver",
            "receiver",
            vec![],
            transport.clone(),
            fast_config(),
        )
        .with_payment(PaymentConfig {
            backend,
            required_amount: 100,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: true,
            receiving_account: "0xrecipient".to_string(),
        });
        let rpk = receiver.pubkey().to_string();
        let _ = receiver.poll_tasks().await.unwrap();

        let sender =
            WakuA2ANode::with_config("sender", "sender", vec![], transport.clone(), fast_config());

        // First use — accepted
        let mut t1 = Task::new(sender.pubkey(), &rpk, "first");
        t1.payment_tx = Some("0xreplay_onchain".to_string());
        t1.payment_amount = Some(200);
        sender.send_task(&t1).await.unwrap();
        assert_eq!(receiver.poll_tasks().await.unwrap().len(), 1);

        // Replay — rejected
        let mut t2 = Task::new(sender.pubkey(), &rpk, "replay");
        t2.payment_tx = Some("0xreplay_onchain".to_string());
        t2.payment_amount = Some(200);
        sender.send_task(&t2).await.unwrap();
        assert!(receiver.poll_tasks().await.unwrap().is_empty());
    }

    #[test]
    fn test_with_payment_builder() {
        let transport = MockTransport::new();
        let backend = Arc::new(MockExecutionBackend);
        let node =
            WakuA2ANode::new("test", "test", vec![], transport).with_payment(PaymentConfig {
                backend,
                required_amount: 42,
                auto_pay: true,
                auto_pay_amount: 10,
                verify_on_chain: true,
                receiving_account: "0xabc".to_string(),
            });
        assert!(node.payment.is_some());
        let pay = node.payment.as_ref().unwrap();
        assert_eq!(pay.required_amount, 42);
        assert!(pay.auto_pay);
        assert_eq!(pay.auto_pay_amount, 10);
        assert!(pay.verify_on_chain);
        assert_eq!(pay.receiving_account, "0xabc");
    }
}
