//! End-to-end demo: two agents negotiate and execute a task with payment.
//!
//! Agent A (requester) sends a "summarize this text" task to Agent B (worker).
//! Agent A auto-pays before sending. Agent B verifies payment before processing.
//! All communication happens over InMemoryTransport — no external deps required.
//!
//! Usage:
//!   cargo run --example two_agents

use anyhow::Result;
use async_trait::async_trait;
use logos_messaging_a2a::{
    A2AEnvelope, AgentId, ExecutionBackend, ExecutionError, InMemoryTransport, PaymentConfig, Task,
    TransferDetails, Transport, TxHash, WakuA2ANode,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Mock execution backend for local demos.
/// Tracks balances in-memory and always succeeds.
struct MockPaymentBackend {
    balance_a: AtomicU64,
    balance_b: AtomicU64,
}

impl MockPaymentBackend {
    fn new(initial_a: u64, initial_b: u64) -> Arc<Self> {
        Arc::new(Self {
            balance_a: AtomicU64::new(initial_a),
            balance_b: AtomicU64::new(initial_b),
        })
    }
}

#[async_trait]
impl ExecutionBackend for MockPaymentBackend {
    async fn register_agent(
        &self,
        _card: &logos_messaging_a2a::AgentCard,
    ) -> Result<TxHash, ExecutionError> {
        Ok(TxHash([0; 32]))
    }

    async fn pay(&self, _to: &AgentId, amount: u64) -> Result<TxHash, ExecutionError> {
        let prev = self.balance_a.fetch_sub(amount, Ordering::Relaxed);
        self.balance_b.fetch_add(amount, Ordering::Relaxed);
        println!(
            "  💰 Payment: {} tokens transferred (A: {} → {})",
            amount,
            prev,
            prev - amount
        );
        Ok(TxHash([0xaa; 32]))
    }

    async fn balance(&self, _agent: &AgentId) -> Result<u64, ExecutionError> {
        Ok(self.balance_a.load(Ordering::Relaxed))
    }

    async fn verify_transfer(&self, _tx_hash: &str) -> Result<TransferDetails, ExecutionError> {
        Ok(TransferDetails {
            from: "0xagent_a".into(),
            to: "0xagent_b".into(),
            amount: 100,
            block_number: 1,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== LMAO End-to-End Demo ===");
    println!("Two agents: one task, one payment, zero servers.\n");

    let transport = InMemoryTransport::new();
    let payment_backend = MockPaymentBackend::new(1000, 0);

    // --- Agent A: Requester (auto-pays 100 tokens per task) ---
    let agent_a = WakuA2ANode::new(
        "agent-a-requester",
        "Requests text summarization",
        vec!["text".to_string()],
        transport.clone(),
    )
    .with_payment(PaymentConfig {
        backend: payment_backend.clone(),
        required_amount: 0,
        auto_pay: true,
        auto_pay_amount: 100,
        verify_on_chain: false,
        receiving_account: String::new(),
    });

    // --- Agent B: Worker (requires 50 tokens minimum) ---
    let agent_b = WakuA2ANode::new(
        "agent-b-worker",
        "Summarizes text for payment",
        vec!["text".to_string(), "summarization".to_string()],
        transport.clone(),
    )
    .with_payment(PaymentConfig {
        backend: payment_backend.clone(),
        required_amount: 50,
        auto_pay: false,
        auto_pay_amount: 0,
        verify_on_chain: false,
        receiving_account: String::new(),
    });

    println!(
        "Agent A (requester): {} ({}...)",
        agent_a.card.name,
        &agent_a.pubkey()[..16]
    );
    println!(
        "Agent B (worker):    {} ({}...)\n",
        agent_b.card.name,
        &agent_b.pubkey()[..16]
    );

    // Step 1: Both agents announce themselves
    println!("📢 Step 1: Agents announce on the discovery topic");
    agent_a.announce().await?;
    agent_b.announce().await?;

    // Step 2: Agent A discovers Agent B
    println!("🔍 Step 2: Agent A discovers peers");
    let discovered = agent_a.discover().await?;
    let worker = discovered
        .iter()
        .find(|c| c.capabilities.contains(&"summarization".to_string()))
        .expect("No summarization agent found!");
    println!(
        "   Found: {} with capabilities {:?}\n",
        worker.name, worker.capabilities
    );

    // Step 3: Agent A sends a task (auto-pay triggers)
    println!("📤 Step 3: Agent A sends task with auto-payment");
    let input_text = "The Logos Network is a decentralized infrastructure project \
        building censorship-resistant communication, storage, and computation. \
        It includes Waku for messaging, Codex for storage, and the Status app \
        as a user-facing product. The goal is to provide public goods infrastructure \
        that cannot be controlled by any single entity.";

    let task = Task::new(agent_a.pubkey(), &worker.public_key, input_text);
    println!("   Task {}: \"{}...\"", &task.id[..8], &input_text[..60]);

    // Auto-pay then publish directly (bypasses SDS retransmit for demo simplicity)
    let task = agent_a.maybe_auto_pay(&task).await?;
    let envelope = A2AEnvelope::Task(task.clone());
    let payload = serde_json::to_vec(&envelope)?;
    let topic = logos_messaging_a2a::topics::task_topic(&worker.public_key);
    // Ensure B is subscribed first
    agent_b.poll_tasks().await?;
    transport.publish(&topic, &payload).await?;

    // Step 4: Agent B receives and processes
    println!("\n📥 Step 4: Agent B receives task");
    let tasks = agent_b.poll_tasks().await?;

    for t in &tasks {
        let text = t.text().unwrap_or("(no text)");
        println!("   ✅ Task {} received", &t.id[..8]);
        println!("   Input: \"{}...\"", &text[..60.min(text.len())]);

        // "Summarize" the text (mock processing)
        let summary = "Logos Network: decentralized infra for censorship-resistant \
            messaging (Waku), storage (Codex), and computation. Status app is the UX layer.";
        println!("\n🔧 Step 5: Agent B processes and responds");
        println!("   Summary: \"{}\"", summary);
        agent_b.respond(t, summary).await?;
    }

    // Step 6: Agent A receives the result
    println!("\n📬 Step 6: Agent A receives the result");
    let results = agent_a.poll_tasks().await?;
    for r in &results {
        if let Some(text) = r.result_text() {
            println!("   ✅ Result: \"{}\"", text);
        }
    }

    println!("\n=== Demo Complete ===");
    println!("✨ Two agents discovered each other, negotiated a task,");
    println!("   exchanged payment, and delivered results — all peer-to-peer");
    println!("   over Waku topics with zero HTTP servers.\n");

    Ok(())
}
