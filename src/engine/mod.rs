/// Vectorized query engine.
///
/// Implements the pipeline:
///   RecordBatch → VectorizedScan → PredicateEval → LateMaterialize → HashAggregate → TwoPhaseMerge
///
/// All components are columnar and operate on fixed-size batches.

/// Columnar batch of rows — the fundamental unit of data flow through the engine.
pub struct RecordBatch {
    /// Column names (for display/debug only).
    pub schema: Vec<String>,
    /// Columnar data stored as `Vec<u64>` byte-reinterpreted columns.
    /// Each column is a contiguous array of 64-bit values.
    pub columns: Vec<Vec<u64>>,
    /// Per-column validity bitmap (None = all valid).
    pub validity: Vec<Option<Vec<bool>>>,
    /// Number of rows in this batch.
    pub num_rows: usize,
}

/// Selection bitmap output from predicate evaluation.
/// Indicates which rows from a batch survive the filter.
pub type Selection = Vec<bool>;

/// Aggregation result: (key, sum).
pub struct AggResult {
    pub key: u64,
    pub sum: i64,
}
