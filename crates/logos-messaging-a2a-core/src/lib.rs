//! Core protocol types for the Logos Messaging A2A (Agent-to-Agent) protocol.
//!
//! This crate defines the shared data structures, envelopes, and topic helpers
//! used across the LMAO stack. It is transport-agnostic — no networking code
//! lives here — so both the Waku-based node implementation and FFI bindings
//! depend on it for canonical type definitions.
//!
//! # Modules
//!
//! - [`agent`] — Agent identity and capability advertisement ([`AgentCard`]).
//! - [`envelope`] — Wire envelope ([`A2AEnvelope`]) for all Waku messages.
//! - [`task`] — Task lifecycle types ([`Task`], [`TaskState`], [`Message`], [`Part`]).
//! - [`topics`] — Waku content topic string helpers.
//! - [`presence`] — Signed presence announcements for ephemeral discovery.
//! - [`registry`] — Persistent agent registry trait and in-memory implementation.
//! - [`retry`] — Exponential-backoff retry configuration.

pub mod agent;
pub mod envelope;
pub mod presence;
pub mod registry;
pub mod retry;
pub mod task;
pub mod topics;

// Re-export everything at crate root so existing imports don't break.
pub use agent::*;
pub use envelope::*;
pub use presence::*;
pub use retry::*;
pub use task::*;
