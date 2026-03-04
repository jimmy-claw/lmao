//! SDS (Scalable Data Sync) — reliable delivery layer for Logos Messaging.
//!
//! This module implements the SDS protocol as specified by the Logos Messaging team,
//! providing reliable, causally-ordered message delivery over fire-and-forget transports.
//!
//! ## Architecture
//!
//! - **MessageChannel**: The main API. Wraps a Transport with SDS reliability.
//! - **SdsBloomFilter**: Space-efficient deduplication (replaces naive HashSet).
//! - **Message types**: Content (ordered), Sync (consistency), Ephemeral (fire-and-forget).
//!
//! ## Usage
//!
//! ```rust,no_run
//! use logos_messaging_a2a_transport::sds::channel::MessageChannel;
//! use logos_messaging_a2a_transport::memory::InMemoryTransport;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let transport = InMemoryTransport::new();
//! let channel = MessageChannel::new(
//!     "my-channel".to_string(),
//!     "agent-alice".to_string(),
//!     transport,
//! );
//!
//! // Send a causally ordered message
//! let msg = channel.send("/lmao/1/tasks/proto", b"hello").await?;
//!
//! // Send with explicit ACK reliability
//! let (msg, acked) = channel.send_reliable("/lmao/1/tasks/proto", b"important").await?;
//!
//! // Send ephemeral (fire-and-forget)
//! channel.send_ephemeral("/lmao/1/status/proto", b"heartbeat").await?;
//!
//! // Send sync for consistency protocol
//! channel.send_sync("/lmao/1/sync/proto").await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Migration from minimal SDS
//!
//! The old `SdsTransport` wrapper is preserved as `LegacySdsTransport` for
//! backward compatibility. New code should use `MessageChannel` directly.
//!
//! Reference: <https://forum.research.logos.co/t/introducing-the-reliable-channel-api/580>

pub mod bloom;
pub mod channel;
pub mod message;

// Re-export key types for convenience
pub use bloom::SdsBloomFilter;
pub use channel::{ChannelConfig, MessageChannel};
pub use message::{
    compute_message_id, ChannelId, ContentMessage, EphemeralMessage, HistoryEntry, MessageId,
    ParticipantId, SdsMessage, SyncMessage,
};

// --- Legacy compatibility layer ---

use crate::Transport;
use anyhow::Result;
use std::time::Duration;

/// Legacy SDS configuration (backward compat).
#[derive(Debug, Clone)]
pub struct SdsConfig {
    pub ack_timeout: Duration,
    pub max_retries: u32,
}

impl Default for SdsConfig {
    fn default() -> Self {
        Self {
            ack_timeout: Duration::from_secs(10),
            max_retries: 3,
        }
    }
}

impl SdsConfig {
    pub fn fire_and_forget() -> Self {
        Self {
            ack_timeout: Duration::from_millis(0),
            max_retries: 0,
        }
    }
}

/// Legacy SDS wrapper — preserved for backward compatibility.
/// New code should use `MessageChannel` instead.
pub struct LegacySdsTransport<T: Transport> {
    channel: MessageChannel<T>,
}

impl<T: Transport> LegacySdsTransport<T> {
    pub fn new(transport: T) -> Self {
        Self::with_config(transport, SdsConfig::default())
    }

    pub fn with_config(transport: T, config: SdsConfig) -> Self {
        let channel_config = ChannelConfig {
            ack_timeout: config.ack_timeout,
            max_retries: config.max_retries,
            ..Default::default()
        };
        Self {
            channel: MessageChannel::with_config(
                "legacy".to_string(),
                "legacy".to_string(),
                transport,
                channel_config,
            ),
        }
    }

    pub fn inner(&self) -> &T {
        self.channel.transport()
    }

    pub async fn publish_reliable(
        &self,
        topic: &str,
        payload: &[u8],
        message_id: &str,
    ) -> Result<bool> {
        let ack_topic = format!("/lmao/1/ack/{}/proto", message_id);
        let transport = self.channel.transport();
        let mut ack_rx = transport.subscribe(&ack_topic).await?;
        let config = &self.channel.config();

        for attempt in 0..=config.max_retries {
            if attempt > 0 {
                eprintln!(
                    "[sds] retransmit attempt {}/{} for {}",
                    attempt, config.max_retries, message_id
                );
            }

            transport.publish(topic, payload).await?;

            match tokio::time::timeout(config.ack_timeout, ack_rx.recv()).await {
                Ok(Some(ack_data)) => {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&ack_data) {
                        if val.get("message_id").and_then(|v| v.as_str()) == Some(message_id) {
                            let _ = transport.unsubscribe(&ack_topic).await;
                            return Ok(true);
                        }
                    }
                }
                _ => continue,
            }
        }

        let _ = transport.unsubscribe(&ack_topic).await;
        eprintln!(
            "[sds] no ACK after {} retries for {}",
            config.max_retries, message_id
        );
        Ok(false)
    }

    pub async fn send_ack(&self, message_id: &str) -> Result<()> {
        self.channel.send_ack("", message_id).await
    }

    pub fn is_duplicate(&self, message_id: &str) -> bool {
        self.channel.is_duplicate(message_id)
    }

    pub fn mark_seen(&self, message_id: &str) {
        self.channel.bloom.set(message_id);
    }

    pub fn filter_dedup(&self, messages: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        messages
            .into_iter()
            .filter(|msg| {
                if let Ok(envelope) = serde_json::from_slice::<serde_json::Value>(msg) {
                    if let Some(id) = envelope.get("id").and_then(|v| v.as_str()) {
                        if self.is_duplicate(id) {
                            return false;
                        }
                        self.mark_seen(id);
                    }
                }
                true
            })
            .collect()
    }
}

/// Backward-compatible alias. Use `MessageChannel` for new code.
pub type SdsTransport<T> = LegacySdsTransport<T>;
