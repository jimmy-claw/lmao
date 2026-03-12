//! Retry layer for unreliable P2P transport sends.
//!
//! [`RetryLayer`] wraps the SDS [`MessageChannel`] send path and replays a
//! failed `send_reliable` call with exponential backoff according to a
//! [`RetryConfig`].

use anyhow::Result;
use logos_messaging_a2a_core::RetryConfig;
use logos_messaging_a2a_transport::sds::MessageChannel;
use logos_messaging_a2a_transport::Transport;
use std::time::Duration;

/// Retry wrapper around [`MessageChannel::send_reliable`].
///
/// When a send fails (returns `Err`), the layer retries up to
/// `config.max_attempts - 1` additional times with exponential backoff.
/// A successful send that is **not** ACKed (`Ok((_, false))`) is *not*
/// retried — the SDS layer already handles its own retransmission loop for
/// that case.
pub struct RetryLayer<'a, T: Transport> {
    channel: &'a MessageChannel<T>,
    config: &'a RetryConfig,
}

impl<'a, T: Transport> RetryLayer<'a, T> {
    /// Create a new retry layer wrapping the given channel with the specified config.
    pub fn new(channel: &'a MessageChannel<T>, config: &'a RetryConfig) -> Self {
        Self { channel, config }
    }

    /// Send with exponential-backoff retry on failure.
    ///
    /// Returns the same `(ContentMessage, was_acked)` tuple as
    /// `MessageChannel::send_reliable`.
    pub async fn send_reliable(
        &self,
        topic: &str,
        payload: &[u8],
    ) -> Result<(logos_messaging_a2a_transport::sds::ContentMessage, bool)> {
        let mut last_err = None;

        for attempt in 0..self.config.max_attempts {
            match self.channel.send_reliable(topic, payload).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    last_err = Some(e);
                    // Don't sleep after the final attempt
                    if attempt + 1 < self.config.max_attempts {
                        let delay = self.compute_delay(attempt);
                        tracing::debug!(
                            attempt = attempt + 1,
                            max_attempts = self.config.max_attempts,
                            delay_ms = delay.as_millis() as u64,
                            "Send attempt failed, retrying"
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_err
            .unwrap()
            .context(format!("all {} attempts failed", self.config.max_attempts)))
    }

    /// Compute the delay for a given 0-indexed attempt, with optional jitter.
    fn compute_delay(&self, attempt: u32) -> Duration {
        let base = self.config.delay_ms(attempt);
        let delay = if self.config.jitter {
            // Simple jitter: uniform random in [0, base]
            let jitter = fastrand::u64(0..=base);
            base.saturating_add(jitter) / 2
        } else {
            base
        };
        Duration::from_millis(delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use tokio::sync::mpsc;

    #[derive(Clone)]
    struct DummyTransport;

    #[async_trait]
    impl Transport for DummyTransport {
        async fn publish(&self, _topic: &str, _payload: &[u8]) -> anyhow::Result<()> {
            Ok(())
        }
        async fn subscribe(&self, _topic: &str) -> anyhow::Result<mpsc::Receiver<Vec<u8>>> {
            let (_, rx) = mpsc::channel(1);
            Ok(rx)
        }
        async fn unsubscribe(&self, _topic: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn make_layer<'a>(
        channel: &'a MessageChannel<DummyTransport>,
        config: &'a RetryConfig,
    ) -> RetryLayer<'a, DummyTransport> {
        RetryLayer::new(channel, config)
    }

    #[test]
    fn compute_delay_no_jitter_base_case() {
        let config = RetryConfig {
            max_attempts: 5,
            base_delay_ms: 100,
            max_delay_ms: 10_000,
            jitter: false,
        };
        let channel = MessageChannel::new("test".into(), "test-pk".into(), DummyTransport);
        let layer = make_layer(&channel, &config);
        assert_eq!(layer.compute_delay(0), Duration::from_millis(100));
    }

    #[test]
    fn compute_delay_exponential_growth() {
        let config = RetryConfig {
            max_attempts: 10,
            base_delay_ms: 100,
            max_delay_ms: 100_000,
            jitter: false,
        };
        let channel = MessageChannel::new("test".into(), "test-pk".into(), DummyTransport);
        let layer = make_layer(&channel, &config);

        assert_eq!(layer.compute_delay(0), Duration::from_millis(100));
        assert_eq!(layer.compute_delay(1), Duration::from_millis(200));
        assert_eq!(layer.compute_delay(2), Duration::from_millis(400));
        assert_eq!(layer.compute_delay(3), Duration::from_millis(800));
        assert_eq!(layer.compute_delay(4), Duration::from_millis(1600));
    }

    #[test]
    fn compute_delay_capped_at_max_delay() {
        let config = RetryConfig {
            max_attempts: 10,
            base_delay_ms: 1000,
            max_delay_ms: 5000,
            jitter: false,
        };
        let channel = MessageChannel::new("test".into(), "test-pk".into(), DummyTransport);
        let layer = make_layer(&channel, &config);

        // attempt 0: 1000
        assert_eq!(layer.compute_delay(0), Duration::from_millis(1000));
        // attempt 1: 2000
        assert_eq!(layer.compute_delay(1), Duration::from_millis(2000));
        // attempt 2: 4000
        assert_eq!(layer.compute_delay(2), Duration::from_millis(4000));
        // attempt 3: 8000 capped to 5000
        assert_eq!(layer.compute_delay(3), Duration::from_millis(5000));
        // attempt 10: still capped
        assert_eq!(layer.compute_delay(10), Duration::from_millis(5000));
    }

    #[test]
    fn compute_delay_with_jitter_is_bounded() {
        let config = RetryConfig {
            max_attempts: 5,
            base_delay_ms: 1000,
            max_delay_ms: 60_000,
            jitter: true,
        };
        let channel = MessageChannel::new("test".into(), "test-pk".into(), DummyTransport);
        let layer = make_layer(&channel, &config);

        // With jitter: delay = (base + rand(0..=base)) / 2
        // For attempt 0, base = 1000
        // min = (1000 + 0) / 2 = 500
        // max = (1000 + 1000) / 2 = 1000
        for _ in 0..100 {
            let delay = layer.compute_delay(0);
            assert!(delay >= Duration::from_millis(500));
            assert!(delay <= Duration::from_millis(1000));
        }
    }

    #[test]
    fn compute_delay_high_attempt_does_not_overflow() {
        let config = RetryConfig {
            max_attempts: 100,
            base_delay_ms: 1000,
            max_delay_ms: 60_000,
            jitter: false,
        };
        let channel = MessageChannel::new("test".into(), "test-pk".into(), DummyTransport);
        let layer = make_layer(&channel, &config);

        // Very high attempt should be capped, not overflow
        let delay = layer.compute_delay(50);
        assert_eq!(delay, Duration::from_millis(60_000));
    }

    #[test]
    fn compute_delay_zero_base_delay() {
        let config = RetryConfig {
            max_attempts: 5,
            base_delay_ms: 0,
            max_delay_ms: 10_000,
            jitter: false,
        };
        let channel = MessageChannel::new("test".into(), "test-pk".into(), DummyTransport);
        let layer = make_layer(&channel, &config);

        assert_eq!(layer.compute_delay(0), Duration::from_millis(0));
        assert_eq!(layer.compute_delay(5), Duration::from_millis(0));
    }
}
