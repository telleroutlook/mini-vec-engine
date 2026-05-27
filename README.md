# mini-vec-engine

DataFusion-flavored toy vectorized engine, in active development.

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

## Status

See [docs/MILESTONES.md](docs/MILESTONES.md) for detailed progress tracking.

## Benchmarking

All benchmarks use cycle-accurate TSC timing with environment noise detection, carried over
from the HFT latency lab project.

```bash
cargo bench
```

## License

MIT
