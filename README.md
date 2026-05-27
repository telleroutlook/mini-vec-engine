# mini-vec-engine

DataFusion-flavored toy vectorized query engine.

## What is this?

A from-scratch vectorized query engine that processes `SELECT key, SUM(val) FROM t WHERE val > C GROUP BY key`
using columnar memory layout, selection bitmaps, and parallel hash aggregation.

Built as a learning vehicle for database kernel internals, with rigorous benchmarking
inherited from [hft-latency-lab](https://github.com/user/hft-latency-lab).

## Architecture

```
Input Columns ──→ Vectorized Scan ──→ Predicate Eval ──→ Selection Bitmap
                                                              │
                   ┌──────────────────────────────────────────┘
                   ▼
            Late Materialization (decode only live rows)
                   │
                   ▼
         Parallel Hash Aggregate (per-thread local tables)
                   │
                   ▼
            Two-Phase Merge ──→ Result
```

## Engine Variants

| Engine | Description |
|---|---|
| **Naive** | Row-by-row reference implementation (gold standard) |
| **Vectorized (early)** | Batch processing, materializes all columns before filtering |
| **Vectorized (late)** | Evaluates predicate first, only decodes key for surviving rows |
| **Parallel** | Thread-local hash tables via rayon + two-phase merge |

## Quick Start

```bash
# Run the engine with default parameters (10M rows)
cargo run --release

# Run benchmarks
cargo bench

# Run differential tests (verify all engines produce identical results)
cargo test
```

## Correctness

All engine variants are verified against the naive row-by-row implementation via
differential testing across multiple data shapes (varying selectivity, cardinality,
batch alignment). See `tests/differential_test.rs`.

## Status

See [docs/MILESTONES.md](docs/MILESTONES.md) for detailed progress tracking.

## License

MIT
