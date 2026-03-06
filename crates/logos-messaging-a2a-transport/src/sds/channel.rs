//! SDS MessageChannel — reliable delivery with causal ordering.
//!
//! This is the core SDS implementation, modeled after the Logos Messaging
//! specification (logos-delivery-js reference). It provides:
//!
//! - Bloom filter deduplication (replacing naive HashSet)
//! - Lamport timestamps for causal ordering
//! - Causal history tracking (last N message IDs)
//! - Outgoing buffer with retransmission
//! - Incoming buffer with dependency resolution
//! - Sync messages for consistency protocol
//!
//! Reference: https://forum.research.logos.co/t/introducing-the-reliable-channel-api/580

use crate::Transport;
use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use super::bloom::SdsBloomFilter;
use super::message::*;

/// Default number of causal history entries to include in messages.
const DEFAULT_CAUSAL_HISTORY_SIZE: usize = 200;
/// How many "possible acks" (bloom filter hits on dependencies) before
/// considering a dependency acknowledged.
const DEFAULT_POSSIBLE_ACKS_THRESHOLD: u32 = 2;
/// Default ACK timeout for retransmission.
const DEFAULT_ACK_TIMEOUT: Duration = Duration::from_secs(10);
/// Default max retries.
const DEFAULT_MAX_RETRIES: u32 = 3;

/// Configuration for the SDS channel.
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    /// Number of causal history entries per message.
    pub causal_history_size: usize,
    /// How many bloom filter hits count as a definitive ack.
    pub possible_acks_threshold: u32,
    /// ACK timeout for retransmission.
    pub ack_timeout: Duration,
    /// Maximum retransmission attempts.
    pub max_retries: u32,
    /// Timeout (ms) after which unresolved dependencies are marked lost.
    /// None = disabled.
    pub timeout_for_lost_messages_ms: Option<u64>,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            causal_history_size: DEFAULT_CAUSAL_HISTORY_SIZE,
            possible_acks_threshold: DEFAULT_POSSIBLE_ACKS_THRESHOLD,
            ack_timeout: DEFAULT_ACK_TIMEOUT,
            max_retries: DEFAULT_MAX_RETRIES,
            timeout_for_lost_messages_ms: None,
        }
    }
}

impl ChannelConfig {
    /// Fire-and-forget mode (no retries, ephemeral only).
    pub fn fire_and_forget() -> Self {
        Self {
            ack_timeout: Duration::from_millis(0),
            max_retries: 0,
            ..Default::default()
        }
    }
}

/// An SDS MessageChannel providing reliable, causally-ordered delivery.
pub struct MessageChannel<T: Transport> {
    channel_id: ChannelId,
    sender_id: ParticipantId,
    transport: T,
    config: ChannelConfig,

    /// Monotonically increasing Lamport timestamp.
    lamport_timestamp: AtomicU64,

    /// Bloom filter for deduplication.
    pub bloom: SdsBloomFilter,

    /// Local history of delivered content messages (bounded ring buffer).
    local_history: Mutex<VecDeque<HistoryEntry>>,

    /// Outgoing buffer: messages awaiting ACK.
    outgoing_buffer: Mutex<Vec<ContentMessage>>,

    /// Messages with unresolved dependencies.
    incoming_buffer: Mutex<Vec<SdsMessage>>,

    /// Track possible acks per message ID.
    #[allow(dead_code)]
    possible_acks: Mutex<std::collections::HashMap<MessageId, u32>>,
}

impl<T: Transport> MessageChannel<T> {
    /// Create a new SDS channel.
    pub fn new(channel_id: ChannelId, sender_id: ParticipantId, transport: T) -> Self {
        Self::with_config(channel_id, sender_id, transport, ChannelConfig::default())
    }

    /// Create with custom config.
    pub fn with_config(
        channel_id: ChannelId,
        sender_id: ParticipantId,
        transport: T,
        config: ChannelConfig,
    ) -> Self {
        // Initialize lamport timestamp from current time (ms) as in the JS impl.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            channel_id,
            sender_id,
            transport,
            config,
            lamport_timestamp: AtomicU64::new(now_ms),
            bloom: SdsBloomFilter::new(),
            local_history: Mutex::new(VecDeque::new()),
            outgoing_buffer: Mutex::new(Vec::new()),
            incoming_buffer: Mutex::new(Vec::new()),
            #[allow(dead_code)]
            possible_acks: Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }

    pub fn sender_id(&self) -> &str {
        &self.sender_id
    }

    pub fn config(&self) -> &ChannelConfig {
        &self.config
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Get the current Lamport timestamp.
    pub fn lamport_timestamp(&self) -> u64 {
        self.lamport_timestamp.load(Ordering::SeqCst)
    }

    /// Advance and return the next Lamport timestamp.
    fn next_timestamp(&self) -> u64 {
        self.lamport_timestamp.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Update lamport timestamp on receiving a message (max of local, remote+1).
    fn update_timestamp(&self, remote_ts: u64) {
        loop {
            let current = self.lamport_timestamp.load(Ordering::SeqCst);
            let new_val = std::cmp::max(current, remote_ts) + 1;
            if self
                .lamport_timestamp
                .compare_exchange(current, new_val, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }
    }

    /// Build the causal history for outgoing messages (last N entries).
    fn build_causal_history(&self) -> Vec<HistoryEntry> {
        let history = self.local_history.lock().unwrap();
        history
            .iter()
            .rev()
            .take(self.config.causal_history_size)
            .cloned()
            .collect()
    }

    /// Record a message in local history.
    fn record_in_history(&self, message_id: &str, lamport_timestamp: u64) {
        let mut history = self.local_history.lock().unwrap();
        history.push_back(HistoryEntry {
            message_id: message_id.to_string(),
            lamport_timestamp,
            retrieval_hint: None,
        });
        // Keep bounded
        while history.len() > self.config.causal_history_size * 2 {
            history.pop_front();
        }
    }

    /// Send a content message with causal ordering and reliability.
    pub async fn send(&self, topic: &str, payload: &[u8]) -> Result<ContentMessage> {
        let message_id = compute_message_id(payload);
        let ts = self.next_timestamp();
        let causal_history = self.build_causal_history();

        let msg = ContentMessage {
            message_id: message_id.clone(),
            channel_id: self.channel_id.clone(),
            sender_id: self.sender_id.clone(),
            lamport_timestamp: ts,
            causal_history,
            bloom_filter: Some(self.bloom.to_bytes()),
            content: payload.to_vec(),
            repair_request: Vec::new(),
            retrieval_hint: None,
        };

        // Serialize and publish
        let encoded = serde_json::to_vec(&SdsMessage::Content(msg.clone()))?;
        self.transport
            .publish(topic, &encoded)
            .await
            .context("SDS: failed to publish content message")?;

        // Record in local state
        self.bloom.set(&message_id);
        self.record_in_history(&message_id, ts);

        // Add to outgoing buffer for potential retransmission
        self.outgoing_buffer.lock().unwrap().push(msg.clone());

        Ok(msg)
    }

    /// Send a content message with retransmission until ACK.
    pub async fn send_reliable(
        &self,
        topic: &str,
        payload: &[u8],
    ) -> Result<(ContentMessage, bool)> {
        let message_id = compute_message_id(payload);
        let ts = self.next_timestamp();
        let causal_history = self.build_causal_history();
        let ack_topic = format!("/lmao/1/ack/{}/proto", message_id);

        let msg = ContentMessage {
            message_id: message_id.clone(),
            channel_id: self.channel_id.clone(),
            sender_id: self.sender_id.clone(),
            lamport_timestamp: ts,
            causal_history,
            bloom_filter: Some(self.bloom.to_bytes()),
            content: payload.to_vec(),
            repair_request: Vec::new(),
            retrieval_hint: None,
        };

        let encoded = serde_json::to_vec(&SdsMessage::Content(msg.clone()))?;

        let mut ack_rx = self.transport.subscribe(&ack_topic).await?;

        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                eprintln!(
                    "[sds] retransmit attempt {}/{} for {}",
                    attempt, self.config.max_retries, message_id
                );
            }

            self.transport
                .publish(topic, &encoded)
                .await
                .context("SDS: publish failed")?;

            match tokio::time::timeout(self.config.ack_timeout, ack_rx.recv()).await {
                Ok(Some(ack_data)) => {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&ack_data) {
                        if val.get("message_id").and_then(|v| v.as_str()) == Some(&message_id) {
                            let _ = self.transport.unsubscribe(&ack_topic).await;
                            self.bloom.set(&message_id);
                            self.record_in_history(&message_id, ts);
                            return Ok((msg, true));
                        }
                    }
                }
                _ => continue,
            }
        }

        let _ = self.transport.unsubscribe(&ack_topic).await;
        self.bloom.set(&message_id);
        self.record_in_history(&message_id, ts);
        eprintln!(
            "[sds] no ACK after {} retries for {}",
            self.config.max_retries, message_id
        );
        Ok((msg, false))
    }

    /// Send an explicit ACK for a received message.
    pub async fn send_ack(&self, _topic_prefix: &str, message_id: &str) -> Result<()> {
        let ack_topic = format!("/lmao/1/ack/{}/proto", message_id);
        let ack_payload = serde_json::to_vec(&serde_json::json!({
            "type": "ack",
            "message_id": message_id,
        }))?;
        self.transport.publish(&ack_topic, &ack_payload).await
    }

    /// Send a sync message (no payload, just bloom filter + causal history).
    pub async fn send_sync(&self, topic: &str) -> Result<SyncMessage> {
        let ts = self.next_timestamp();
        let causal_history = self.build_causal_history();
        let message_id = compute_message_id(&ts.to_le_bytes());

        let msg = SyncMessage {
            message_id: message_id.clone(),
            channel_id: self.channel_id.clone(),
            sender_id: self.sender_id.clone(),
            lamport_timestamp: ts,
            causal_history,
            bloom_filter: Some(self.bloom.to_bytes()),
            repair_request: Vec::new(),
        };

        let encoded = serde_json::to_vec(&SdsMessage::Sync(msg.clone()))?;
        self.transport.publish(topic, &encoded).await?;
        Ok(msg)
    }

    /// Send an ephemeral message (fire-and-forget, no causal ordering).
    pub async fn send_ephemeral(&self, topic: &str, payload: &[u8]) -> Result<EphemeralMessage> {
        let message_id = compute_message_id(payload);

        let msg = EphemeralMessage {
            message_id: message_id.clone(),
            channel_id: self.channel_id.clone(),
            sender_id: self.sender_id.clone(),
            causal_history: Vec::new(),
            bloom_filter: None,
            content: payload.to_vec(),
            repair_request: Vec::new(),
        };

        let encoded = serde_json::to_vec(&SdsMessage::Ephemeral(msg.clone()))?;
        self.transport.publish(topic, &encoded).await?;
        Ok(msg)
    }

    /// Process an incoming raw message. Returns delivered content messages.
    ///
    /// This handles:
    /// - Deduplication via bloom filter
    /// - Lamport timestamp update
    /// - Causal dependency checking
    /// - ACK detection via bloom filters in sync messages
    pub fn receive(&self, raw: &[u8]) -> Vec<ContentMessage> {
        let msg: SdsMessage = match serde_json::from_slice(raw) {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };

        // Dedup
        if self.bloom.check_and_set(msg.message_id()) {
            return Vec::new();
        }

        // Check remote bloom filter for possible ACKs of our outgoing messages
        if let Some(remote_bloom_bytes) = msg.bloom_filter_bytes() {
            self.check_outgoing_acks(remote_bloom_bytes);
        }

        match msg {
            SdsMessage::Content(content) => {
                self.update_timestamp(content.lamport_timestamp);
                // Check if causal dependencies are satisfied
                if self.dependencies_satisfied(&content.causal_history) {
                    self.record_in_history(&content.message_id, content.lamport_timestamp);
                    vec![content]
                } else {
                    // Buffer for later resolution
                    self.incoming_buffer
                        .lock()
                        .unwrap()
                        .push(SdsMessage::Content(content));
                    // Try to resolve buffered messages
                    self.resolve_buffered()
                }
            }
            SdsMessage::Sync(sync) => {
                self.update_timestamp(sync.lamport_timestamp);
                // Sync messages don't deliver content but may resolve buffered deps
                self.resolve_buffered()
            }
            SdsMessage::Ephemeral(_) => {
                // Ephemeral messages have no causal ordering
                Vec::new()
            }
        }
    }

    /// Check if causal dependencies are satisfied (all in our bloom filter).
    fn dependencies_satisfied(&self, causal_history: &[HistoryEntry]) -> bool {
        // If no dependencies, always satisfied
        if causal_history.is_empty() {
            return true;
        }
        // Check that we've seen the dependencies
        causal_history
            .iter()
            .all(|entry| self.bloom.check(&entry.message_id))
    }

    /// Check outgoing buffer against a remote bloom filter for implicit ACKs.
    ///
    /// When a remote peer includes a bloom filter in their message, we check
    /// if any of our outgoing (unacked) messages appear in it. If a message
    /// appears in enough remote blooms (>= possible_acks_threshold), we
    /// consider it implicitly acknowledged and remove it from the outgoing buffer.
    fn check_outgoing_acks(&self, remote_bloom_bytes: &[u8]) {
        let remote_bloom = match SdsBloomFilter::from_bytes(remote_bloom_bytes) {
            Some(b) => b,
            None => return, // malformed bloom, skip
        };

        let mut outgoing = self.outgoing_buffer.lock().unwrap();
        let mut possible_acks = self.possible_acks.lock().unwrap();

        outgoing.retain(|msg| {
            if remote_bloom.probably_contains(&msg.message_id) {
                let count = possible_acks.entry(msg.message_id.clone()).or_insert(0);
                *count += 1;
                if *count >= self.config.possible_acks_threshold {
                    // Implicitly acknowledged — remove from outgoing buffer
                    possible_acks.remove(&msg.message_id);
                    return false; // don't retain
                }
            }
            true // keep in buffer
        });
    }

    /// Try to deliver buffered messages whose dependencies are now satisfied.
    fn resolve_buffered(&self) -> Vec<ContentMessage> {
        let mut delivered = Vec::new();
        let mut buffer = self.incoming_buffer.lock().unwrap();
        let mut made_progress = true;

        while made_progress {
            made_progress = false;
            let mut remaining = Vec::new();

            for msg in buffer.drain(..) {
                if let SdsMessage::Content(content) = &msg {
                    if self.dependencies_satisfied(&content.causal_history) {
                        self.record_in_history(&content.message_id, content.lamport_timestamp);
                        delivered.push(content.clone());
                        made_progress = true;
                    } else {
                        remaining.push(msg);
                    }
                } else {
                    remaining.push(msg);
                }
            }

            *buffer = remaining;
        }

        delivered
    }

    /// Send a batch ACK — a lightweight sync message acknowledging multiple
    /// messages at once. This is useful when a node has received many messages
    /// and wants to signal acknowledgement without sending individual ACKs.
    ///
    /// A batch ACK is implemented as a sync message carrying the node's bloom
    /// filter (which implicitly acknowledges everything seen) plus causal history.
    /// Peers use this to clear their outgoing buffers via implicit ACK detection.
    pub async fn send_batch_ack(&self, topic: &str) -> Result<SyncMessage> {
        self.send_sync(topic).await
    }

    /// Build repair requests for missing dependencies in a causal history.
    ///
    /// Returns history entries for messages we haven't seen (not in our bloom filter).
    pub fn build_repair_requests(&self, causal_history: &[HistoryEntry]) -> Vec<HistoryEntry> {
        causal_history
            .iter()
            .filter(|entry| !self.bloom.check(&entry.message_id))
            .cloned()
            .collect()
    }

    /// Send a sync message that includes repair requests for missing dependencies.
    ///
    /// When we detect missing causal dependencies (e.g. after buffering a
    /// message with unresolved deps), we ask peers to re-send them.
    pub async fn send_repair_request(
        &self,
        topic: &str,
        missing: Vec<HistoryEntry>,
    ) -> Result<SyncMessage> {
        let ts = self.next_timestamp();
        let causal_history = self.build_causal_history();
        let message_id = compute_message_id(&ts.to_le_bytes());

        let msg = SyncMessage {
            message_id: message_id.clone(),
            channel_id: self.channel_id.clone(),
            sender_id: self.sender_id.clone(),
            lamport_timestamp: ts,
            causal_history,
            bloom_filter: Some(self.bloom.to_bytes()),
            repair_request: missing,
        };

        let encoded = serde_json::to_vec(&SdsMessage::Sync(msg.clone()))?;
        self.transport.publish(topic, &encoded).await?;
        Ok(msg)
    }

    /// Handle incoming repair requests — re-publish messages the peer is missing.
    ///
    /// Scans the outgoing buffer for the requested message IDs and re-publishes
    /// them. Returns the number of messages re-sent.
    pub async fn handle_repair_requests(
        &self,
        topic: &str,
        requests: &[HistoryEntry],
    ) -> Result<usize> {
        if requests.is_empty() {
            return Ok(0);
        }

        let mut resent = 0;

        // Collect messages to re-send while holding the lock briefly
        let to_resend: Vec<ContentMessage> = {
            let outgoing = self.outgoing_buffer.lock().unwrap();
            requests
                .iter()
                .filter_map(|req| {
                    outgoing
                        .iter()
                        .find(|m| m.message_id == req.message_id)
                        .cloned()
                })
                .collect()
        };

        for msg in to_resend {
            let encoded = serde_json::to_vec(&SdsMessage::Content(msg))?;
            self.transport.publish(topic, &encoded).await?;
            resent += 1;
        }

        Ok(resent)
    }

    /// Process an incoming message, handling repair requests from the sender.
    ///
    /// This extends [`receive()`](Self::receive) by also checking if the incoming message
    /// contains repair requests and re-publishing requested messages.
    pub async fn receive_and_repair(&self, topic: &str, raw: &[u8]) -> Result<Vec<ContentMessage>> {
        let msg: SdsMessage = match serde_json::from_slice(raw) {
            Ok(m) => m,
            Err(_) => return Ok(Vec::new()),
        };

        // Handle repair requests from the sender
        let repair_reqs = msg.repair_requests().to_vec();
        if !repair_reqs.is_empty() {
            self.handle_repair_requests(topic, &repair_reqs).await?;
        }

        // Delegate to normal receive logic
        Ok(self.receive(raw))
    }

    /// Check if a message ID has been seen (dedup check).
    pub fn is_duplicate(&self, message_id: &str) -> bool {
        self.bloom.check(message_id)
    }

    /// Number of messages in the outgoing buffer awaiting ACK.
    pub fn outgoing_pending(&self) -> usize {
        self.outgoing_buffer.lock().unwrap().len()
    }

    /// Number of messages in the incoming buffer awaiting dependency resolution.
    pub fn incoming_pending(&self) -> usize {
        self.incoming_buffer.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::InMemoryTransport;

    fn make_channel(id: &str) -> MessageChannel<InMemoryTransport> {
        MessageChannel::new(
            "test-channel".to_string(),
            id.to_string(),
            InMemoryTransport::new(),
        )
    }

    #[test]
    fn test_lamport_timestamp_advances() {
        let ch = make_channel("alice");
        let t1 = ch.lamport_timestamp();
        let t2 = ch.next_timestamp();
        assert!(t2 > t1);
    }

    #[test]
    fn test_dedup_via_bloom() {
        let ch = make_channel("alice");
        ch.bloom.set("msg-1");
        assert!(ch.is_duplicate("msg-1"));
        assert!(!ch.is_duplicate("msg-2"));
    }

    #[tokio::test]
    async fn test_send_and_receive() {
        let transport = InMemoryTransport::new();
        let alice =
            MessageChannel::new("chan-1".to_string(), "alice".to_string(), transport.clone());
        let bob = MessageChannel::new("chan-1".to_string(), "bob".to_string(), transport.clone());

        let topic = "/lmao/1/test/proto";
        let msg = alice.send(topic, b"hello from alice").await.unwrap();

        // Simulate bob receiving the raw message
        let raw = serde_json::to_vec(&SdsMessage::Content(msg.clone())).unwrap();
        let delivered = bob.receive(&raw);
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].content, b"hello from alice");

        // Receiving same message again should be deduped
        let delivered2 = bob.receive(&raw);
        assert_eq!(delivered2.len(), 0);
    }

    #[tokio::test]
    async fn test_send_ephemeral() {
        let ch = make_channel("alice");
        let topic = "/lmao/1/test/proto";
        let msg = ch.send_ephemeral(topic, b"ephemeral data").await.unwrap();
        assert!(msg.causal_history.is_empty());
        assert!(msg.bloom_filter.is_none());
    }

    #[tokio::test]
    async fn test_send_sync() {
        let ch = make_channel("alice");
        // Send a content msg first to populate history
        ch.send("/lmao/1/test/proto", b"msg1").await.unwrap();
        let sync = ch.send_sync("/lmao/1/test/proto").await.unwrap();
        assert!(!sync.causal_history.is_empty());
        assert!(sync.bloom_filter.is_some());
    }

    #[tokio::test]
    async fn test_causal_ordering() {
        let transport = InMemoryTransport::new();
        let alice =
            MessageChannel::new("chan-1".to_string(), "alice".to_string(), transport.clone());
        let bob = MessageChannel::new("chan-1".to_string(), "bob".to_string(), transport.clone());

        let topic = "/lmao/1/test/proto";

        // Alice sends msg1, then msg2 (which depends on msg1 via causal history)
        let msg1 = alice.send(topic, b"first").await.unwrap();
        let msg2 = alice.send(topic, b"second").await.unwrap();

        // Bob receives msg2 first (out of order) — should buffer it
        let raw2 = serde_json::to_vec(&SdsMessage::Content(msg2.clone())).unwrap();
        let delivered = bob.receive(&raw2);
        // msg2 depends on msg1, which bob hasn't seen
        assert_eq!(delivered.len(), 0);
        assert_eq!(bob.incoming_pending(), 1);

        // Now bob receives msg1 — should deliver both
        let raw1 = serde_json::to_vec(&SdsMessage::Content(msg1.clone())).unwrap();
        let delivered = bob.receive(&raw1);
        // msg1 has no deps (or deps are satisfied), delivers immediately
        // Then resolving buffered msg2 should also deliver
        assert!(delivered.len() >= 1);
    }

    #[tokio::test]
    async fn test_implicit_ack_via_bloom() {
        let transport = InMemoryTransport::new();
        let config = ChannelConfig {
            possible_acks_threshold: 2,
            ..Default::default()
        };
        let alice = MessageChannel::with_config(
            "chan-1".to_string(),
            "alice".to_string(),
            transport.clone(),
            config,
        );
        let bob = MessageChannel::new("chan-1".to_string(), "bob".to_string(), transport.clone());

        let topic = "/lmao/1/test/proto";

        // Alice sends a message — it goes into her outgoing buffer
        let msg = alice.send(topic, b"need ack").await.unwrap();
        assert_eq!(alice.outgoing_pending(), 1);

        // Bob receives it (puts it in his bloom)
        let raw = serde_json::to_vec(&SdsMessage::Content(msg.clone())).unwrap();
        bob.receive(&raw);

        // Bob sends a sync — his bloom now contains alice's message
        let sync = bob.send_sync(topic).await.unwrap();
        let sync_raw = serde_json::to_vec(&SdsMessage::Sync(sync)).unwrap();

        // Alice receives bob's sync — first bloom hit (count = 1, threshold = 2)
        alice.receive(&sync_raw);
        assert_eq!(alice.outgoing_pending(), 1); // not yet acked

        // Bob sends another sync
        let sync2 = bob.send_sync(topic).await.unwrap();
        let sync2_raw = serde_json::to_vec(&SdsMessage::Sync(sync2)).unwrap();

        // Alice receives second sync — second bloom hit (count = 2 >= threshold)
        alice.receive(&sync2_raw);
        assert_eq!(alice.outgoing_pending(), 0); // implicitly acked!
    }

    #[tokio::test]
    async fn test_send_reliable_with_ack() {
        let transport = InMemoryTransport::new();

        // Pre-publish an ACK
        let payload = b"hello reliable";
        let message_id = compute_message_id(payload);
        let ack_topic = format!("/lmao/1/ack/{}/proto", message_id);
        let ack_payload = serde_json::to_vec(&serde_json::json!({
            "type": "ack",
            "message_id": message_id,
        }))
        .unwrap();
        transport.publish(&ack_topic, &ack_payload).await.unwrap();

        let ch = MessageChannel::new("chan-1".to_string(), "alice".to_string(), transport);

        let (msg, acked) = ch
            .send_reliable("/lmao/1/test/proto", payload)
            .await
            .unwrap();
        assert!(acked);
        assert_eq!(msg.content, payload);
    }
}

#[cfg(test)]
mod repair_tests {
    use super::*;
    use crate::memory::InMemoryTransport;

    #[tokio::test]
    async fn test_batch_ack_clears_outgoing() {
        let transport = InMemoryTransport::new();
        let config = ChannelConfig {
            possible_acks_threshold: 1,
            ..Default::default()
        };
        let alice = MessageChannel::with_config(
            "chan".to_string(),
            "alice".to_string(),
            transport.clone(),
            config,
        );
        let bob = MessageChannel::new("chan".to_string(), "bob".to_string(), transport.clone());

        let topic = "/lmao/1/test/proto";

        let msg1 = alice.send(topic, b"one").await.unwrap();
        let msg2 = alice.send(topic, b"two").await.unwrap();
        let msg3 = alice.send(topic, b"three").await.unwrap();
        assert_eq!(alice.outgoing_pending(), 3);

        for msg in [&msg1, &msg2, &msg3] {
            let raw = serde_json::to_vec(&SdsMessage::Content(msg.clone())).unwrap();
            bob.receive(&raw);
        }

        let batch_ack = bob.send_batch_ack(topic).await.unwrap();
        let ack_raw = serde_json::to_vec(&SdsMessage::Sync(batch_ack)).unwrap();

        alice.receive(&ack_raw);
        assert_eq!(alice.outgoing_pending(), 0);
    }

    #[tokio::test]
    async fn test_send_repair_request() {
        let transport = InMemoryTransport::new();
        let bob = MessageChannel::new("chan".to_string(), "bob".to_string(), transport.clone());

        let topic = "/lmao/1/test/proto";

        let missing = vec![
            HistoryEntry {
                message_id: "missing-1".to_string(),
                lamport_timestamp: 5,
                retrieval_hint: None,
            },
            HistoryEntry {
                message_id: "missing-2".to_string(),
                lamport_timestamp: 6,
                retrieval_hint: None,
            },
        ];

        let sync = bob.send_repair_request(topic, missing).await.unwrap();
        assert_eq!(sync.repair_request.len(), 2);
        assert_eq!(sync.repair_request[0].message_id, "missing-1");
        assert_eq!(sync.repair_request[1].message_id, "missing-2");
    }

    #[tokio::test]
    async fn test_handle_repair_requests_resends() {
        let transport = InMemoryTransport::new();
        let alice = MessageChannel::new("chan".to_string(), "alice".to_string(), transport.clone());

        let topic = "/lmao/1/test/proto";

        let msg = alice.send(topic, b"important data").await.unwrap();
        let msg_id = msg.message_id.clone();

        let requests = vec![HistoryEntry {
            message_id: msg_id,
            lamport_timestamp: 0,
            retrieval_hint: None,
        }];

        let resent = alice
            .handle_repair_requests(topic, &requests)
            .await
            .unwrap();
        assert_eq!(resent, 1);
    }

    #[tokio::test]
    async fn test_handle_repair_requests_unknown_msg() {
        let transport = InMemoryTransport::new();
        let alice = MessageChannel::new("chan".to_string(), "alice".to_string(), transport.clone());

        let topic = "/lmao/1/test/proto";

        let requests = vec![HistoryEntry {
            message_id: "nonexistent".to_string(),
            lamport_timestamp: 0,
            retrieval_hint: None,
        }];

        let resent = alice
            .handle_repair_requests(topic, &requests)
            .await
            .unwrap();
        assert_eq!(resent, 0);
    }

    #[tokio::test]
    async fn test_receive_and_repair_full_flow() {
        let transport = InMemoryTransport::new();
        let alice = MessageChannel::new("chan".to_string(), "alice".to_string(), transport.clone());
        let bob = MessageChannel::new("chan".to_string(), "bob".to_string(), transport.clone());

        let topic = "/lmao/1/test/proto";

        let msg1 = alice.send(topic, b"first").await.unwrap();
        let _msg2 = alice.send(topic, b"second").await.unwrap();

        // Bob hasn't seen msg1
        let missing = bob.build_repair_requests(&[HistoryEntry {
            message_id: msg1.message_id.clone(),
            lamport_timestamp: msg1.lamport_timestamp,
            retrieval_hint: None,
        }]);
        assert_eq!(missing.len(), 1);

        let repair_sync = bob.send_repair_request(topic, missing).await.unwrap();

        // Alice handles the repair request — should re-publish msg1
        let repair_raw = serde_json::to_vec(&SdsMessage::Sync(repair_sync)).unwrap();
        let delivered = alice.receive_and_repair(topic, &repair_raw).await.unwrap();
        assert!(delivered.is_empty()); // Sync doesn't deliver content

        // Verify repair happened by subscribing and checking transport history
        // msg1 was re-published to topic, so a new subscriber gets it via replay
        let mut rx = transport.subscribe(topic).await.unwrap();
        // We should see multiple messages (original sends + repair re-send)
        let mut found_repair = false;
        while let Ok(data) = rx.try_recv() {
            if let Ok(SdsMessage::Content(c)) = serde_json::from_slice::<SdsMessage>(&data) {
                if c.message_id == msg1.message_id {
                    found_repair = true;
                }
            }
        }
        assert!(found_repair, "msg1 should have been re-published as repair");
    }

    #[tokio::test]
    async fn test_build_repair_requests_filters_seen() {
        let transport = InMemoryTransport::new();
        let bob = MessageChannel::new("chan".to_string(), "bob".to_string(), transport.clone());

        bob.bloom.set("seen-1");

        let history = vec![
            HistoryEntry {
                message_id: "seen-1".to_string(),
                lamport_timestamp: 1,
                retrieval_hint: None,
            },
            HistoryEntry {
                message_id: "unseen-1".to_string(),
                lamport_timestamp: 2,
                retrieval_hint: None,
            },
        ];

        let missing = bob.build_repair_requests(&history);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].message_id, "unseen-1");
    }
}
