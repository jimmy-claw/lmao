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
//! # async fn example() -> Result<(), logos_messaging_a2a_transport::TransportError> {
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
//!
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
