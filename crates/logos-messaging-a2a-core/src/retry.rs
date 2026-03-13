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

    #[test]
    fn test_delay_high_attempt_no_overflow() {
        let cfg = RetryConfig {
            max_attempts: 100,
            base_delay_ms: 1_000,
            max_delay_ms: u64::MAX,
            jitter: false,
        };
        // attempt 31 should be the max shift (1 << 31)
        let d31 = cfg.delay_ms(31);
        assert_eq!(d31, 1_000 * (1u64 << 31));
        // attempt 32+ should clamp shift at 31
        let d32 = cfg.delay_ms(32);
        assert_eq!(d32, d31);
        let d100 = cfg.delay_ms(100);
        assert_eq!(d100, d31);
    }

    #[test]
    fn test_delay_zero_base() {
        let cfg = RetryConfig {
            max_attempts: 5,
            base_delay_ms: 0,
            max_delay_ms: 60_000,
            jitter: false,
        };
        assert_eq!(cfg.delay_ms(0), 0);
        assert_eq!(cfg.delay_ms(3), 0);
    }

    #[test]
    fn test_delay_zero_max() {
        let cfg = RetryConfig {
            max_attempts: 5,
            base_delay_ms: 1_000,
            max_delay_ms: 0,
            jitter: false,
        };
        assert_eq!(cfg.delay_ms(0), 0);
        assert_eq!(cfg.delay_ms(3), 0);
    }

    #[test]
    fn test_delay_attempt_zero() {
        let cfg = RetryConfig {
            max_attempts: 3,
            base_delay_ms: 500,
            max_delay_ms: 10_000,
            jitter: false,
        };
        // 2^0 = 1, so delay = base_delay_ms
        assert_eq!(cfg.delay_ms(0), 500);
    }

    #[test]
    fn test_custom_config_serialization() {
        let cfg = RetryConfig {
            max_attempts: 10,
            base_delay_ms: 500,
            max_delay_ms: 30_000,
            jitter: false,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: RetryConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.max_attempts, 10);
        assert_eq!(deser.base_delay_ms, 500);
        assert_eq!(deser.max_delay_ms, 30_000);
        assert!(!deser.jitter);
    }

    #[test]
    fn test_clone_and_debug() {
        let cfg = RetryConfig::default();
        let cloned = cfg.clone();
        assert_eq!(cloned.max_attempts, cfg.max_attempts);
        let debug = format!("{:?}", cfg);
        assert!(debug.contains("RetryConfig"));
        assert!(debug.contains("max_attempts"));
    }

    #[test]
    fn test_delay_monotonically_increases_then_caps() {
        let cfg = RetryConfig {
            max_attempts: 10,
            base_delay_ms: 100,
            max_delay_ms: 5_000,
            jitter: false,
        };
        let mut prev = 0;
        for attempt in 0..10 {
            let delay = cfg.delay_ms(attempt);
            assert!(delay >= prev, "delay should be >= previous");
            prev = delay;
        }
        // Last few should all be capped at max
        assert_eq!(cfg.delay_ms(8), 5_000);
        assert_eq!(cfg.delay_ms(9), 5_000);
    }

    #[test]
    fn test_json_field_names() {
        let cfg = RetryConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("max_attempts").is_some());
        assert!(v.get("base_delay_ms").is_some());
        assert!(v.get("max_delay_ms").is_some());
        assert!(v.get("jitter").is_some());
    }

    #[test]
    fn test_deserialization_missing_fields_fails() {
        let json = r#"{"max_attempts":5}"#;
        let result = serde_json::from_str::<RetryConfig>(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_single_attempt_config() {
        let cfg = RetryConfig {
            max_attempts: 1,
            base_delay_ms: 100,
            max_delay_ms: 100,
            jitter: false,
        };
        assert_eq!(cfg.delay_ms(0), 100);
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: RetryConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.max_attempts, 1);
    }

    #[test]
    fn test_delay_at_exact_cap_boundary() {
        let cfg = RetryConfig {
            max_attempts: 5,
            base_delay_ms: 1_000,
            max_delay_ms: 8_000,
            jitter: false,
        };
        // 2^3 * 1000 = 8000 = max_delay_ms exactly
        assert_eq!(cfg.delay_ms(3), 8_000);
        // Next attempt should also be capped
        assert_eq!(cfg.delay_ms(4), 8_000);
    }

    #[test]
    fn test_base_equals_max() {
        let cfg = RetryConfig {
            max_attempts: 3,
            base_delay_ms: 5_000,
            max_delay_ms: 5_000,
            jitter: false,
        };
        // All delays should be the same
        assert_eq!(cfg.delay_ms(0), 5_000);
        assert_eq!(cfg.delay_ms(1), 5_000);
        assert_eq!(cfg.delay_ms(2), 5_000);
    }

    #[test]
    fn test_max_delay_less_than_base_caps_immediately() {
        let cfg = RetryConfig {
            max_attempts: 3,
            base_delay_ms: 10_000,
            max_delay_ms: 1_000,
            jitter: false,
        };
        // base * 2^0 = 10_000 but max is 1_000
        assert_eq!(cfg.delay_ms(0), 1_000);
    }

    #[test]
    fn test_deserialization_from_json_with_extra_fields() {
        let json = r#"{"max_attempts":3,"base_delay_ms":100,"max_delay_ms":1000,"jitter":true,"extra":42}"#;
        let cfg: RetryConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.max_attempts, 3);
    }

    #[test]
    fn test_max_attempts_zero() {
        let cfg = RetryConfig {
            max_attempts: 0,
            base_delay_ms: 1_000,
            max_delay_ms: 10_000,
            jitter: false,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: RetryConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.max_attempts, 0);
    }

    #[test]
    fn test_serde_json_value_roundtrip() {
        let cfg = RetryConfig::default();
        let value = serde_json::to_value(&cfg).unwrap();
        let deser: RetryConfig = serde_json::from_value(value).unwrap();
        assert_eq!(deser.max_attempts, cfg.max_attempts);
        assert_eq!(deser.base_delay_ms, cfg.base_delay_ms);
        assert_eq!(deser.max_delay_ms, cfg.max_delay_ms);
        assert_eq!(deser.jitter, cfg.jitter);
    }
}
