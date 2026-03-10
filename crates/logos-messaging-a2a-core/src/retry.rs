use serde::{Deserialize, Serialize};

/// Configuration for exponential-backoff retry of message sends over
/// unreliable P2P transports.
///
/// Delay for attempt `n` (0-indexed) is:
///
/// ```text
/// min(base_delay_ms * 2^n, max_delay_ms)  [+ random jitter]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of send attempts (including the first try).
    pub max_attempts: u32,
    /// Base delay in milliseconds before the first retry.
    pub base_delay_ms: u64,
    /// Upper bound on the delay between retries, in milliseconds.
    pub max_delay_ms: u64,
    /// When `true`, add random jitter (0 .. delay) to each backoff interval
    /// to avoid thundering-herd problems.
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay_ms: 1_000,
            max_delay_ms: 60_000,
            jitter: true,
        }
    }
}

impl RetryConfig {
    /// Compute the delay (in milliseconds) for the given 0-indexed retry
    /// attempt.  Jitter, if enabled, is applied by the caller.
    pub fn delay_ms(&self, attempt: u32) -> u64 {
        let exp = self.base_delay_ms.saturating_mul(1u64 << attempt.min(31));
        exp.min(self.max_delay_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let cfg = RetryConfig::default();
        assert_eq!(cfg.max_attempts, 5);
        assert_eq!(cfg.base_delay_ms, 1_000);
        assert_eq!(cfg.max_delay_ms, 60_000);
        assert!(cfg.jitter);
    }

    #[test]
    fn test_delay_exponential() {
        let cfg = RetryConfig {
            max_attempts: 5,
            base_delay_ms: 1_000,
            max_delay_ms: 60_000,
            jitter: false,
        };
        assert_eq!(cfg.delay_ms(0), 1_000);
        assert_eq!(cfg.delay_ms(1), 2_000);
        assert_eq!(cfg.delay_ms(2), 4_000);
        assert_eq!(cfg.delay_ms(3), 8_000);
        assert_eq!(cfg.delay_ms(4), 16_000);
    }

    #[test]
    fn test_delay_capped() {
        let cfg = RetryConfig {
            max_attempts: 10,
            base_delay_ms: 1_000,
            max_delay_ms: 10_000,
            jitter: false,
        };
        assert_eq!(cfg.delay_ms(5), 10_000); // 32_000 capped to 10_000
        assert_eq!(cfg.delay_ms(9), 10_000);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let cfg = RetryConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: RetryConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.max_attempts, cfg.max_attempts);
        assert_eq!(deser.base_delay_ms, cfg.base_delay_ms);
        assert_eq!(deser.max_delay_ms, cfg.max_delay_ms);
        assert_eq!(deser.jitter, cfg.jitter);
    }
}
