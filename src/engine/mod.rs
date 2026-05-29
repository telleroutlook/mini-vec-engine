//! Vectorized query engine.
//!
//! Implements the pipeline:
//!   RecordBatch → PredicateEval → LateMaterialize → HashAggregate → TwoPhaseMerge
//!
//! Target query: `SELECT key, SUM(val) FROM t WHERE val > C GROUP BY key`

use std::collections::HashMap;

use crate::bitmap::Bitmap;

pub const BATCH_SIZE: usize = 2048;
pub const BATCH_WORDS: usize = BATCH_SIZE / 64;

pub type SelectionBitmap = Bitmap<BATCH_WORDS>;

/// Columnar batch — the fundamental unit of data flow through the engine.
pub struct RecordBatch {
    pub keys: Vec<u32>,
    pub vals: Vec<i64>,
    pub num_rows: usize,
}

/// Aggregation result: (key, sum).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggResult {
    pub key: u32,
    pub sum: i64,
}

/// Query parameters for the hard-coded template:
/// `SELECT key, SUM(val) FROM t WHERE val > threshold GROUP BY key`
pub struct QueryParams {
    pub threshold: i64,
}

pub mod aggregate;
pub mod arena_agg;
pub mod data_gen;
pub mod expr;
pub mod instrumented;
pub mod naive;
pub mod parallel;
pub mod pruning;
pub mod simplify;
pub mod vectorized;

/// Convert a HashMap into sorted AggResults for deterministic comparison.
pub fn sorted_results(agg: HashMap<u32, i64>) -> Vec<AggResult> {
    let mut results: Vec<AggResult> = agg
        .into_iter()
        .map(|(key, sum)| AggResult { key, sum })
        .collect();
    results.sort_by_key(|r| r.key);
    results
}
