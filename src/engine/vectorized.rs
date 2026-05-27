use std::collections::HashMap;

use super::aggregate::{aggregate_selected, evaluate_predicate};
use super::{sorted_results, AggResult, QueryParams, RecordBatch};

/// Vectorized execution with early materialization.
///
/// Reads both columns for every row and filters into temporary vectors
/// before aggregating. This models the "materialize everything first" approach.
pub fn execute_early(batches: &[RecordBatch], params: &QueryParams) -> Vec<AggResult> {
    let mut agg: HashMap<u32, i64> = HashMap::new();
    for batch in batches {
        let mut filtered_keys = Vec::with_capacity(batch.num_rows);
        let mut filtered_vals = Vec::with_capacity(batch.num_rows);

        for i in 0..batch.num_rows {
            if batch.vals[i] > params.threshold {
                filtered_keys.push(batch.keys[i]);
                filtered_vals.push(batch.vals[i]);
            }
        }

        for i in 0..filtered_keys.len() {
            *agg.entry(filtered_keys[i]).or_insert(0) += filtered_vals[i];
        }
    }
    sorted_results(agg)
}

/// Vectorized execution with late materialization.
///
/// Evaluates predicate on val column to produce a selection bitmap,
/// then only reads key for surviving rows — skipping key column access
/// for filtered-out rows.
pub fn execute_late(batches: &[RecordBatch], params: &QueryParams) -> Vec<AggResult> {
    let mut agg: HashMap<u32, i64> = HashMap::new();
    for batch in batches {
        let sel = evaluate_predicate(&batch.vals, params.threshold);
        aggregate_selected(&batch.keys, &batch.vals, &sel, batch.num_rows, &mut agg);
    }
    sorted_results(agg)
}
