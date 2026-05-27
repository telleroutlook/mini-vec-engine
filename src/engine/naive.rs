use std::collections::HashMap;

use super::{sorted_results, AggResult, QueryParams, RecordBatch};

/// Naive row-by-row execution — gold-standard reference for differential testing.
pub fn execute(batches: &[RecordBatch], params: &QueryParams) -> Vec<AggResult> {
    let mut agg: HashMap<u32, i64> = HashMap::new();
    for batch in batches {
        for i in 0..batch.num_rows {
            if batch.vals[i] > params.threshold {
                *agg.entry(batch.keys[i]).or_insert(0) += batch.vals[i];
            }
        }
    }
    sorted_results(agg)
}
