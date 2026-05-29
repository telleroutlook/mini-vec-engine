//! Arena-backed open-addressing hash table for hash aggregation.
//!
//! Inspired by `OrderArena` from hft-latency-lab: pre-allocate all storage
//! upfront so the hot aggregation path performs zero heap allocation.
//!
//! The `ArenaHashTable` uses open-addressing with linear probing, which is
//! more cache-friendly than chaining and avoids pointer chasing entirely.
//! Keys, values, and occupancy metadata live in contiguous pre-allocated arrays.

/// Sentinel value for unoccupied slots in the key array.
const EMPTY_KEY: u32 = u32::MAX;

/// Arena-backed hash table for `key -> accumulated_sum` aggregation.
///
/// All memory is allocated at construction time. `insert_or_add` never
/// allocates — it probes the fixed-size table using linear probing.
pub struct ArenaHashTable {
    keys: Vec<u32>,
    vals: Vec<i64>,
    occupied: Vec<u8>,
    len: usize,
    #[allow(dead_code)]
    capacity: usize,
    /// Mask for fast modulo via bitwise AND (capacity must be power-of-2).
    mask: usize,
}

impl ArenaHashTable {
    /// Create a new arena hash table pre-allocated for `capacity` entries.
    ///
    /// The actual table size is rounded up to the next power of two and then
    /// multiplied by the load factor divisor to keep occupancy below ~75%.
    /// Panics if `capacity` is zero.
    pub fn new(requested_capacity: usize) -> Self {
        assert!(requested_capacity > 0, "capacity must be > 0");

        // Size the internal table to keep load factor <= 0.75.
        // Round up to next power of two, then ensure we have at least
        // 4/3 * requested_capacity slots.
        let min_slots = (requested_capacity * 4).div_ceil(3);
        let capacity = min_slots.next_power_of_two();
        let mask = capacity - 1;

        Self {
            keys: vec![EMPTY_KEY; capacity],
            vals: vec![0i64; capacity],
            occupied: vec![0u8; capacity],
            len: 0,
            capacity,
            mask,
        }
    }

    /// Insert `key` with initial `value`, or add `value` to existing entry.
    ///
    /// Uses FxHash-style multiply-shift hash for fast distribution,
    /// then linear probing for collision resolution.
    #[inline]
    pub fn insert_or_add(&mut self, key: u32, value: i64) {
        debug_assert!(
            key != EMPTY_KEY,
            "EMPTY_KEY cannot be used as aggregation key"
        );

        let mut idx = Self::hash_key(key) & self.mask;
        loop {
            if self.occupied[idx] == 0 {
                // Empty slot — insert here.
                self.keys[idx] = key;
                self.vals[idx] = value;
                self.occupied[idx] = 1;
                self.len += 1;
                return;
            }
            if self.keys[idx] == key {
                // Existing key — accumulate.
                self.vals[idx] += value;
                return;
            }
            // Collision — linear probe.
            idx = (idx + 1) & self.mask;
        }
    }

    /// Number of distinct keys currently stored.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Iterate over all `(key, sum)` entries.
    pub fn iter(&self) -> impl Iterator<Item = (u32, i64)> + '_ {
        self.occupied
            .iter()
            .enumerate()
            .filter_map(move |(i, &occ)| {
                if occ != 0 {
                    Some((self.keys[i], self.vals[i]))
                } else {
                    None
                }
            })
    }

    /// FxHash-style multiply-shift: fast and sufficient for aggregation keys.
    #[inline]
    fn hash_key(key: u32) -> usize {
        // FxHash constant for 32-bit: multiply then shift-right.
        const FX_MULTIPLY: u64 = 0x517cc1b727220a95;
        (key as u64).wrapping_mul(FX_MULTIPLY) as usize
    }
}

/// Aggregate qualifying rows into an `ArenaHashTable`.
///
/// This is the arena-backed equivalent of `aggregate_selected()` from
/// the `aggregate` module — same semantics, zero heap allocation on
/// the hot path.
pub fn aggregate_selected_arena(
    keys: &[u32],
    vals: &[i64],
    selection: &crate::bitmap::Bitmap<{ super::BATCH_WORDS }>,
    num_rows: usize,
    agg: &mut ArenaHashTable,
) {
    for i in selection.iter_set_bits() {
        if i >= num_rows {
            break;
        }
        let k = unsafe { *keys.get_unchecked(i) };
        let v = unsafe { *vals.get_unchecked(i) };
        agg.insert_or_add(k, v);
    }
}

/// Profiled wrapper: time `aggregate_selected_arena` and return elapsed cycles.
#[cfg(feature = "profile")]
pub fn aggregate_selected_arena_timed(
    keys: &[u32],
    vals: &[i64],
    selection: &crate::bitmap::Bitmap<{ super::BATCH_WORDS }>,
    num_rows: usize,
    agg: &mut ArenaHashTable,
) -> u64 {
    let t = crate::timing::OperationTimer::new();
    aggregate_selected_arena(keys, vals, selection, num_rows, agg);
    t.elapsed()
}

/// Merge multiple arena hash tables into a single `HashMap` for final output.
///
/// Since the arena tables don't implement `IntoIterator` for `HashMap`
/// directly, this collects and merges. Used at the end of parallel execution
/// to combine thread-local arena results.
pub fn merge_arena_tables(tables: &[ArenaHashTable]) -> std::collections::HashMap<u32, i64> {
    let mut global = std::collections::HashMap::new();
    for table in tables {
        for (k, v) in table.iter() {
            *global.entry(k).or_insert(0i64) += v;
        }
    }
    global
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_table_is_empty() {
        let t = ArenaHashTable::new(16);
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn insert_single_key() {
        let mut t = ArenaHashTable::new(16);
        t.insert_or_add(42, 100);
        assert_eq!(t.len(), 1);
        let entries: Vec<_> = t.iter().collect();
        assert_eq!(entries, vec![(42, 100)]);
    }

    #[test]
    fn accumulate_existing_key() {
        let mut t = ArenaHashTable::new(16);
        t.insert_or_add(10, 5);
        t.insert_or_add(10, 7);
        t.insert_or_add(10, 3);
        assert_eq!(t.len(), 1);
        assert_eq!(t.iter().next(), Some((10, 15)));
    }

    #[test]
    fn multiple_distinct_keys() {
        let mut t = ArenaHashTable::new(64);
        for k in 0..50u32 {
            t.insert_or_add(k, k as i64 * 10);
        }
        assert_eq!(t.len(), 50);

        let mut results: Vec<_> = t.iter().collect();
        results.sort_by_key(|&(k, _)| k);
        for (i, (k, v)) in results.iter().enumerate() {
            assert_eq!(*k, i as u32);
            assert_eq!(*v, i as i64 * 10);
        }
    }

    #[test]
    fn linear_probing_collision() {
        // Use a tiny table so collisions are guaranteed.
        let mut t = ArenaHashTable::new(4);
        // Insert keys that may hash to the same slot.
        t.insert_or_add(0, 1);
        t.insert_or_add(1, 2);
        t.insert_or_add(2, 3);
        assert_eq!(t.len(), 3);
        assert_eq!(t.iter().find(|&(k, _)| k == 0), Some((0, 1)));
        assert_eq!(t.iter().find(|&(k, _)| k == 1), Some((1, 2)));
        assert_eq!(t.iter().find(|&(k, _)| k == 2), Some((2, 3)));
    }

    #[test]
    fn differential_vs_hashmap() {
        use crate::bitmap::Bitmap;
        use std::collections::HashMap;

        // Build synthetic data
        let keys: Vec<u32> = (0..512).map(|i| i % 100).collect();
        let vals: Vec<i64> = (0..512).map(|i| (i * 7) as i64).collect();
        let mut sel = Bitmap::<{ super::super::BATCH_WORDS }>::zeroed();
        for (i, v) in vals.iter().enumerate().take(512) {
            if *v > 100 {
                sel.set(i);
            }
        }

        // HashMap reference
        let mut ref_map: HashMap<u32, i64> = HashMap::new();
        crate::engine::aggregate::aggregate_selected(&keys, &vals, &sel, 512, &mut ref_map);

        // Arena version
        let mut arena = ArenaHashTable::new(128);
        aggregate_selected_arena(&keys, &vals, &sel, 512, &mut arena);

        // Compare results
        let mut arena_results: Vec<_> = arena.iter().collect();
        arena_results.sort_by_key(|&(k, _)| k);
        let mut ref_results: Vec<_> = ref_map.into_iter().collect();
        ref_results.sort_by_key(|(k, _)| *k);
        assert_eq!(arena_results, ref_results);
    }

    #[test]
    fn merge_arena_tables_combines() {
        let mut t1 = ArenaHashTable::new(16);
        let mut t2 = ArenaHashTable::new(16);
        t1.insert_or_add(1, 10);
        t1.insert_or_add(2, 20);
        t2.insert_or_add(2, 30);
        t2.insert_or_add(3, 40);

        let merged = merge_arena_tables(&[t1, t2]);
        assert_eq!(merged.get(&1), Some(&10));
        assert_eq!(merged.get(&2), Some(&50));
        assert_eq!(merged.get(&3), Some(&40));
    }
}
