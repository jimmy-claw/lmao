//! Lightweight observability counters for [`WakuA2ANode`](crate::WakuA2ANode).
//!
//! Every node embeds a [`Metrics`] instance that tracks key operational events
//! using lock-free [`AtomicU64`] counters. Call [`Metrics::snapshot`] to obtain
//! a serializable [`MetricsSnapshot`] suitable for JSON output or future
//! Prometheus/OpenTelemetry export.

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

/// Atomic counters for node operational events.
///
/// All counters are monotonically increasing and use [`Ordering::Relaxed`]
/// for maximum throughput — callers that need a consistent cross-counter view
/// should take a [`snapshot`](Self::snapshot).
#[derive(Debug)]
pub struct Metrics {
    /// Tasks sent to other agents.
    tasks_sent: AtomicU64,
    /// Tasks received from other agents.
    tasks_received: AtomicU64,
    /// Tasks that failed to send (transport or serialization errors).
    tasks_failed: AtomicU64,
    /// Messages published to Waku topics (discovery, presence, etc.).
    messages_published: AtomicU64,
    /// Discovery events — agent cards found via `discover()`.
    discovery_events: AtomicU64,
    /// Presence announcements broadcast by this node.
    presence_announcements: AtomicU64,
    /// Encryption operations (encrypt + decrypt).
    encryption_ops: AtomicU64,
    /// Conversation sessions created.
    sessions_created: AtomicU64,
    /// Individual retry attempts (each attempt within a retry loop).
    retry_attempts: AtomicU64,
    /// Retry exhaustions — all attempts failed.
    retry_exhaustions: AtomicU64,
}

/// A point-in-time, non-atomic copy of all [`Metrics`] counters.
///
/// Suitable for serialization (JSON output, logging, future export).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetricsSnapshot {
    /// Tasks sent to other agents.
    pub tasks_sent: u64,
    /// Tasks received from other agents.
    pub tasks_received: u64,
    /// Tasks that failed to send.
    pub tasks_failed: u64,
    /// Messages published to Waku topics.
    pub messages_published: u64,
    /// Discovery events — agent cards found.
    pub discovery_events: u64,
    /// Presence announcements broadcast.
    pub presence_announcements: u64,
    /// Encryption operations performed.
    pub encryption_ops: u64,
    /// Conversation sessions created.
    pub sessions_created: u64,
    /// Individual retry attempts.
    pub retry_attempts: u64,
    /// Retry exhaustions (all attempts failed).
    pub retry_exhaustions: u64,
}

impl Metrics {
    /// Create a new set of zeroed counters.
    pub fn new() -> Self {
        Self {
            tasks_sent: AtomicU64::new(0),
            tasks_received: AtomicU64::new(0),
            tasks_failed: AtomicU64::new(0),
            messages_published: AtomicU64::new(0),
            discovery_events: AtomicU64::new(0),
            presence_announcements: AtomicU64::new(0),
            encryption_ops: AtomicU64::new(0),
            sessions_created: AtomicU64::new(0),
            retry_attempts: AtomicU64::new(0),
            retry_exhaustions: AtomicU64::new(0),
        }
    }

    /// Take a consistent snapshot of all counters.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            tasks_sent: self.tasks_sent.load(Ordering::Relaxed),
            tasks_received: self.tasks_received.load(Ordering::Relaxed),
            tasks_failed: self.tasks_failed.load(Ordering::Relaxed),
            messages_published: self.messages_published.load(Ordering::Relaxed),
            discovery_events: self.discovery_events.load(Ordering::Relaxed),
            presence_announcements: self.presence_announcements.load(Ordering::Relaxed),
            encryption_ops: self.encryption_ops.load(Ordering::Relaxed),
            sessions_created: self.sessions_created.load(Ordering::Relaxed),
            retry_attempts: self.retry_attempts.load(Ordering::Relaxed),
            retry_exhaustions: self.retry_exhaustions.load(Ordering::Relaxed),
        }
    }

    /// Increment the tasks-sent counter by one.
    pub fn inc_tasks_sent(&self) {
        self.tasks_sent.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the tasks-received counter by the given amount.
    pub fn inc_tasks_received(&self, n: u64) {
        self.tasks_received.fetch_add(n, Ordering::Relaxed);
    }

    /// Increment the tasks-failed counter by one.
    pub fn inc_tasks_failed(&self) {
        self.tasks_failed.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the messages-published counter by one.
    pub fn inc_messages_published(&self) {
        self.messages_published.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the discovery-events counter by the given amount.
    pub fn inc_discovery_events(&self, n: u64) {
        self.discovery_events.fetch_add(n, Ordering::Relaxed);
    }

    /// Increment the presence-announcements counter by one.
    pub fn inc_presence_announcements(&self) {
        self.presence_announcements.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the encryption-ops counter by one.
    pub fn inc_encryption_ops(&self) {
        self.encryption_ops.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the sessions-created counter by one.
    pub fn inc_sessions_created(&self) {
        self.sessions_created.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the retry-attempts counter by one.
    pub fn inc_retry_attempts(&self) {
        self.retry_attempts.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment the retry-exhaustions counter by one.
    pub fn inc_retry_exhaustions(&self) {
        self.retry_exhaustions.fetch_add(1, Ordering::Relaxed);
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn new_metrics_are_zeroed() {
        let m = Metrics::new();
        let snap = m.snapshot();
        assert_eq!(snap.tasks_sent, 0);
        assert_eq!(snap.tasks_received, 0);
        assert_eq!(snap.tasks_failed, 0);
        assert_eq!(snap.messages_published, 0);
        assert_eq!(snap.discovery_events, 0);
        assert_eq!(snap.presence_announcements, 0);
        assert_eq!(snap.encryption_ops, 0);
        assert_eq!(snap.sessions_created, 0);
        assert_eq!(snap.retry_attempts, 0);
        assert_eq!(snap.retry_exhaustions, 0);
    }

    #[test]
    fn default_metrics_are_zeroed() {
        let m = Metrics::default();
        assert_eq!(m.snapshot(), Metrics::new().snapshot());
    }

    #[test]
    fn increment_tasks_sent() {
        let m = Metrics::new();
        m.inc_tasks_sent();
        m.inc_tasks_sent();
        assert_eq!(m.snapshot().tasks_sent, 2);
    }

    #[test]
    fn increment_tasks_received() {
        let m = Metrics::new();
        m.inc_tasks_received(3);
        m.inc_tasks_received(2);
        assert_eq!(m.snapshot().tasks_received, 5);
    }

    #[test]
    fn increment_tasks_failed() {
        let m = Metrics::new();
        m.inc_tasks_failed();
        assert_eq!(m.snapshot().tasks_failed, 1);
    }

    #[test]
    fn increment_messages_published() {
        let m = Metrics::new();
        m.inc_messages_published();
        m.inc_messages_published();
        m.inc_messages_published();
        assert_eq!(m.snapshot().messages_published, 3);
    }

    #[test]
    fn increment_discovery_events() {
        let m = Metrics::new();
        m.inc_discovery_events(5);
        assert_eq!(m.snapshot().discovery_events, 5);
    }

    #[test]
    fn increment_presence_announcements() {
        let m = Metrics::new();
        m.inc_presence_announcements();
        assert_eq!(m.snapshot().presence_announcements, 1);
    }

    #[test]
    fn increment_encryption_ops() {
        let m = Metrics::new();
        m.inc_encryption_ops();
        m.inc_encryption_ops();
        assert_eq!(m.snapshot().encryption_ops, 2);
    }

    #[test]
    fn increment_sessions_created() {
        let m = Metrics::new();
        m.inc_sessions_created();
        assert_eq!(m.snapshot().sessions_created, 1);
    }

    #[test]
    fn increment_retry_attempts() {
        let m = Metrics::new();
        m.inc_retry_attempts();
        m.inc_retry_attempts();
        m.inc_retry_attempts();
        assert_eq!(m.snapshot().retry_attempts, 3);
    }

    #[test]
    fn increment_retry_exhaustions() {
        let m = Metrics::new();
        m.inc_retry_exhaustions();
        assert_eq!(m.snapshot().retry_exhaustions, 1);
    }

    #[test]
    fn snapshot_is_independent_copy() {
        let m = Metrics::new();
        m.inc_tasks_sent();
        let snap1 = m.snapshot();
        m.inc_tasks_sent();
        let snap2 = m.snapshot();

        assert_eq!(snap1.tasks_sent, 1);
        assert_eq!(snap2.tasks_sent, 2);
    }

    #[test]
    fn snapshot_serializes_to_json() {
        let m = Metrics::new();
        m.inc_tasks_sent();
        m.inc_tasks_received(3);
        m.inc_encryption_ops();
        let snap = m.snapshot();
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["tasks_sent"], 1);
        assert_eq!(parsed["tasks_received"], 3);
        assert_eq!(parsed["encryption_ops"], 1);
        assert_eq!(parsed["tasks_failed"], 0);
    }

    #[test]
    fn snapshot_equality() {
        let m = Metrics::new();
        m.inc_tasks_sent();
        let s1 = m.snapshot();
        let s2 = m.snapshot();
        assert_eq!(s1, s2);
    }

    #[test]
    fn snapshot_clone() {
        let m = Metrics::new();
        m.inc_discovery_events(7);
        let snap = m.snapshot();
        let cloned = snap.clone();
        assert_eq!(snap, cloned);
    }

    #[test]
    fn concurrent_increments() {
        let m = Arc::new(Metrics::new());
        let mut handles = Vec::new();

        for _ in 0..10 {
            let m = Arc::clone(&m);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    m.inc_tasks_sent();
                    m.inc_tasks_received(1);
                    m.inc_messages_published();
                    m.inc_retry_attempts();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let snap = m.snapshot();
        assert_eq!(snap.tasks_sent, 10_000);
        assert_eq!(snap.tasks_received, 10_000);
        assert_eq!(snap.messages_published, 10_000);
        assert_eq!(snap.retry_attempts, 10_000);
    }

    #[test]
    fn all_counters_independent() {
        let m = Metrics::new();
        m.inc_tasks_sent();
        m.inc_tasks_received(2);
        m.inc_tasks_failed();
        m.inc_messages_published();
        m.inc_discovery_events(3);
        m.inc_presence_announcements();
        m.inc_encryption_ops();
        m.inc_sessions_created();
        m.inc_retry_attempts();
        m.inc_retry_exhaustions();

        let snap = m.snapshot();
        assert_eq!(snap.tasks_sent, 1);
        assert_eq!(snap.tasks_received, 2);
        assert_eq!(snap.tasks_failed, 1);
        assert_eq!(snap.messages_published, 1);
        assert_eq!(snap.discovery_events, 3);
        assert_eq!(snap.presence_announcements, 1);
        assert_eq!(snap.encryption_ops, 1);
        assert_eq!(snap.sessions_created, 1);
        assert_eq!(snap.retry_attempts, 1);
        assert_eq!(snap.retry_exhaustions, 1);
    }

    #[test]
    fn increment_by_zero_is_noop() {
        let m = Metrics::new();
        m.inc_tasks_received(0);
        m.inc_discovery_events(0);
        assert_eq!(m.snapshot().tasks_received, 0);
        assert_eq!(m.snapshot().discovery_events, 0);
    }

    #[test]
    fn large_increment() {
        let m = Metrics::new();
        m.inc_tasks_received(u64::MAX / 2);
        m.inc_tasks_received(1);
        assert_eq!(m.snapshot().tasks_received, u64::MAX / 2 + 1);
    }

    #[test]
    fn snapshot_debug_format() {
        let m = Metrics::new();
        m.inc_tasks_sent();
        let snap = m.snapshot();
        let debug = format!("{:?}", snap);
        assert!(debug.contains("tasks_sent: 1"));
    }

    #[test]
    fn metrics_debug_format() {
        let m = Metrics::new();
        let debug = format!("{:?}", m);
        assert!(debug.contains("Metrics"));
    }
}
