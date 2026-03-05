//! Bloom filter wrapper for SDS message deduplication.
//!
//! Replaces the naive HashSet approach with a space-efficient bloom filter
//! as specified in the SDS protocol.

use bloomfilter::Bloom;
use std::sync::Mutex;

/// Default bloom filter capacity (number of items).
const DEFAULT_CAPACITY: usize = 10_000;
/// Default false positive rate.
const DEFAULT_ERROR_RATE: f64 = 0.001;

/// Thread-safe bloom filter for SDS deduplication.
pub struct SdsBloomFilter {
    inner: Mutex<Bloom<str>>,
    capacity: usize,
    item_count: Mutex<usize>,
}

impl SdsBloomFilter {
    /// Create a new bloom filter with default parameters.
    pub fn new() -> Self {
        Self::with_params(DEFAULT_CAPACITY, DEFAULT_ERROR_RATE)
    }

    /// Create a bloom filter with custom capacity and error rate.
    pub fn with_params(capacity: usize, error_rate: f64) -> Self {
        Self {
            inner: Mutex::new(Bloom::new_for_fp_rate(capacity, error_rate)),
            capacity,
            item_count: Mutex::new(0),
        }
    }

    /// Check if a message ID has probably been seen.
    pub fn check(&self, message_id: &str) -> bool {
        self.inner.lock().unwrap().check(message_id)
    }

    /// Add a message ID to the filter.
    pub fn set(&self, message_id: &str) {
        let mut filter = self.inner.lock().unwrap();
        filter.set(message_id);
        let mut count = self.item_count.lock().unwrap();
        *count += 1;

        // Auto-reset if we exceed capacity to avoid excessive false positives
        if *count >= self.capacity {
            *filter = Bloom::new_for_fp_rate(self.capacity, DEFAULT_ERROR_RATE);
            *count = 0;
            // Note: this means we lose dedup state for old messages.
            // In practice, old messages should already be in local history.
            eprintln!(
                "[sds::bloom] filter reset after reaching capacity {}",
                self.capacity
            );
        }
    }

    /// Check and set atomically — returns true if already seen.
    pub fn check_and_set(&self, message_id: &str) -> bool {
        let mut filter = self.inner.lock().unwrap();
        let seen = filter.check(message_id);
        if !seen {
            filter.set(message_id);
            let mut count = self.item_count.lock().unwrap();
            *count += 1;
        }
        seen
    }

    /// Serialize the bloom filter to bytes for inclusion in SDS messages.
    ///
    /// The format is: [8 bytes bitmap_bits LE][4 bytes k_num LE][32 bytes sip_keys][bitmap...]
    pub fn to_bytes(&self) -> Vec<u8> {
        let filter = self.inner.lock().unwrap();
        let bitmap = filter.bitmap();
        let bitmap_bits = filter.number_of_bits();
        let k_num = filter.number_of_hash_functions();
        let sip_keys = filter.sip_keys();

        let mut out = Vec::with_capacity(8 + 4 + 32 + bitmap.len());
        out.extend_from_slice(&bitmap_bits.to_le_bytes());
        out.extend_from_slice(&k_num.to_le_bytes());
        for &(a, b) in &sip_keys {
            out.extend_from_slice(&a.to_le_bytes());
            out.extend_from_slice(&b.to_le_bytes());
        }
        out.extend_from_slice(&bitmap);
        out
    }

    /// Deserialize a bloom filter from bytes produced by [`to_bytes`].
    ///
    /// Returns `None` if the data is too short or malformed.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        // Header: 8 (bitmap_bits) + 4 (k_num) + 32 (sip_keys) = 44 bytes
        if data.len() < 44 {
            return None;
        }
        let bitmap_bits = u64::from_le_bytes(data[0..8].try_into().ok()?);
        let k_num = u32::from_le_bytes(data[8..12].try_into().ok()?);
        let mut sip_keys = [(0u64, 0u64); 2];
        for (i, sip_key) in sip_keys.iter_mut().enumerate() {
            let offset = 12 + i * 16;
            let a = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            let b = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().ok()?);
            *sip_key = (a, b);
        }
        let bitmap = &data[44..];
        let filter = Bloom::from_existing(bitmap, bitmap_bits, k_num, sip_keys);
        Some(Self {
            inner: Mutex::new(filter),
            capacity: DEFAULT_CAPACITY,
            item_count: Mutex::new(0), // unknown after deserialization
        })
    }

    /// Check if a message ID is probably in this filter (for remote bloom checks).
    /// This is a convenience for checking against deserialized remote filters.
    pub fn probably_contains(&self, message_id: &str) -> bool {
        self.check(message_id)
    }

    /// Number of items added since last reset.
    pub fn len(&self) -> usize {
        *self.item_count.lock().unwrap()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for SdsBloomFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_check_and_set() {
        let filter = SdsBloomFilter::new();
        assert!(!filter.check("msg-1"));
        assert!(!filter.check_and_set("msg-1"));
        assert!(filter.check("msg-1"));
        assert!(filter.check_and_set("msg-1"));
    }

    #[test]
    fn test_bloom_no_false_negatives() {
        let filter = SdsBloomFilter::new();
        for i in 0..100 {
            let id = format!("msg-{}", i);
            filter.set(&id);
        }
        for i in 0..100 {
            let id = format!("msg-{}", i);
            assert!(filter.check(&id), "false negative for {}", id);
        }
    }

    #[test]
    fn test_bloom_serialization() {
        let filter = SdsBloomFilter::new();
        filter.set("test-msg");
        let bytes = filter.to_bytes();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn test_bloom_roundtrip() {
        let filter = SdsBloomFilter::new();
        for i in 0..50 {
            filter.set(&format!("msg-{}", i));
        }
        let bytes = filter.to_bytes();
        let restored = SdsBloomFilter::from_bytes(&bytes).expect("deserialization failed");
        for i in 0..50 {
            assert!(restored.check(&format!("msg-{}", i)), "missing msg-{}", i);
        }
        assert!(!restored.check("never-added"));
    }
}
