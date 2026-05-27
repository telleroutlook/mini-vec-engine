use std::collections::HashMap;

use rayon::prelude::*;

use super::aggregate::{aggregate_selected, evaluate_predicate, merge_maps};
use super::{sorted_results, AggResult, QueryParams, RecordBatch};

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
