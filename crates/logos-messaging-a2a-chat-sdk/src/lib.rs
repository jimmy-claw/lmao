//! Rust bindings for Logos Chat SDK — agent-to-agent encrypted sessions.
//!
//! This crate bridges the LMAO agent stack with Logos Chat SDK crypto primitives,
//! providing:
//!
//! - [`ChatSession`] — manages an encrypted session between two agents using
//!   X25519 ECDH key agreement and ChaCha20-Poly1305 AEAD, matching the Chat SDK
//!   crypto layer.
//! - [`IntroBundleTopic`] — content topic helpers for broadcasting and discovering
//!   agent intro bundles over Logos Messaging (replacing out-of-band exchange).
//! - [`GroupSession`] — ephemeral multi-agent session type for task-scoped groups
//!   (foundation for MLS group chat support in Chat SDK v0.2).
//!
//! # Agent-to-agent use case
//!
//! Agents discover each other's [`IntroBundle`] on a well-known Logos Messaging
//! content topic, establish pairwise encrypted sessions via ECDH, and communicate
//! with forward secrecy per task thread. This gives agent fleets the same crypto
//! guarantees as Logos Chat users.

mod chat_session;
mod group_session;
mod intro_topic;

pub use chat_session::{ChatSession, ChatSessionError};
pub use group_session::{GroupMember, GroupSession, GroupSessionError};
pub use intro_topic::{intro_bundle_topic, INTRO_BUNDLE_DISCOVERY};

// Re-export crypto primitives so consumers don't need a direct dependency.
pub use logos_messaging_a2a_crypto::{AgentIdentity, EncryptedPayload, IntroBundle, SessionKey};
