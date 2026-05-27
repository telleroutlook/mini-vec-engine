# mini-vec-engine API Contract

> Phase 0 deliverable. This document defines the scope boundary for Phase 2 implementation.

## Input

- **Query**: Hard-coded query template `SELECT key, SUM(val) FROM t WHERE val > C GROUP BY key`.
  No SQL parser. Parameters (C, table schema) are configured at construction time.
- **Data**: In-memory `RecordBatch`-style columnar arrays. No file I/O, no Parquet reader.

## Output

- **CLI binary**: Runs a predefined query against generated data, prints results + benchmark stats.
- **Library crate**: Exposes engine components for benchmarking and differential testing.

## Explicit Non-Goals

- ❌ SQL parsing / query optimization / cost-based planner
- ❌ Spilling to disk (in-memory only)
- ❌ Persistent storage / file formats
- ❌ Network protocol / client-server
- ❌ Multi-node / distributed execution
- ❌ NUMA-aware allocation (single-socket 5600G; documented limitation)

## Phase 2 Acceptance Criteria

Given a table of `(key: u32, val: i64)` with configurable row count:

1. **Correctness**: Vectorized results match naive row-by-row implementation on random data (differential test).
2. **Performance**: Measurable throughput improvement over naive baseline, with per-stage latency histograms.
3. **Late materialization**: Demonstrable throughput delta between early and late decode strategies.
4. **Parallel aggregation**: Thread-local hash tables with two-phase merge, scaling with core count.
5. **Benchmark report**: Throughput + branch-miss/cache-miss (via `perf`) + latency percentiles (p50/p99/p999).

## Engine Components

```
┌─────────────────────────────────────────────────────┐
│  RecordBatch (columnar: key[], val[], validity[])    │
├─────────────────────────────────────────────────────┤
│  VectorizedScan   — iterate batches from input       │
│  PredicateEval    — produce SelectionBitmap from val  │
│  LateMaterialize  — decode only live key[] entries    │
│  HashAggregate    — per-partition local hash table    │
│  TwoPhaseMerge    — combine thread-local results      │
├─────────────────────────────────────────────────────┤
│  bench_infra/ (timer, histogram, bench_env)          │
└─────────────────────────────────────────────────────┘
```
