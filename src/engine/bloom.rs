//! Bloom filter for probabilistic set membership testing.
//!
//! False positives are possible; false negatives are impossible.

/// Probabilistic set membership filter.
/// False positives possible; false negatives impossible.
pub struct BloomFilter {
    bits: Vec<u64>,
    num_hashes: usize,
}

impl BloomFilter {
    /// Create a new bloom filter sized for `expected_items` with target false positive rate `fp_rate`.
    pub fn new(expected_items: usize, fp_rate: f64) -> Self {
        let fp_rate = fp_rate.clamp(1e-9, 0.999);
        let n = expected_items.max(1);

        // Optimal bit count: m = -(n * ln(p)) / (ln(2)^2)
        let m = (-(n as f64 * fp_rate.ln()) / (2.0_f64.ln().powi(2))).ceil() as usize;
        // Round up to a multiple of 64 for clean u64 storage
        let num_words = m.div_ceil(64);
        let num_bits = num_words * 64;

        // Optimal hash count: k = (m/n) * ln(2)
        let k = ((num_bits as f64 / n as f64) * 2.0_f64.ln()).round() as usize;
        let num_hashes = k.max(1);

        Self {
            bits: vec![0u64; num_words],
            num_hashes,
        }
    }

    /// Insert a key into the filter.
    pub fn insert(&mut self, key: u32) {
        let (h1, h2) = self.hash_pair(key);
        let num_bits = self.bits.len() * 64;
        for i in 0..self.num_hashes {
            let idx = h1.wrapping_add((i as u64).wrapping_mul(h2)) % (num_bits as u64);
            let word = idx as usize / 64;
            let bit = idx as usize % 64;
            self.bits[word] |= 1u64 << bit;
        }
    }

    /// Check if a key might be in the filter.
    /// Returns `true` if the key is possibly present (may be a false positive).
    /// Returns `false` if the key is definitely not present (never a false negative).
    pub fn contains(&self, key: u32) -> bool {
        let (h1, h2) = self.hash_pair(key);
        let num_bits = self.bits.len() * 64;
        for i in 0..self.num_hashes {
            let idx = h1.wrapping_add((i as u64).wrapping_mul(h2)) % (num_bits as u64);
            let word = idx as usize / 64;
            let bit = idx as usize % 64;
            if self.bits[word] & (1u64 << bit) == 0 {
                return false;
            }
        }
        true
    }

    /// Compute two independent hash values using fast multiply-shift.
    fn hash_pair(&self, key: u32) -> (u64, u64) {
        let k = key as u64;
        let h1 = k
            .wrapping_mul(0xc6a4_a793_5bd1_e995)
            .wrapping_add(0x9e37_79b9_7f4a_7c15);
        let h2 = k
            .wrapping_mul(0x517c_c1b7_2722_0a95)
            .wrapping_add(0x5bd1_e995_c6a4_a793);
        (h1, h2)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_contains() {
        let mut bf = BloomFilter::new(100, 0.01);
        bf.insert(42);
        bf.insert(0);
        bf.insert(u32::MAX);

        assert!(bf.contains(42));
        assert!(bf.contains(0));
        assert!(bf.contains(u32::MAX));
    }

    #[test]
    fn no_false_negatives() {
        let mut bf = BloomFilter::new(1000, 0.01);
        for i in 0u32..500 {
            bf.insert(i);
        }
        for i in 0u32..500 {
            assert!(bf.contains(i), "False negative for key {}", i);
        }
    }

    #[test]
    fn false_positive_rate_reasonable() {
        let mut bf = BloomFilter::new(1000, 0.01);
        for i in 0u32..1000 {
            bf.insert(i);
        }

        // Test keys not in the filter (100000..101000)
        let mut false_positives = 0usize;
        let test_count = 1000usize;
        for i in 100_000u32..100_000 + test_count as u32 {
            if bf.contains(i) {
                false_positives += 1;
            }
        }

        let fp_rate = false_positives as f64 / test_count as f64;
        // With target 1%, we allow generous headroom to avoid flaky tests.
        assert!(
            fp_rate < 0.05,
            "False positive rate too high: {:.4} ({} out of {})",
            fp_rate,
            false_positives,
            test_count,
        );
    }

    #[test]
    fn empty_filter_no_false_positives() {
        let bf = BloomFilter::new(100, 0.01);
        for i in 0u32..100 {
            assert!(
                !bf.contains(i),
                "Unexpected hit in empty filter for key {}",
                i
            );
        }
    }

    #[test]
    fn single_item() {
        let mut bf = BloomFilter::new(10, 0.01);
        bf.insert(7);
        assert!(bf.contains(7));
    }

    #[test]
    fn different_fp_rates() {
        for &target_fp in &[0.001, 0.01, 0.05, 0.1] {
            let mut bf = BloomFilter::new(500, target_fp);
            for i in 0u32..500 {
                bf.insert(i);
            }
            // Verify no false negatives
            for i in 0u32..500 {
                assert!(bf.contains(i));
            }
        }
    }
}
