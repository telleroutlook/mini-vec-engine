use std::time::Instant;

use mini_vec_engine::bench_infra::bench_env::EnvSnapshot;
use mini_vec_engine::bench_infra::timer;
use mini_vec_engine::engine::{self, data_gen, instrumented, QueryParams};

fn main() {
    let total_rows = 10_000_000;
    let threshold = 500i64;
    let n_distinct_keys = 100u32;

    println!("=== mini-vec-engine ===");
    println!(
        "Rows: {} | Threshold: {} | Distinct keys: {} | Batch size: {}\n",
        total_rows,
        threshold,
        n_distinct_keys,
        engine::BATCH_SIZE
    );

    // Calibrate TSC
    let ghz = timer::calibrate_ghz();
    println!("TSC calibrated: {:.3} GHz\n", ghz);

    // Generate data
    println!("Generating data...");
    let config = data_gen::DataGenConfig {
        total_rows,
        n_distinct_keys,
        val_min: 0,
        val_max: 1000,
        seed: 42,
    };
    let start = Instant::now();
    let batches = data_gen::generate_batches(&config);
    let gen_time = start.elapsed();
    println!(
        "Generated {} batches in {:.3}ms\n",
        batches.len(),
        gen_time.as_secs_f64() * 1000.0
    );

    let params = QueryParams { threshold };

    // --- Naive (instrumented) ---
    println!("--- Naive (row-by-row) ---");
    let before = EnvSnapshot::take();
    let start = Instant::now();
    let (naive_results, naive_lat) = instrumented::execute_naive(&batches, &params);
    let naive_time = start.elapsed();
    let after = EnvSnapshot::take();
    let clean = before.isolation_clean(&after);
    println!(
        "Time: {:.3}ms | Throughput: {:.0} rows/s | Clean env: {}",
        naive_time.as_secs_f64() * 1000.0,
        total_rows as f64 / naive_time.as_secs_f64(),
        clean
    );
    println!("Groups: {}", naive_results.len());
    naive_lat.print_reports("Naive", ghz);

    // --- Vectorized Early (instrumented) ---
    println!("\n--- Vectorized (early materialization) ---");
    let before = EnvSnapshot::take();
    let start = Instant::now();
    let (early_results, early_lat) = instrumented::execute_early(&batches, &params);
    let early_time = start.elapsed();
    let after = EnvSnapshot::take();
    let clean = before.isolation_clean(&after);
    println!(
        "Time: {:.3}ms | Throughput: {:.0} rows/s | Clean env: {}",
        early_time.as_secs_f64() * 1000.0,
        total_rows as f64 / early_time.as_secs_f64(),
        clean
    );
    assert_eq!(
        naive_results, early_results,
        "Early materialization results mismatch!"
    );
    println!("Results match naive");
    early_lat.print_reports("Early", ghz);

    // --- Vectorized Late (instrumented) ---
    println!("\n--- Vectorized (late materialization) ---");
    let before = EnvSnapshot::take();
    let start = Instant::now();
    let (late_results, late_lat) = instrumented::execute_late(&batches, &params);
    let late_time = start.elapsed();
    let after = EnvSnapshot::take();
    let clean = before.isolation_clean(&after);
    println!(
        "Time: {:.3}ms | Throughput: {:.0} rows/s | Clean env: {}",
        late_time.as_secs_f64() * 1000.0,
        total_rows as f64 / late_time.as_secs_f64(),
        clean
    );
    assert_eq!(
        naive_results, late_results,
        "Late materialization results mismatch!"
    );
    let late_speedup = naive_time.as_secs_f64() / late_time.as_secs_f64();
    println!("Results match naive | Late vs Naive: {:.2}x", late_speedup);
    late_lat.print_reports("Late", ghz);

    // --- Parallel ---
    println!("\n--- Parallel (rayon + two-phase merge) ---");
    let before = EnvSnapshot::take();
    let start = Instant::now();
    let parallel_results = engine::parallel::execute(&batches, &params);
    let parallel_time = start.elapsed();
    let after = EnvSnapshot::take();
    let clean = before.isolation_clean(&after);
    println!(
        "Time: {:.3}ms | Throughput: {:.0} rows/s | Clean env: {}",
        parallel_time.as_secs_f64() * 1000.0,
        total_rows as f64 / parallel_time.as_secs_f64(),
        clean
    );
    assert_eq!(
        naive_results, parallel_results,
        "Parallel results mismatch!"
    );
    let parallel_speedup = naive_time.as_secs_f64() / parallel_time.as_secs_f64();
    println!(
        "Results match naive | Parallel vs Naive: {:.2}x",
        parallel_speedup
    );

    // --- Summary Table ---
    println!("\n=== Summary ===");
    println!("| Engine | Time (ms) | Throughput (rows/s) | Speedup |");
    println!("|--------|-----------|---------------------|---------|");
    println!(
        "| Naive | {:.3} | {:.0} | 1.00x |",
        naive_time.as_secs_f64() * 1000.0,
        total_rows as f64 / naive_time.as_secs_f64()
    );
    println!(
        "| Vec-Early | {:.3} | {:.0} | {:.2}x |",
        early_time.as_secs_f64() * 1000.0,
        total_rows as f64 / early_time.as_secs_f64(),
        naive_time.as_secs_f64() / early_time.as_secs_f64()
    );
    println!(
        "| Vec-Late | {:.3} | {:.0} | {:.2}x |",
        late_time.as_secs_f64() * 1000.0,
        total_rows as f64 / late_time.as_secs_f64(),
        late_speedup
    );
    println!(
        "| Parallel | {:.3} | {:.0} | {:.2}x |",
        parallel_time.as_secs_f64() * 1000.0,
        total_rows as f64 / parallel_time.as_secs_f64(),
        parallel_speedup
    );

    // --- Per-stage Latency Percentiles ---
    println!("\n=== Per-stage Latency (ns) ===");
    println!("| Engine | Stage | p50 | p99 | p99.9 | p99.99 | max | n |");
    println!("|--------|-------|-----|-----|-------|--------|-----|---|");
    for row in naive_lat.to_markdown_rows("Naive", ghz) {
        println!("{row}");
    }
    for row in early_lat.to_markdown_rows("Early", ghz) {
        println!("{row}");
    }
    for row in late_lat.to_markdown_rows("Late", ghz) {
        println!("{row}");
    }

    println!("\nTop 10 groups:");
    for r in naive_results.iter().take(10) {
        println!("  key={} sum={}", r.key, r.sum);
    }
    if naive_results.len() > 10 {
        println!("  ... ({} more)", naive_results.len() - 10);
    }
}
