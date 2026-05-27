# DataFusion Physical Execution Layer vs mini-vec-engine: Technique Mapping

> Phase 1 deliverable. Maps concepts between our toy engine and Apache DataFusion's production implementation.

## Query Lifecycle Comparison

```
DataFusion:  SQL → Parser → LogicalPlan → Optimizer → PhysicalPlan → Stream<RecordBatch>
mini-vec:    (hard-coded)                              → Pipeline    → Vec<RecordBatch>
```

DataFusion goes through full SQL parsing, logical plan optimization, and physical plan generation, then executes via async `SendableRecordBatchStream`. Our engine skips the first three steps and directly executes the physical pipeline.

## 1. Columnar Memory Layout

| Concept | mini-vec-engine | DataFusion / Arrow |
|---|---|---|
| Columnar array | `Vec<u32>`, `Vec<i64>` | `PrimitiveArray<T>` = values buffer + validity bitmap + `ArrayData` |
| Batch | `RecordBatch { keys, vals, num_rows }` | `arrow::record_batch::RecordBatch` = schema + column arrays |
| Validity | None (all rows valid) | `NullBufferBuilder` → `BooleanArray` validity mask |
| Batch size | Fixed 2048 rows | Configurable, default 8192 |

**Key difference**: Arrow's `ArrayData` packages values buffer, validity bitmap, and offsets into a self-describing structure supporting arbitrary types. We use concrete `Vec<u32>` / `Vec<i64>` to avoid type-erasure overhead, at the cost of generality.

## 2. Predicate Evaluation & Selection Bitmap

### mini-vec-engine

```rust
fn evaluate_predicate(vals: &[i64], threshold: i64) -> SelectionBitmap {
    let mut sel = Bitmap::<32>::zeroed();
    for (i, &v) in vals.iter().enumerate() {
        if v > threshold { sel.set(i); }
    }
    sel
}
```

Element-wise scan of the val column, setting bitmap bits for rows satisfying `val > threshold`. Output is `Bitmap<32>` (2048 bits); downstream operators use `iter_set_bits()` to access only surviving rows.

### DataFusion FilterExec

File: `datafusion/physical-plan/src/filter.rs`

`FilterExec` pulls input batches asynchronously via `SendableRecordBatchStream`:
1. `input.poll_next_unpin(cx)` — pull one batch
2. `predicate.evaluate(&batch)` — evaluate predicate → `ColumnarValue` → `BooleanArray`
3. `filter_record_batch(&batch, filter_array)` — call arrow-rs filter kernel
4. `LimitedBatchCoalescer` — merge small filtered batches up to target size (default 8192 rows)

### arrow-rs filter kernel

File: `arrow-select/src/filter.rs`

Entry point: `filter(values: &dyn Array, predicate: &BooleanArray) -> Result<ArrayRef>`

`FilterBuilder` provides three strategies (`IterationStrategy::default_strategy`):
1. **Selectivity > 80%** → `SlicesIterator`: iterate contiguous `[start, end)` ranges, memcpy compress
2. **Selectivity ≤ 80%** → `IndexIterator`: iterate individual true-bit indices, gather
3. **All / None**: direct slice or return empty

Core dispatch uses `downcast_primitive_array!` macro to generate specialized code paths per physical type.

**Comparison**: Our `Bitmap::iter_set_bits()` uses `trailing_zeros` + `x & (x-1)` bit-by-bit scanning, corresponding to arrow-rs's scalar path. Arrow-rs additionally has a SIMD fast path that we have not yet implemented.

## 3. Vectorized Filter Strategies

| Strategy | mini-vec-engine | DataFusion |
|---|---|---|
| Early materialization | Filter into temp Vec, then aggregate | `filter()` kernel directly outputs compressed RecordBatch |
| Late materialization | Selection bitmap passed to aggregate stage, only reads live rows | DataFusion does not explicitly support — filter always materializes |

**Key difference**: Our late materialization skips decoding the key column for filtered-out rows. This is one of the core techniques used by ClickHouse and DataFusion when competing on ClickBench. DataFusion's filter always fully materializes output batches.

## 4. Hash Aggregation

### mini-vec-engine

```rust
// Single-threaded: direct HashMap accumulation
// Parallel: rayon fold → thread-local HashMap → merge_maps
```

- Each batch iterates set bits of the selection bitmap
- `HashMap<u32, i64>` accumulates key → sum
- Parallel version uses `rayon::par_iter().fold()` for thread-local HashMaps
- Final `merge_maps` combines all thread-local HashMaps

### DataFusion GroupedHashAggregateStream

File: `datafusion/physical-plan/src/aggregates/row_hash.rs`

Core struct `GroupedHashAggregateStream` contains:
- `group_values: Box<dyn GroupValues>` — stores deduplicated group keys and maps to group indices
- `accumulators: Vec<Box<dyn GroupsAccumulator>>` — one per aggregate expression
- `current_group_indices: Vec<usize>` — per-row group index
- State machine: `ReadingInput → ProducingOutput → Done`

Per-batch execution flow (`group_aggregate_batch`):
1. `evaluate_group_by()` — evaluate GROUP BY expressions
2. `group_values.intern(keys, &mut indices)` — look up / assign group index
3. `acc.update_batch(values, group_indices, ...)` — vectorized accumulation

GroupValues implementations (`aggregates/group_values/mod.rs`):
- `GroupValuesPrimitive<T>` — single primitive column (Int32, Float64, etc.)
- `GroupValuesBoolean` — boolean type
- `GroupValuesBytes<O>` — string/binary
- `GroupValuesColumn` — multi-column GROUP BY
- `GroupValuesRows` — Arrow row format fallback

Two-phase aggregation modes:
1. `AggregateMode::Partial` — each partition aggregates independently → outputs `[key, partial_sum_state]`
2. `RepartitionExec(Partitioning::Hash([key], M))` — repartition by key hash
3. `AggregateMode::FinalPartitioned` — each partition merges partial states for its key subset

Memory management: `OutOfMemoryMode` controls behavior (`EmitEarly` / `Spill` / `ReportError`).

Skip aggregation optimization: when group count / input row ratio > 0.8, skip aggregation and directly convert rows to state.

**Comparison table**:

| Concept | mini-vec-engine | DataFusion |
|---|---|---|
| Group key | `u32` directly | `Row` encoding (generic byte array comparison) |
| Hash table | `std::HashMap` | `ahash::HashMap` + custom `GroupTracker` |
| Accumulator | `*entry.or_insert(0) += val` | `Accumulator::update_batch(values)` trait |
| Parallel strategy | rayon fold + merge | Partition → Partial aggregate → Shuffle → Final |
| Merge | `merge_maps` single-threaded | `Accumulator::merge_batch()` parallelizable |

## 5. Type Dispatch (Monomorphized Dispatch)

### mini-vec-engine

```rust
// Bitmap<const W: usize> — compile-time fixed word count
// Produces monomorphized instances for W=1, W=2, W=4, ...
```

### DataFusion / arrow-rs

```rust
// downcast_primitive_array! macro
downcast_primitive_array! {
    arr => match arr {
        // Generate specialized code per physical type
        Int32Array => ...,
        Int64Array => ...,
        Float64Array => ...,
    }
}
```

Arrow-rs's `downcast_primitive_array!` macro expands to `match data_type { Int32 => ... Int64 => ... }`, making each type branch call a monomorphized kernel. This is the same pattern as our `Bitmap<W>` monomorphization by word count — **lifting runtime differences to compile time**.

## 6. Execution Model

| Dimension | mini-vec-engine | DataFusion |
|---|---|---|
| Execution | Synchronous pull (for loop over batches) | Async Stream (`poll_next` / `SendableRecordBatchStream`) |
| Parallelism | rayon (work-stealing thread pool) | Tokio + rayon (mixed async + sync kernels) |
| Scheduling unit | One task per batch | Morsel (a batch of rows as one scheduling unit) |
| Memory management | Unlimited | `MemoryPool` + spilling to disk |

**Key difference**: DataFusion's async Stream model allows operators to yield during I/O waits (cooperative multitasking), while our synchronous model assumes all data is in memory. This is the largest architectural gap between a toy engine and a production engine.

## 7. Performance Measurement Infrastructure

| Tool | mini-vec-engine | DataFusion |
|---|---|---|
| Timing | TSC + lfence dual barrier | `Instant::now()` / `metrics::ExecutionPlanMetrics` |
| Latency reporting | HdrHistogram p50/p99/p999 | criterion + `Metrics` |
| Environment detection | `/proc/self/status` context switches + `/proc/interrupts` IRQ | None built-in |
| Profiling | `perf stat` (scripted) | `EXPLAIN ANALYZE` + `metrics` |

Our TSC dual-barrier timing is an order of magnitude more precise than `Instant::now()` (~20 cycles vs ~100+ cycles overhead), but at the cost of platform binding (x86_64 + invariant TSC).

## 8. DataFusion Core Mechanisms Not Yet Covered

| Mechanism | Description | Difficulty |
|---|---|---|
| **Spilling** | Aggregate overflow to disk, streaming processing | High |
| **Sort + Merge** | External Sorter for ORDER BY | Medium |
| **Join** | Hash Join / Sort-Merge Join | High |
| **Expression JIT** | Compile expressions with LLVM or Cranelift | Very High |
| **Parquet Scan** | Columnar storage read + predicate pushdown + projection pushdown | Medium |
| **Dynamic Filtering** | Runtime filter propagation between scan and filter | High |
| **Repartitioning** | Hash/Random RoundRobin partition redistribution | Medium |

## Conclusion

mini-vec-engine precisely covers the core concepts of vectorized execution (columnar layout, selection bitmap, late materialization, parallel hash aggregation, two-phase merge), but the gap with DataFusion is primarily in three areas:

1. **Generality**: DataFusion handles arbitrary types (via `downcast_primitive_array!`), arbitrary operator composition (via `ExecutionPlan` trait), and arbitrary SQL (via Parser + Optimizer)
2. **Scalability**: DataFusion supports async execution, memory management, spilling, and multi-node distribution
3. **SIMD fast paths**: arrow-rs filter/take kernels have hand-written SSE/AVX optimizations

Next contribution direction: contribute SIMD optimizations or filter kernel micro-optimizations to arrow-rs or DataFusion's physical expression layer, directly leveraging our bitmap + cache-line alignment + measurement discipline expertise.

## Key Source File Index

| Component | Repository | File Path |
|---|---|---|
| FilterExec | apache/datafusion | `datafusion/physical-plan/src/filter.rs` |
| Hash Aggregate | apache/datafusion | `datafusion/physical-plan/src/aggregates/row_hash.rs` |
| AggregateMode | apache/datafusion | `datafusion/physical-plan/src/aggregates/mod.rs` |
| GroupValues | apache/datafusion | `datafusion/physical-plan/src/aggregates/group_values/mod.rs` |
| Arrow filter kernel | apache/arrow-rs | `arrow-select/src/filter.rs` |
| Downcast macros | apache/arrow-rs | `arrow-array/src/cast.rs` |
| RepartitionExec | apache/datafusion | `datafusion/physical-plan/src/repartition/mod.rs` |
| Stream infrastructure | apache/datafusion | `datafusion/physical-plan/src/stream.rs` |
