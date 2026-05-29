use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use rayon::prelude::*;

use super::aggregate::{aggregate_selected, evaluate_predicate, merge_maps};
use super::{sorted_results, AggResult, QueryParams, RecordBatch};
use crate::bitmap::Bitmap;
use crate::spsc::SpscRing;

// ---------------------------------------------------------------------------
// Adaptive segment sizing (borrowed from BlazingGoldbach adaptive_segment_size)
// ---------------------------------------------------------------------------

/// Adjust partition granularity based on key density within a key range.
///
/// Borrowed from BlazingGoldbach's `adaptive_segment_size()`: when survival
/// density (here: qualifying rows per key range) is high, use smaller
/// partitions for better cache locality and work distribution; when sparse,
/// use larger partitions to amortize the fixed overhead of partition setup.
///
/// - ratio >= `high_threshold` (dense): return `min_size`
/// - ratio <= `low_threshold` (sparse): return `max_size`
/// - in between: linearly interpolate
pub fn adaptive_partition_size(
    base_size: usize,
    rows_in_range: usize,
    keys_in_range: usize,
    min_size: usize,
    max_size: usize,
) -> usize {
    if keys_in_range == 0 {
        return base_size;
    }
    let ratio = rows_in_range as f64 / keys_in_range as f64;

    const HIGH_THRESHOLD: f64 = 8.0; // many rows per key → dense
    const LOW_THRESHOLD: f64 = 1.0; // ~1 row per key → sparse

    let size = if ratio >= HIGH_THRESHOLD {
        min_size
    } else if ratio <= LOW_THRESHOLD {
        max_size
    } else {
        let t = (HIGH_THRESHOLD - ratio) / (HIGH_THRESHOLD - LOW_THRESHOLD);
        let interpolated = min_size as f64 + t * (max_size - min_size) as f64;
        interpolated.round() as usize
    };

    size.clamp(min_size, max_size)
}

/// Parallel hash aggregation with thread-local hash tables + two-phase merge.
///
/// Each rayon worker accumulates into a thread-local HashMap via `fold`,
/// then `merge_maps` combines them in a single pass.
pub fn execute(batches: &[RecordBatch], params: &QueryParams) -> Vec<AggResult> {
    let thread_local_maps: Vec<HashMap<u32, i64>> = batches
        .par_iter()
        .fold(HashMap::new, |mut local, batch| {
            let sel = evaluate_predicate(&batch.vals, params.threshold);
            aggregate_selected(&batch.keys, &batch.vals, &sel, batch.num_rows, &mut local);
            local
        })
        .collect();

    let global = merge_maps(&thread_local_maps);
    sorted_results(global)
}

/// Data chunk passed through the SPSC ring from producer to consumer.
/// Contains the keys, values, and selection bitmap for one batch.
struct AggChunk {
    keys: Vec<u32>,
    vals: Vec<i64>,
    selection: Bitmap<{ super::BATCH_WORDS }>,
    num_rows: usize,
}

/// Pipelined aggregation using SpscRing (ported from hft-latency-lab).
///
/// Architecture:
///   Producer thread — iterates batches, evaluates predicates, pushes AggChunks
///   Consumer thread — pops AggChunks, accumulates into a hash table
///
/// This decouples predicate evaluation from aggregation, allowing each stage
/// to proceed at its own pace without mutex contention (lock-free SPSC).
pub fn execute_pipelined(batches: &[RecordBatch], params: &QueryParams) -> Vec<AggResult> {
    let ring = Arc::new(SpscRing::<AggChunk, 64>::new());

    // Clone the batch data so the producer thread can own it.
    // In a real system these would be reference-counted columnar buffers.
    let batch_data: Vec<(Vec<u32>, Vec<i64>, usize)> = batches
        .iter()
        .map(|b| (b.keys.clone(), b.vals.clone(), b.num_rows))
        .collect();

    let threshold = params.threshold;

    // Producer: evaluate predicates and push chunks
    let producer_ring = Arc::clone(&ring);
    let producer = thread::spawn(move || {
        for (keys, vals, num_rows) in batch_data {
            let selection = evaluate_predicate(&vals, threshold);
            let chunk = AggChunk {
                keys,
                vals,
                selection,
                num_rows,
            };
            // Spin until there is space, then push.
            // In SPSC mode only the consumer pops, so once we observe
            // len < capacity the push is guaranteed to succeed — the only
            // concurrent mutation is the consumer reducing len further.
            while producer_ring.len() >= producer_ring.capacity() {
                thread::yield_now();
            }
            // push() takes ownership; with capacity guaranteed it always returns true.
            let _ok = producer_ring.push(chunk);
            debug_assert!(_ok);
        }
        // Push a sentinel (empty chunk) to signal end-of-stream
        while producer_ring.len() >= producer_ring.capacity() {
            thread::yield_now();
        }
        let sentinel = AggChunk {
            keys: Vec::new(),
            vals: Vec::new(),
            selection: Bitmap::zeroed(),
            num_rows: 0,
        };
        let _ok = producer_ring.push(sentinel);
        debug_assert!(_ok);
    });

    // Consumer: aggregate chunks from the ring
    let consumer_ring = Arc::clone(&ring);
    let consumer = thread::spawn(move || {
        let mut agg: HashMap<u32, i64> = HashMap::new();
        loop {
            let chunk = loop {
                if let Some(c) = consumer_ring.pop() {
                    break c;
                }
                thread::yield_now();
            };
            // Sentinel check: empty keys means end-of-stream
            if chunk.keys.is_empty() {
                break;
            }
            aggregate_selected(
                &chunk.keys,
                &chunk.vals,
                &chunk.selection,
                chunk.num_rows,
                &mut agg,
            );
        }
        agg
    });

    producer.join().expect("producer thread panicked");
    let agg = consumer.join().expect("consumer thread panicked");
    sorted_results(agg)
}

// ---------------------------------------------------------------------------
// Key-range partitioned parallel aggregation (inspired by Golomb-Vanguard
// stub-based work decomposition).  Instead of partitioning by batch index,
// we partition by key range so each worker's local HashMap has disjoint key
// sets, eliminating the merge_maps bottleneck entirely.
// ---------------------------------------------------------------------------

/// A key-range partition covering `[lo, hi)` with pre-sliced row indices.
#[allow(dead_code)]
struct KeyPartition {
    lo: u32,
    hi: u32,
    /// Flat list of (batch_index, row_index_within_batch) for rows whose key
    /// falls in [lo, hi).  Built once during partitioning, then each worker
    /// iterates its own list with no contention.
    rows: Vec<(usize, usize)>,
}

/// Partition work by key range using stub generation (inspired by
/// Golomb-Vanguard's `generate_stubs`).
///
/// In Golomb-Vanguard, `generate_stubs` enumerates all valid placements for
/// the first few marks, creating independent work units for parallel DFS.
/// Here we adapt the same idea: instead of exploring mark placements, we
/// partition the key space into contiguous ranges (stubs), where each stub
/// becomes an independent aggregation work unit.
///
/// Each partition gets its own local HashMap with disjoint keys, so the
/// final merge is a trivial concatenation instead of a hash-merge —
/// eliminating the `merge_maps` bottleneck from the original `execute()`.
///
/// Steps:
///   1. Scan all batches to find min/max key and count distinct keys.
///   2. Generate N_PARTITIONS key-range stubs (contiguous [lo, hi) ranges).
///   3. Assign each qualifying row to its partition.
///   4. Rayon parallel iterator over partitions — each builds a local HashMap.
///   5. Collect results (no merge needed; key ranges are disjoint).
pub fn execute_partitioned(batches: &[RecordBatch], params: &QueryParams) -> Vec<AggResult> {
    // ---- Phase 1: discover key range ----
    let mut key_min = u32::MAX;
    let mut key_max = 0u32;
    let mut total_qualifying = 0usize;

    // Pre-compute selection bitmaps and count qualifying rows per batch
    let selections: Vec<Bitmap<{ super::BATCH_WORDS }>> = batches
        .iter()
        .map(|b| evaluate_predicate(&b.vals, params.threshold))
        .collect();

    for (bi, batch) in batches.iter().enumerate() {
        for row_idx in selections[bi].iter_set_bits() {
            if row_idx >= batch.num_rows {
                break;
            }
            let k = unsafe { *batch.keys.get_unchecked(row_idx) };
            key_min = key_min.min(k);
            key_max = key_max.max(k);
            total_qualifying += 1;
        }
    }

    // Edge case: no qualifying rows
    if total_qualifying == 0 {
        return Vec::new();
    }

    // ---- Phase 2: generate key-range stubs (partitions) ----
    // Like Golomb-Vanguard's generate_stubs, we partition the key space into
    // contiguous ranges. Each range is an independent work unit for parallel
    // processing. The number of partitions is chosen to be proportional to
    // rayon's thread pool size but also respects key density.
    let n_threads = rayon::current_num_threads();
    let n_distinct_keys = (key_max - key_min + 1) as usize;
    let n_partitions = std::cmp::min(n_threads * 4, n_distinct_keys).max(1);
    let keys_per_partition = n_distinct_keys / n_partitions;

    let mut partitions: Vec<KeyPartition> = Vec::with_capacity(n_partitions);
    for i in 0..n_partitions {
        let lo = key_min + (i as u32) * (keys_per_partition as u32);
        let hi = if i == n_partitions - 1 {
            key_max + 1 // last partition takes the remainder
        } else {
            key_min + ((i + 1) as u32) * (keys_per_partition as u32)
        };
        partitions.push(KeyPartition {
            lo,
            hi,
            rows: Vec::new(),
        });
    }

    // ---- Phase 3: assign qualifying rows to partitions ----
    for (bi, batch) in batches.iter().enumerate() {
        for row_idx in selections[bi].iter_set_bits() {
            if row_idx >= batch.num_rows {
                break;
            }
            let k = unsafe { *batch.keys.get_unchecked(row_idx) };

            // Assign to the partition containing key k
            let part_idx = if keys_per_partition > 0 {
                let offset = (k as usize).saturating_sub(key_min as usize);
                std::cmp::min(offset / keys_per_partition, n_partitions - 1)
            } else {
                0
            };
            // Store (batch_index, row_index, value) — we repack to avoid
            // re-reading the value during aggregation. Actually, just store
            // (batch_index, row_index); the worker will read key + val.
            // But we already have k and v here — to avoid re-reading, store
            // them directly in a flat row list per partition.
            // For simplicity and cache-friendliness, we store (key, value)
            // pairs directly so each partition owns its data.
            partitions[part_idx].rows.push((bi, row_idx));
        }
    }

    // ---- Phase 4: parallel aggregation per partition ----
    // Each partition's HashMap has disjoint keys, so no merge is needed.
    let results: Vec<HashMap<u32, i64>> = partitions
        .into_par_iter()
        .map(|part| {
            let mut local = HashMap::new();
            // Pre-allocate for expected size to reduce rehashing
            let est_capacity = (part.rows.len() * 2).max(16);
            local.reserve(est_capacity);

            for (bi, row_idx) in part.rows {
                let batch = &batches[bi];
                let k = unsafe { *batch.keys.get_unchecked(row_idx) };
                let v = unsafe { *batch.vals.get_unchecked(row_idx) };
                *local.entry(k).or_insert(0) += v;
            }
            local
        })
        .collect();

    // ---- Phase 5: combine results (simple chain, no merge_maps needed) ----
    // Since partitions have disjoint key ranges, we just chain them together.
    let mut global = HashMap::new();
    let total_entries: usize = results.iter().map(|m| m.len()).sum();
    global.reserve(total_entries);
    for local in results {
        global.extend(local);
    }

    sorted_results(global)
}

/// Profiled wrapper: time each phase of `execute_partitioned` and return a QueryProfile.
#[cfg(feature = "profile")]
pub fn execute_partitioned_profiled(
    batches: &[RecordBatch],
    params: &QueryParams,
) -> (Vec<AggResult>, crate::timing::QueryProfile) {
    use crate::timing::{OperationTimer, QueryProfile};
    let total = OperationTimer::new();

    // ---- Phase 1: discover key range + predicate ----
    let pred_timer = OperationTimer::new();
    let mut key_min = u32::MAX;
    let mut key_max = 0u32;
    let mut total_qualifying = 0usize;

    let selections: Vec<Bitmap<{ super::BATCH_WORDS }>> = batches
        .iter()
        .map(|b| evaluate_predicate(&b.vals, params.threshold))
        .collect();

    for (bi, batch) in batches.iter().enumerate() {
        for row_idx in selections[bi].iter_set_bits() {
            if row_idx >= batch.num_rows {
                break;
            }
            let k = unsafe { *batch.keys.get_unchecked(row_idx) };
            key_min = key_min.min(k);
            key_max = key_max.max(k);
            total_qualifying += 1;
        }
    }
    let predicate_cycles = pred_timer.elapsed();

    if total_qualifying == 0 {
        let mut profile = QueryProfile::new("execute_partitioned");
        profile.predicate_cycles = predicate_cycles;
        profile.total_cycles = total.elapsed();
        return (Vec::new(), profile);
    }

    // ---- Phase 2: generate key-range stubs ----
    let part_timer = OperationTimer::new();
    let n_threads = rayon::current_num_threads();
    let n_distinct_keys = (key_max - key_min + 1) as usize;
    let n_partitions = std::cmp::min(n_threads * 4, n_distinct_keys).max(1);
    let keys_per_partition = n_distinct_keys / n_partitions;

    let mut partitions: Vec<KeyPartition> = Vec::with_capacity(n_partitions);
    for i in 0..n_partitions {
        let lo = key_min + (i as u32) * (keys_per_partition as u32);
        let hi = if i == n_partitions - 1 {
            key_max + 1
        } else {
            key_min + ((i + 1) as u32) * (keys_per_partition as u32)
        };
        partitions.push(KeyPartition {
            lo,
            hi,
            rows: Vec::new(),
        });
    }

    for (bi, batch) in batches.iter().enumerate() {
        for row_idx in selections[bi].iter_set_bits() {
            if row_idx >= batch.num_rows {
                break;
            }
            let k = unsafe { *batch.keys.get_unchecked(row_idx) };
            let part_idx = if keys_per_partition > 0 {
                let offset = (k as usize).saturating_sub(key_min as usize);
                std::cmp::min(offset / keys_per_partition, n_partitions - 1)
            } else {
                0
            };
            partitions[part_idx].rows.push((bi, row_idx));
        }
    }
    let partition_cycles = part_timer.elapsed();

    // ---- Phase 3: parallel aggregation ----
    let agg_timer = OperationTimer::new();
    let results: Vec<HashMap<u32, i64>> = partitions
        .into_par_iter()
        .map(|part| {
            let mut local = HashMap::new();
            local.reserve((part.rows.len() * 2).max(16));
            for (bi, row_idx) in part.rows {
                let batch = &batches[bi];
                let k = unsafe { *batch.keys.get_unchecked(row_idx) };
                let v = unsafe { *batch.vals.get_unchecked(row_idx) };
                *local.entry(k).or_insert(0) += v;
            }
            local
        })
        .collect();

    let mut global = HashMap::new();
    let total_entries: usize = results.iter().map(|m| m.len()).sum();
    global.reserve(total_entries);
    for local in results {
        global.extend(local);
    }
    let aggregate_cycles = agg_timer.elapsed();

    let mut profile = QueryProfile::new("execute_partitioned");
    profile.predicate_cycles = predicate_cycles;
    profile.partition_cycles = partition_cycles;
    profile.aggregate_cycles = aggregate_cycles;
    profile.total_cycles = total.elapsed();

    (sorted_results(global), profile)
}

// ---------------------------------------------------------------------------
// Adaptive partitioned execution
// ---------------------------------------------------------------------------

/// Histogram bucket for counting qualifying rows per key-range sub-interval.
struct DensityBucket {
    lo: u32,
    hi: u32,
    row_count: usize,
}

/// Key-range partitioned aggregation with adaptive segment sizing.
///
/// Extends `execute_partitioned()` with adaptive partition granularity
/// borrowed from BlazingGoldbach's `adaptive_segment_size()` logic:
///
/// 1. First pass: discover key range and count qualifying rows per
///    sub-interval (histogram of key density).
/// 2. Compute adaptive partition sizes: dense ranges (many rows per key)
///    get smaller partitions for better parallelism and cache locality;
///    sparse ranges get larger partitions to amortize overhead.
/// 3. Merge adjacent sub-intervals into final partitions of approximately
///    the target size.
/// 4. Assign rows and aggregate in parallel (disjoint keys, no merge needed).
pub fn adaptive_execute_partitioned(
    batches: &[RecordBatch],
    params: &QueryParams,
) -> Vec<AggResult> {
    // ---- Phase 1: pre-compute selection bitmaps ----
    let selections: Vec<Bitmap<{ super::BATCH_WORDS }>> = batches
        .iter()
        .map(|b| evaluate_predicate(&b.vals, params.threshold))
        .collect();

    // ---- Phase 2: discover key range + build density histogram ----
    let mut key_min = u32::MAX;
    let mut key_max = 0u32;
    let mut total_qualifying = 0usize;

    for (bi, batch) in batches.iter().enumerate() {
        for row_idx in selections[bi].iter_set_bits() {
            if row_idx >= batch.num_rows {
                break;
            }
            let k = unsafe { *batch.keys.get_unchecked(row_idx) };
            key_min = key_min.min(k);
            key_max = key_max.max(k);
            total_qualifying += 1;
        }
    }

    if total_qualifying == 0 {
        return Vec::new();
    }

    let n_distinct_keys = (key_max - key_min + 1) as usize;
    let n_threads = rayon::current_num_threads();

    // Subdivide the key range into fine-grained buckets for density estimation.
    // Use ~4x the number of expected partitions to get a granular histogram.
    let n_buckets = std::cmp::min(n_threads * 16, n_distinct_keys).max(1);
    let keys_per_bucket = n_distinct_keys / n_buckets;

    let mut buckets: Vec<DensityBucket> = (0..n_buckets)
        .map(|i| {
            let lo = key_min + (i as u32) * (keys_per_bucket as u32);
            let hi = if i == n_buckets - 1 {
                key_max + 1
            } else {
                key_min + ((i + 1) as u32) * (keys_per_bucket as u32)
            };
            DensityBucket {
                lo,
                hi,
                row_count: 0,
            }
        })
        .collect();

    // Count qualifying rows per bucket.
    for (bi, batch) in batches.iter().enumerate() {
        for row_idx in selections[bi].iter_set_bits() {
            if row_idx >= batch.num_rows {
                break;
            }
            let k = unsafe { *batch.keys.get_unchecked(row_idx) };
            let offset = (k as usize).saturating_sub(key_min as usize);
            let bucket_idx = if keys_per_bucket > 0 {
                std::cmp::min(offset / keys_per_bucket, n_buckets - 1)
            } else {
                0
            };
            buckets[bucket_idx].row_count += 1;
        }
    }

    // ---- Phase 3: adaptive partition sizing ----
    // Determine a target partition size (in rows) using the adaptive formula.
    // Then merge adjacent buckets into partitions of approximately that size.
    let min_part_keys = std::cmp::max(n_distinct_keys / (n_threads * 8), 1);
    let max_part_keys = std::cmp::max(n_distinct_keys / n_threads, 1);

    let mut partitions: Vec<KeyPartition> = Vec::new();
    let mut current_lo = key_min;

    for bucket in &buckets {
        let bucket_keys = (bucket.hi - bucket.lo) as usize;
        let adaptive_size = adaptive_partition_size(
            max_part_keys,
            bucket.row_count,
            bucket_keys.max(1),
            min_part_keys,
            max_part_keys,
        );

        let key_span = (bucket.hi - current_lo) as usize;
        if key_span >= adaptive_size {
            partitions.push(KeyPartition {
                lo: current_lo,
                hi: bucket.lo,
                rows: Vec::new(),
            });
            current_lo = bucket.lo;
        }
    }

    // Ensure the last range is captured.
    if current_lo <= key_max {
        partitions.push(KeyPartition {
            lo: current_lo,
            hi: key_max + 1,
            rows: Vec::new(),
        });
    }

    // ---- Phase 4: assign qualifying rows to adaptive partitions ----
    let n_partitions = partitions.len();
    let partition_borders: Vec<u32> = partitions.iter().map(|p| p.hi).collect();

    for (bi, batch) in batches.iter().enumerate() {
        for row_idx in selections[bi].iter_set_bits() {
            if row_idx >= batch.num_rows {
                break;
            }
            let k = unsafe { *batch.keys.get_unchecked(row_idx) };
            let part_idx = match partition_borders.binary_search(&k) {
                Ok(i) => std::cmp::min(i + 1, n_partitions - 1),
                Err(i) => std::cmp::min(i, n_partitions - 1),
            };
            partitions[part_idx].rows.push((bi, row_idx));
        }
    }

    // ---- Phase 5: parallel aggregation per partition ----
    let results: Vec<HashMap<u32, i64>> = partitions
        .into_par_iter()
        .map(|part| {
            let mut local = HashMap::new();
            local.reserve((part.rows.len() * 2).max(16));
            for (bi, row_idx) in part.rows {
                let batch = &batches[bi];
                let k = unsafe { *batch.keys.get_unchecked(row_idx) };
                let v = unsafe { *batch.vals.get_unchecked(row_idx) };
                *local.entry(k).or_insert(0) += v;
            }
            local
        })
        .collect();

    // ---- Phase 6: combine (disjoint keys -> simple extend) ----
    let mut global = HashMap::new();
    let total_entries: usize = results.iter().map(|m| m.len()).sum();
    global.reserve(total_entries);
    for local in results {
        global.extend(local);
    }

    sorted_results(global)
}
