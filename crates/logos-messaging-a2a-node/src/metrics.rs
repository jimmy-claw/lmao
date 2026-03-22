//! Lightweight observability counters for [`WakuA2ANode`](crate::WakuA2ANode).
//!
//! All counters are [`AtomicU64`] — lock-free, zero-allocation increments.
//! Call [`Metrics::snapshot`] to get a serializable [`MetricsSnapshot`] of the
//! current values.

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

/// Atomic counters for node operations.
///
/// Shared across all methods of a [`WakuA2ANode`](crate::WakuA2ANode) via an
/// internal `Arc`-free field (the node itself is typically `Arc`-wrapped by
/// callers).  Every counter uses [`Ordering::Relaxed`] — sufficient for
/// monotonic counters that are only read for display / export.
#[derive(Debug)]
pub struct Metrics {
    /// Tasks sent (via `send_task` / `send_task_to`).
    pub tasks_sent: AtomicU64,
    /// Tasks received (extracted from `poll_tasks`).
    pub tasks_received: AtomicU64,
    /// Task sends that failed (transport or other error in `send_task_to`).
    pub tasks_failed: AtomicU64,
    /// Messages published to transport (all topics).
    pub messages_published: AtomicU64,
    /// Messages received from subscriptions.
    pub messages_received: AtomicU64,
    /// Agent cards discovered (via `discover` / `discover_all`).
    pub discoveries: AtomicU64,
    /// Presence announcements sent.
    pub announcements_sent: AtomicU64,
    /// Presence announcements received (peers tracked).
    pub peers_discovered: AtomicU64,
    /// Payloads encrypted before sending.
    pub encryptions: AtomicU64,
    /// Payloads decrypted on receive.
    pub decryptions: AtomicU64,
    /// Sessions created (both local and auto-created on receive).
    pub sessions_created: AtomicU64,
    /// Delegation requests sent.
    pub delegations_sent: AtomicU64,
    /// Stream chunks published.
    pub stream_chunks_sent: AtomicU64,
    /// Stream chunks received.
    pub stream_chunks_received: AtomicU64,
    /// Retry attempts (transport-level retries).
    pub retry_attempts: AtomicU64,
    /// Retries exhausted (all attempts failed).
    pub retries_exhausted: AtomicU64,
    /// Responses sent (via `respond` / `respond_to`).
    pub responses_sent: AtomicU64,
}

impl Metrics {
    /// Create a new zeroed metrics instance.
    pub fn new() -> Self {
        Self {
            tasks_sent: AtomicU64::new(0),
            tasks_received: AtomicU64::new(0),
            tasks_failed: AtomicU64::new(0),
            messages_published: AtomicU64::new(0),
            messages_received: AtomicU64::new(0),
            discoveries: AtomicU64::new(0),
            announcements_sent: AtomicU64::new(0),
            peers_discovered: AtomicU64::new(0),
            encryptions: AtomicU64::new(0),
            decryptions: AtomicU64::new(0),
            sessions_created: AtomicU64::new(0),
            delegations_sent: AtomicU64::new(0),
            stream_chunks_sent: AtomicU64::new(0),
            stream_chunks_received: AtomicU64::new(0),
            retry_attempts: AtomicU64::new(0),
            retries_exhausted: AtomicU64::new(0),
            responses_sent: AtomicU64::new(0),
        }
    }

    /// Increment a counter by 1.
    #[inline]
    pub(crate) fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment a counter by an arbitrary amount.
    #[inline]
    pub(crate) fn inc_by(counter: &AtomicU64, n: u64) {
        counter.fetch_add(n, Ordering::Relaxed);
    }

    /// Take a point-in-time snapshot of all counters.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            tasks_sent: self.tasks_sent.load(Ordering::Relaxed),
            tasks_received: self.tasks_received.load(Ordering::Relaxed),
            tasks_failed: self.tasks_failed.load(Ordering::Relaxed),
            messages_published: self.messages_published.load(Ordering::Relaxed),
            messages_received: self.messages_received.load(Ordering::Relaxed),
            discoveries: self.discoveries.load(Ordering::Relaxed),
            announcements_sent: self.announcements_sent.load(Ordering::Relaxed),
            peers_discovered: self.peers_discovered.load(Ordering::Relaxed),
            encryptions: self.encryptions.load(Ordering::Relaxed),
            decryptions: self.decryptions.load(Ordering::Relaxed),
            sessions_created: self.sessions_created.load(Ordering::Relaxed),
            delegations_sent: self.delegations_sent.load(Ordering::Relaxed),
            stream_chunks_sent: self.stream_chunks_sent.load(Ordering::Relaxed),
            stream_chunks_received: self.stream_chunks_received.load(Ordering::Relaxed),
            retry_attempts: self.retry_attempts.load(Ordering::Relaxed),
            retries_exhausted: self.retries_exhausted.load(Ordering::Relaxed),
            responses_sent: self.responses_sent.load(Ordering::Relaxed),
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Serializable point-in-time snapshot of all metrics counters.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub tasks_sent: u64,
    pub tasks_received: u64,
    pub tasks_failed: u64,
    pub messages_published: u64,
    pub messages_received: u64,
    pub discoveries: u64,
    pub announcements_sent: u64,
    pub peers_discovered: u64,
    pub encryptions: u64,
    pub decryptions: u64,
    pub sessions_created: u64,
    pub delegations_sent: u64,
    pub stream_chunks_sent: u64,
    pub stream_chunks_received: u64,
    pub retry_attempts: u64,
    pub retries_exhausted: u64,
    pub responses_sent: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_metrics_are_zeroed() {
        let m = Metrics::new();
        let snap = m.snapshot();
        assert_eq!(snap.tasks_sent, 0);
        assert_eq!(snap.tasks_received, 0);
        assert_eq!(snap.tasks_failed, 0);
        assert_eq!(snap.messages_published, 0);
        assert_eq!(snap.messages_received, 0);
        assert_eq!(snap.discoveries, 0);
        assert_eq!(snap.announcements_sent, 0);
        assert_eq!(snap.peers_discovered, 0);
        assert_eq!(snap.encryptions, 0);
        assert_eq!(snap.decryptions, 0);
        assert_eq!(snap.sessions_created, 0);
        assert_eq!(snap.delegations_sent, 0);
        assert_eq!(snap.stream_chunks_sent, 0);
        assert_eq!(snap.stream_chunks_received, 0);
        assert_eq!(snap.retry_attempts, 0);
        assert_eq!(snap.retries_exhausted, 0);
        assert_eq!(snap.responses_sent, 0);
    }

    #[test]
    fn default_is_zeroed() {
        let m = Metrics::default();
        assert_eq!(m.snapshot().tasks_sent, 0);
    }

    #[test]
    fn inc_increments_by_one() {
        let m = Metrics::new();
        Metrics::inc(&m.tasks_sent);
        Metrics::inc(&m.tasks_sent);
        Metrics::inc(&m.tasks_sent);
        assert_eq!(m.snapshot().tasks_sent, 3);
    }

    #[test]
    fn inc_by_increments_by_n() {
        let m = Metrics::new();
        Metrics::inc_by(&m.tasks_received, 5);
        Metrics::inc_by(&m.tasks_received, 3);
        assert_eq!(m.snapshot().tasks_received, 8);
    }

    #[test]
    fn snapshot_is_independent_of_later_increments() {
        let m = Metrics::new();
        Metrics::inc(&m.tasks_sent);
        let snap1 = m.snapshot();
        Metrics::inc(&m.tasks_sent);
        let snap2 = m.snapshot();
        assert_eq!(snap1.tasks_sent, 1);
        assert_eq!(snap2.tasks_sent, 2);
    }

    #[test]
    fn snapshot_serializes_to_json() {
        let m = Metrics::new();
        Metrics::inc(&m.tasks_sent);
        Metrics::inc(&m.discoveries);
        let snap = m.snapshot();
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["tasks_sent"], 1);
        assert_eq!(parsed["discoveries"], 1);
        assert_eq!(parsed["tasks_received"], 0);
    }

    #[test]
    fn concurrent_increments_are_correct() {
        use std::sync::Arc;
        use std::thread;

        let m = Arc::new(Metrics::new());
        let mut handles = Vec::new();
        for _ in 0..10 {
            let m = Arc::clone(&m);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    Metrics::inc(&m.tasks_sent);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(m.snapshot().tasks_sent, 10_000);
    }

    #[test]
    fn each_counter_is_independent() {
        let m = Metrics::new();
        Metrics::inc(&m.tasks_sent);
        Metrics::inc(&m.tasks_received);
        Metrics::inc(&m.tasks_received);
        Metrics::inc(&m.encryptions);
        let snap = m.snapshot();
        assert_eq!(snap.tasks_sent, 1);
        assert_eq!(snap.tasks_received, 2);
        assert_eq!(snap.encryptions, 1);
        assert_eq!(snap.tasks_failed, 0);
    }

    #[test]
    fn snapshot_equality() {
        let m = Metrics::new();
        let s1 = m.snapshot();
        let s2 = m.snapshot();
        assert_eq!(s1, s2);
    }

    #[test]
    fn snapshot_clone() {
        let m = Metrics::new();
        Metrics::inc(&m.tasks_sent);
        let snap = m.snapshot();
        let cloned = snap.clone();
        assert_eq!(snap, cloned);
    }
}
