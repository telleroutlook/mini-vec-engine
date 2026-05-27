use std::collections::HashMap;

use crate::bench_infra::histogram::LatencyReport;
use crate::bench_infra::latency_buf::LatencyBuffer;
use crate::bench_infra::timer;

use super::aggregate::{aggregate_selected, evaluate_predicate};
use super::{sorted_results, AggResult, QueryParams, RecordBatch};

/// Per-stage latency samples collected during engine execution.
pub struct StageLatency {
    pub predicate: LatencyBuffer,
    pub aggregate: LatencyBuffer,
    pub merge: LatencyBuffer,
}

impl StageLatency {
    pub fn new(n_batches: usize) -> Self {
        Self {
            predicate: LatencyBuffer::with_capacity(n_batches),
            aggregate: LatencyBuffer::with_capacity(n_batches),
            merge: LatencyBuffer::with_capacity(16), // merge is infrequent
        }
    }

    pub fn print_reports(&self, label: &str, ghz: f64) {
        if !self.predicate.is_empty() {
            let report = LatencyReport::from_cycles(self.predicate.finish(), ghz);
            report.print(&format!("{label} | predicate"));
        }
        if !self.aggregate.is_empty() {
            let report = LatencyReport::from_cycles(self.aggregate.finish(), ghz);
            report.print(&format!("{label} | aggregate"));
        }
        if !self.merge.is_empty() {
            let report = LatencyReport::from_cycles(self.merge.finish(), ghz);
            report.print(&format!("{label} | merge"));
        }
    }

    pub fn to_markdown_rows(&self, label: &str, ghz: f64) -> Vec<String> {
        let mut rows = Vec::new();
        if !self.predicate.is_empty() {
            let r = LatencyReport::from_cycles(self.predicate.finish(), ghz);
            rows.push(format!(
                "| {label} | predicate | {} |",
                r.to_markdown("p50/p99/p999.9/max/n")
            ));
        }
        if !self.aggregate.is_empty() {
            let r = LatencyReport::from_cycles(self.aggregate.finish(), ghz);
            rows.push(format!(
                "| {label} | aggregate | {} |",
                r.to_markdown("p50/p99/p999.9/max/n")
            ));
        }
        if !self.merge.is_empty() {
            let r = LatencyReport::from_cycles(self.merge.finish(), ghz);
            rows.push(format!(
                "| {label} | merge | {} |",
                r.to_markdown("p50/p99/p999.9/max/n")
            ));
        }
        rows
    }
}

/// Instrumented vectorized execution with late materialization.
/// Returns results and per-batch latency samples for each stage.
pub fn execute_late(
    batches: &[RecordBatch],
    params: &QueryParams,
) -> (Vec<AggResult>, StageLatency) {
    let mut agg: HashMap<u32, i64> = HashMap::new();
    let mut lat = StageLatency::new(batches.len());

    for batch in batches {
        let t0 = timer::rdtsc_serialized();
        let sel = evaluate_predicate(&batch.vals, params.threshold);
        let t1 = timer::rdtsc_serialized();
        lat.predicate.record(t1 - t0);

        let t2 = timer::rdtsc_serialized();
        aggregate_selected(&batch.keys, &batch.vals, &sel, batch.num_rows, &mut agg);
        let t3 = timer::rdtsc_serialized();
        lat.aggregate.record(t3 - t2);
    }

    (sorted_results(agg), lat)
}

/// Instrumented vectorized execution with early materialization.
pub fn execute_early(
    batches: &[RecordBatch],
    params: &QueryParams,
) -> (Vec<AggResult>, StageLatency) {
    let mut agg: HashMap<u32, i64> = HashMap::new();
    let mut lat = StageLatency::new(batches.len());

    for batch in batches {
        // Early materialization: filter into temp vectors, then aggregate
        let t0 = timer::rdtsc_serialized();
        let mut filtered_keys = Vec::with_capacity(batch.num_rows);
        let mut filtered_vals = Vec::with_capacity(batch.num_rows);
        for i in 0..batch.num_rows {
            if batch.vals[i] > params.threshold {
                filtered_keys.push(batch.keys[i]);
                filtered_vals.push(batch.vals[i]);
            }
        }
        let t1 = timer::rdtsc_serialized();
        lat.predicate.record(t1 - t0);

        let t2 = timer::rdtsc_serialized();
        for i in 0..filtered_keys.len() {
            *agg.entry(filtered_keys[i]).or_insert(0) += filtered_vals[i];
        }
        let t3 = timer::rdtsc_serialized();
        lat.aggregate.record(t3 - t2);
    }

    (sorted_results(agg), lat)
}

/// Instrumented naive row-by-row execution.
pub fn execute_naive(
    batches: &[RecordBatch],
    params: &QueryParams,
) -> (Vec<AggResult>, StageLatency) {
    let mut agg: HashMap<u32, i64> = HashMap::new();
    let mut lat = StageLatency::new(batches.len());

    for batch in batches {
        let t0 = timer::rdtsc_serialized();
        for i in 0..batch.num_rows {
            if batch.vals[i] > params.threshold {
                *agg.entry(batch.keys[i]).or_insert(0) += batch.vals[i];
            }
        }
        let t1 = timer::rdtsc_serialized();
        lat.predicate.record(t1 - t0);
    }

    (sorted_results(agg), lat)
}
