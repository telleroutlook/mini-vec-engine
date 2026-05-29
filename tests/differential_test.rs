//! Differential testing: verify all engine variants produce identical results.

use mini_vec_engine::engine::{self, data_gen, QueryParams};

fn run_differential(
    total_rows: usize,
    n_distinct_keys: u32,
    val_min: i64,
    val_max: i64,
    threshold: i64,
    seed: u64,
) {
    let config = data_gen::DataGenConfig {
        total_rows,
        n_distinct_keys,
        val_min,
        val_max,
        seed,
    };
    let batches = data_gen::generate_batches(&config);
    let params = QueryParams { threshold };

    let naive = engine::naive::execute(&batches, &params);
    let early = engine::vectorized::execute_early(&batches, &params);
    let late = engine::vectorized::execute_late(&batches, &params);
    let parallel = engine::parallel::execute(&batches, &params);
    let partitioned = engine::parallel::execute_partitioned(&batches, &params);
    let adaptive = engine::parallel::adaptive_execute_partitioned(&batches, &params);

    assert_eq!(naive, early, "Early materialization mismatch");
    assert_eq!(naive, late, "Late materialization mismatch");
    assert_eq!(naive, parallel, "Parallel mismatch");
    assert_eq!(naive, partitioned, "Partitioned mismatch");
    assert_eq!(naive, adaptive, "Adaptive partitioned mismatch");
}

#[test]
fn differential_basic() {
    run_differential(10_000, 10, 0, 100, 50, 42);
}

#[test]
fn differential_large() {
    run_differential(1_000_000, 100, 0, 1000, 500, 42);
}

#[test]
fn differential_high_selectivity() {
    run_differential(100_000, 50, 0, 100, 10, 123);
}

#[test]
fn differential_low_selectivity() {
    run_differential(100_000, 50, 0, 100, 95, 456);
}

#[test]
fn differential_no_rows_pass() {
    run_differential(10_000, 10, 0, 100, 1000, 789);
}

#[test]
fn differential_all_rows_pass() {
    run_differential(10_000, 10, -100, 100, -200, 999);
}

#[test]
fn differential_single_key() {
    run_differential(10_000, 1, 0, 100, 50, 111);
}

#[test]
fn differential_non_batch_aligned() {
    run_differential(12_345, 37, 0, 100, 50, 777);
}
