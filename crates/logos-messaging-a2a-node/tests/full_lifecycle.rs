//! Full lifecycle integration test:
//! discovery (presence + registry) → encrypted task → payment → response.
//!
//! Exercises the complete agent-to-agent flow using InMemoryTransport,
//! InMemoryRegistry, and a mock ExecutionBackend in a single test.

use async_trait::async_trait;
use logos_messaging_a2a_core::registry::{AgentRegistry, InMemoryRegistry};
use logos_messaging_a2a_core::{AgentCard, Task, TaskState};
use logos_messaging_a2a_execution::{AgentId, ExecutionBackend, TransferDetails, TxHash};
use logos_messaging_a2a_node::{PaymentConfig, WakuA2ANode};
use logos_messaging_a2a_transport::memory::InMemoryTransport;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Mock execution backend that tracks payments in-memory.
struct MockPaymentBackend {
    transfers: Mutex<Vec<(String, u64)>>,
}

impl MockPaymentBackend {
    fn new() -> Self {
        Self {
            transfers: Mutex::new(Vec::new()),
        }
    }

    fn transfer_count(&self) -> usize {
        self.transfers.lock().unwrap().len()
    }
}

#[async_trait]
impl ExecutionBackend for MockPaymentBackend {
    async fn register_agent(
        &self,
        _card: &AgentCard,
    ) -> Result<TxHash, logos_messaging_a2a_execution::ExecutionError> {
        Ok(TxHash([0xaa; 32]))
    }

    async fn pay(
        &self,
        to: &AgentId,
        amount: u64,
    ) -> Result<TxHash, logos_messaging_a2a_execution::ExecutionError> {
        self.transfers.lock().unwrap().push((to.0.clone(), amount));
        Ok(TxHash([0xbb; 32]))
    }

    async fn balance(
        &self,
        _agent: &AgentId,
    ) -> Result<u64, logos_messaging_a2a_execution::ExecutionError> {
        Ok(1000)
    }

    async fn verify_transfer(
        &self,
        _tx_hash: &str,
    ) -> Result<TransferDetails, logos_messaging_a2a_execution::ExecutionError> {
        Ok(TransferDetails {
            from: "0xsender".into(),
            to: "0xreceiver".into(),
            amount: 50,
            block_number: 1,
        })
    }
}

/// Full end-to-end: discovery → encrypted task with payment → response.
#[tokio::test]
async fn full_lifecycle_discovery_encrypted_task_payment() {
    let transport = InMemoryTransport::new();
    let registry = Arc::new(InMemoryRegistry::new());
    let backend = Arc::new(MockPaymentBackend::new());

    // Service agent: requires 50 tokens per task
    let service = Arc::new(
        WakuA2ANode::new_encrypted(
            "translator",
            "Translator Service",
            vec!["translate".into(), "text".into()],
            transport.clone(),
        )
        .with_registry(registry.clone())
        .with_payment(PaymentConfig {
            backend: backend.clone(),
            required_amount: 50,
            auto_pay: false,
            auto_pay_amount: 0,
            verify_on_chain: false,
            receiving_account: "0xservice_wallet".into(),
        }),
    );

    // Client agent: auto-pays 50 tokens per task
    let client = Arc::new(
        WakuA2ANode::new_encrypted(
            "client",
            "Client Agent",
            vec!["text".into()],
            transport.clone(),
        )
        .with_registry(registry.clone())
        .with_payment(PaymentConfig {
            backend: backend.clone(),
            required_amount: 0,
            auto_pay: true,
            auto_pay_amount: 50,
            verify_on_chain: false,
            receiving_account: String::new(),
        }),
    );

    // === Phase 1: Discovery ===
    service.register_in_registry().await.unwrap();
    client.register_in_registry().await.unwrap();
    service.announce_presence().await.unwrap();
    client.announce_presence().await.unwrap();

    client.poll_presence().await.unwrap();
    let all = client.discover_all().await.unwrap();

    let service_cards: Vec<_> = all.iter().filter(|c| c.name == "translator").collect();
    assert_eq!(service_cards.len(), 1, "should discover translator");
    assert!(service_cards[0].intro_bundle.is_some());
    assert!(service_cards[0]
        .capabilities
        .contains(&"translate".to_string()));

    // === Phase 2: Send encrypted task with auto-pay ===
    service.poll_tasks().await.unwrap();
    client.poll_tasks().await.unwrap();

    let task = Task::new(
        client.pubkey(),
        service.pubkey(),
        "Translate: hello → Czech",
    );
    let service_card_owned = service_cards[0].clone();
    let client_clone = client.clone();
    let task_clone = task.clone();

    let send_handle = tokio::spawn(async move {
        client_clone
            .send_task_to(&task_clone, Some(&service_card_owned))
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let tasks = service.poll_tasks().await.unwrap();
    send_handle.await.unwrap();

    assert_eq!(tasks.len(), 1);
    let received = &tasks[0];
    assert_eq!(received.text(), Some("Translate: hello → Czech"));
    assert_eq!(received.from, client.pubkey());
    assert!(received.payment_tx.is_some(), "auto-pay should attach tx");
    assert_eq!(received.payment_amount, Some(50));
    assert_eq!(backend.transfer_count(), 1);

    // === Phase 3: Service responds encrypted ===
    let client_card = client.card.clone();
    service
        .respond_to(received, "Ahoj", Some(&client_card))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(10)).await;
    let responses = client.poll_tasks().await.unwrap();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].result_text(), Some("Ahoj"));
    assert_eq!(responses[0].state, TaskState::Completed);
    assert_eq!(responses[0].id, task.id);
}

/// discover_all deduplicates agents found in both presence and registry.
#[tokio::test]
async fn discover_all_deduplicates_presence_and_registry() {
    let transport = InMemoryTransport::new();
    let registry = Arc::new(InMemoryRegistry::new());

    let agent_a = WakuA2ANode::new("aa", "Agent A", vec!["text".into()], transport.clone())
        .with_registry(registry.clone());
    let agent_b = WakuA2ANode::new("bb", "Agent B", vec!["text".into()], transport.clone())
        .with_registry(registry.clone());
    let observer =
        WakuA2ANode::new("obs", "Observer", vec![], transport).with_registry(registry.clone());

    // agent_a in both registry AND presence
    agent_a.register_in_registry().await.unwrap();
    agent_a.announce_presence().await.unwrap();

    // agent_b in registry only
    agent_b.register_in_registry().await.unwrap();

    observer.poll_presence().await.unwrap();
    let all = observer.discover_all().await.unwrap();

    assert_eq!(all.len(), 2, "should find 2 unique agents");
    let names: Vec<_> = all.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"aa"));
    assert!(names.contains(&"bb"));
}

/// Five agents register and announce; observer discovers all of them.
#[tokio::test]
async fn five_agent_concurrent_discovery() {
    let transport = InMemoryTransport::new();
    let registry = Arc::new(InMemoryRegistry::new());

    let mut nodes = Vec::new();
    for i in 0..5 {
        let node = WakuA2ANode::new(
            &format!("agent-{}", i),
            &format!("Agent {}", i),
            vec![format!("cap-{}", i)],
            transport.clone(),
        )
        .with_registry(registry.clone());
        nodes.push(node);
    }

    for node in &nodes {
        node.register_in_registry().await.unwrap();
        node.announce_presence().await.unwrap();
    }

    let observer =
        WakuA2ANode::new("observer", "Observer", vec![], transport).with_registry(registry.clone());
    observer.poll_presence().await.unwrap();
    let all = observer.discover_all().await.unwrap();

    assert_eq!(all.len(), 5, "should discover all 5 agents");

    let cap2 = registry.find_by_capability("cap-2").await.unwrap();
    assert_eq!(cap2.len(), 1);
    assert_eq!(cap2[0].name, "agent-2");
}
