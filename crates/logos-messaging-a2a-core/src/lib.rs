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
//! - [`delegation`] — Multi-agent task delegation types ([`DelegationRequest`], [`DelegationResult`], [`DelegationStrategy`]).
//! - [`envelope`] — Wire envelope ([`A2AEnvelope`]) for all Waku messages.
//! - [`task`] — Task lifecycle types ([`Task`], [`TaskState`], [`Message`], [`Part`]).
//! - [`topics`] — Waku content topic string helpers.
//! - [`presence`] — Signed presence announcements for ephemeral discovery.
//! - [`registry`] — Persistent agent registry trait and in-memory implementation.
//! - [`retry`] — Exponential-backoff retry configuration.

/// Agent identity and capability advertisement types.
pub mod agent;
/// Multi-agent task delegation types and strategies.
pub mod delegation;
/// Wire envelope for all Waku messages.
pub mod envelope;
/// Signed presence announcements for ephemeral peer discovery.
pub mod presence;
pub mod registry;
/// Exponential-backoff retry configuration.
pub mod retry;
/// Task lifecycle types and message parts.
pub mod task;
pub mod topics;

// Re-export everything at crate root so existing imports don't break.
pub use agent::*;
pub use delegation::*;
pub use envelope::*;
pub use presence::*;
pub use retry::*;
pub use task::*;
