<div align="center">

# mini-vec-engine

**DataFusion-Flavored Vectorized Query Engine**

[![Rust](https://img.shields.io/badge/Rust-2021-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![Tests](https://img.shields.io/badge/tests-passing-green.svg)]()

A from-scratch vectorized query engine for learning database kernel internals — columnar processing, selection bitmaps, late materialization, and parallel hash aggregation with rigorous benchmarking and differential correctness testing.

</div>

---

## What It Does

Implements the canonical analytics query:

```sql
SELECT key, SUM(val) FROM t WHERE val > C GROUP BY key
```

on `(key: u32, val: i64)` tuples through four progressively optimized engine variants, each verified against a naive row-by-row gold standard.

---

## Engine Variants

| Engine | Strategy | Key Idea |
|:-------|:---------|:---------|
| **Naive** | Row-by-row | Gold standard reference — never optimized |
| **Vectorized (Early)** | Batch + full decode | Materializes all columns before filtering |
| **Vectorized (Late)** | Predicate-first | Evaluates filter first, decodes key only for surviving rows |
| **Parallel** | Thread-local + merge | Per-thread hash tables via Rayon + two-phase merge |

```
Input Columns ──▶ Vectorized Scan ──▶ Predicate Eval ──▶ Selection Bitmap
                                                              │
                   ┌──────────────────────────────────────────┘
                   ▼
            Late Materialization (decode only live rows)
                   │
                   ▼
         Parallel Hash Aggregate (per-thread local tables)
                   │
                   ▼
            Two-Phase Merge ──▶ Result
```

---

## Quick Start

```bash
git clone https://github.com/telleroutlook/mini-vec-engine.git
cd mini-vec-engine

# Run with default parameters (10M rows)
cargo run --release

# Run benchmarks
cargo bench

# Differential tests — all engines produce identical results
cargo test
```

---

## Architecture

```
src/
├── engine/
│   ├── naive.rs        # Row-by-row reference
│   ├── vectorized.rs   # Early & late materialization
│   └── parallel.rs     # Rayon-based parallel aggregation
├── bench_infra/
│   ├── timers.rs       # High-resolution timing
│   ├── histogram.rs    # Latency distribution (HdrHistogram)
│   └── environment.rs  # CPU topology, cache info
├── bitmap.rs           # Selection vector / bitmap
└── lib.rs
```

**Key abstractions:**
- **RecordBatch** — columnar memory layout
- **Selection Bitmap** — compact row filter representation
- **Thread-local Hash Table** — lock-free parallel aggregation

---

## Design Philosophy

This engine is intentionally minimal — no SQL parser, no disk I/O, no distributed execution, no query planner. It focuses on demonstrating:

- **Correctness** — differential testing across all variants
- **Performance** — quantified speedups with statistical rigor
- **Trade-offs** — late vs early materialization measured, not assumed
- **Measurement discipline** — latency histograms, never averages

Inspired by the benchmarking methodology from [hft-latency-lab](https://github.com/telleroutlook/hft-latency-lab).

---

## Benchmarks

Criterion benchmarks compare all four engine variants across varying selectivity, cardinality, and batch alignment configurations. Run:

```bash
cargo bench
```

Results are saved to `target/criterion/` with full HTML reports.

---

## Correctness

All engine variants are verified against the naive row-by-row implementation via differential testing:

- Multiple data shapes (varying selectivity, cardinality, batch alignment)
- Property-based edge cases (empty result, all pass filter, single key)
- See `tests/differential_test.rs`

---

## Status

| Milestone | Description | Status |
|:----------|:------------|:-------|
| M1 | Naive + columnar layout | Done |
| M2 | Vectorized early materialization | Done |
| M3 | Late materialization + differential tests | Done |
| M4 | Parallel aggregation + Criterion benchmarks | Done |
| M5 | Performance optimizations | In progress |

See [docs/MILESTONES.md](docs/MILESTONES.md) for detailed progress.

---

## License

Apache License 2.0
