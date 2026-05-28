# Milestones & Execution Status

> ✅ = Complete | 🔜 = In Progress | ⏳ = Not Started
>
> Last updated: 2026-05-28

| Milestone | Acceptance Criteria | Target | Status |
|---|---|---|---|
| M0 Placeholder repo | `hft-latency-lab` public repo created | Day 0 | ✅ |
| M0.5 HFT wrap-up | HFT methodology fixes + KNOWN_LIMITATIONS + blog + repo freeze | Week -2~0 | ✅ (blog 🔜) |
| M1 Map built | Hand-drawn DataFusion query dataflow + Arrow memory layout + `DESIGN.md` + `mini-vec-engine` repo live | Week 2 | ✅ |
| M2 Kernels read | Recite filter/aggregate kernels + CMU 15-721 core 4 chapters | Week 6 | ⏳ |
| M3 Build wheels | `mini-vec-engine` published with late materialization + differential test + perf benchmark + tech blog | Week 11 | ✅ |
| M4 First contribution | ≥1 PR submitted and in review | Week 14 | ✅ PR #22579 |
| M5 Perf contribution | ≥2 PRs in review, ≥1 merged, at least one perf optimization with benchmark | Week 16 | 🔜 PR #22580 |
| M6 Specialization | Subsystem regular contributor / mature Epic sub-task / DuckDB C++ | Week 17+ | ⏳ |

---

## Phase 0 — Environment & Map (Week 1–2)

**Status**: ✅ Complete

- [x] Day 1: Create public repo `mini-vec-engine`
- [x] Clone `apache/datafusion`, `cargo build`, run SQL with `datafusion-cli`
- [x] Draw "SQL → LogicalPlan → Optimize → PhysicalPlan → Execute" dataflow diagram
- [x] Trace RecordBatch from TableScan → FilterExec → AggregateExec
- [x] Complete Arrow memory layout self-check
- [x] Finalize `DESIGN.md` API contract document

**Deliverables**:
1. Notes: "The Lifecycle of a DataFusion Query" + dataflow diagram
2. `DESIGN.md` API contract document
3. `mini-vec-engine` public repo live

---

## Phase 1 — Read the Kernel Layer Source (Week 3–6)

**Status**: ✅ Complete

- [x] Read arrow-rs `filter` kernel and `take` kernel
- [x] Read `FilterExec` and `GroupedHashAggregateStream`
- [x] Read `downcast_primitive_array!` and related macros
- [x] Document comparison: `docs/datafusion_comparison.md`

**Deliverables**:
1. `docs/datafusion_comparison.md` — detailed technique mapping between mini-vec-engine and DataFusion

---

## Phase 2 — Build a Toy Vectorized Engine (Week 7–11)

**Status**: ✅ Complete

- [x] Copy HFT infrastructure to `src/bench_infra/`
- [x] Vectorized scan + predicate evaluation (selection bitmap)
- [x] Vectorized filter (compress live rows by bitmap)
- [x] Parallel hash aggregate (rayon partition + two-phase merge)
- [x] Gold standard test (naive row-by-row + random data differential testing)
- [x] Benchmark (naive vs vectorized vs parallel)
- [x] Late Materialization (process filter column first, late-decode other columns by bitmap)
- [x] Two-phase parallel aggregation (thread-local hash tables + two-phase merge)
- [x] Per-stage latency histograms (p50/p99/p999 via TSC + HdrHistogram)
- [x] Core count scaling benchmark (1/2/4/8 threads)
- [x] Key cardinality sweep benchmark
- [x] Perf stat script for branch-miss/cache-miss

**Implementation inventory**:
- `src/engine/mod.rs` — RecordBatch, AggResult, QueryParams, SelectionBitmap
- `src/engine/data_gen.rs` — Random data generation (configurable rows, cardinality, value range, seed)
- `src/engine/naive.rs` — Row-by-row reference implementation
- `src/engine/aggregate.rs` — evaluate_predicate, aggregate_selected, merge_maps
- `src/engine/vectorized.rs` — Early/late materialization variants
- `src/engine/parallel.rs` — Rayon fold + thread-local hash tables + two-phase merge
- `src/engine/instrumented.rs` — Per-stage TSC latency measurement + histogram reporting
- `src/bitmap.rs` — `Bitmap<W>` multi-word bitmap (fixed iter_set_bits zero-word bug)
- `benches/engine_bench.rs` — Criterion throughput + selectivity sweep + scaling + cardinality
- `tests/differential_test.rs` — 8 differential tests (various data shapes)
- `.github/workflows/ci.yml` — fmt + clippy + test + bench compile
- `scripts/bench-perf.sh` — Linux perf stat for branch-miss/cache-miss
- `docs/datafusion_comparison.md` — Phase 1 technique mapping deliverable

---

## Phase 3 — First Open-Source Contribution (Week 12–16)

**Status**: 🔜 In Progress — 2 PRs submitted

Target: ≥2 PRs submitted and in review, ≥1 merged, at least one performance optimization with benchmark.

**Submitted PRs**:
1. [PR #22579](https://github.com/apache/datafusion/pull/22579) — `ln()` raises error for non-positive input (PostgreSQL compat) — CI green, awaiting review
2. [PR #22580](https://github.com/apache/datafusion/pull/22580) — Optimize `check_short_circuit` with early-exit bit scanning (targets #15631) — CI pending

**Remaining candidate issues**:
1. apache/datafusion#19241 — IN list bitmap filters (bitmap ops, cache-line alignment)
2. apache/datafusion#1823 — bitmap_distinct aggregate (bitmap operations)
3. apache/datafusion#20773 — Cache-efficient partial aggregation (hash agg, parallelism)
4. apache/datafusion-comet#2986 — Optimize slow Comet expressions (good first issue, perf)

---

## Phase 4 — Deepen & Expand (Week 17+)

**Status**: ⏳ Not Started
