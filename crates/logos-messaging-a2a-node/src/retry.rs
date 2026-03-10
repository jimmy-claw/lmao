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
                        eprintln!(
                            "[retry] Attempt {}/{} failed, retrying in {}ms",
                            attempt + 1,
                            self.config.max_attempts,
                            delay.as_millis(),
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
