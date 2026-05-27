# DataFusion 物理执行层 vs mini-vec-engine：技巧对照

> Phase 1 deliverable. Maps concepts between our toy engine and Apache DataFusion's production implementation.

## 查询生命周期对比

```
DataFusion:  SQL → Parser → LogicalPlan → Optimizer → PhysicalPlan → Stream<RecordBatch>
mini-vec:    (hard-coded)                              → Pipeline    → Vec<RecordBatch>
```

DataFusion 经过完整的 SQL 解析、逻辑计划优化、物理计划生成，最终通过 async `SendableRecordBatchStream` 拉取执行。我们的引擎跳过前三步，直接执行物理管道。

## 1. 列式内存布局

| 概念 | mini-vec-engine | DataFusion / Arrow |
|---|---|---|
| 列存数组 | `Vec<u32>`, `Vec<i64>` | `PrimitiveArray<T>` = values buffer + validity bitmap + `ArrayData` |
| 批次 | `RecordBatch { keys, vals, num_rows }` | `arrow::record_batch::RecordBatch` = schema + column arrays |
| 有效性 | 无（所有行有效） | `NullBufferBuilder` → `BooleanArray` validity mask |
| 批次大小 | 固定 2048 行 | 可配置，默认 8192 |

**关键差异**：Arrow 的 `ArrayData` 将 values buffer、validity bitmap、offsets 打包成一个自描述结构，支持任意类型。我们用具体的 `Vec<u32>` / `Vec<i64>` 省掉了类型擦除的开销，但丧失了通用性。

## 2. 谓词求值与 Selection Bitmap

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

逐元素扫描 val 列，对满足 `val > threshold` 的行设置 bitmap 位。输出是 `Bitmap<32>`（2048 bits），后续算子通过 `iter_set_bits()` 只访问存活行。

### DataFusion FilterExec

文件：`datafusion/physical-plan/src/filter.rs`

`FilterExec` 通过 `SendableRecordBatchStream` 异步拉取输入 batch：
1. `input.poll_next_unpin(cx)` 拉取一个 batch
2. `predicate.evaluate(&batch)` 求值产生 `ColumnarValue` → 转为 `BooleanArray`
3. `filter_record_batch(&batch, filter_array)` 调用 arrow-rs filter kernel 压缩
4. `LimitedBatchCoalescer` 将小 batch 合并到目标大小（默认 8192 行）

### arrow-rs filter kernel

文件：`arrow-select/src/filter.rs`

入口：`filter(values: &dyn Array, predicate: &BooleanArray) -> Result<ArrayRef>`

`FilterBuilder` 提供三种策略（`IterationStrategy::default_strategy`）：
1. **Selectivity > 80%** → `SlicesIterator`：迭代连续 `[start, end)` 区间，memcpy 压缩
2. **Selectivity ≤ 80%** → `IndexIterator`：逐位迭代 true-bit 索引，gather 操作
3. **All / None**：直接 slice 或返回空

核心分发通过 `downcast_primitive_array!` 宏为每个物理类型生成专用代码路径。

**对照**：我们的 `Bitmap::iter_set_bits()` 用 `trailing_zeros` + `x & (x-1)` 逐位扫描，对应 arrow-rs 的 scalar 路径。Arrow-rs 额外有 SIMD 快路径，我们暂未实现。

## 3. 向量化 Filter 策略

| 策略 | mini-vec-engine | DataFusion |
|---|---|---|
| Early materialization | 过滤到临时 Vec，再聚合 | `filter()` kernel 直接输出压缩后的 RecordBatch |
| Late materialization | Selection bitmap 传到聚合阶段，只读存活行 | DataFusion 不显式支持 — filter 总是物化 |

**关键差异**：我们的 late materialization 跳过了 key 列的解码（对于被过滤掉的行），这是 ClickHouse 和 DataFusion 争 ClickBench 时的核心技术之一。DataFusion 的 filter 总是完整物化输出 batch。

## 4. Hash 聚合

### mini-vec-engine

```rust
// 单线程：直接 HashMap 累积
// 并行：rayon fold → 线程本地 HashMap → merge_maps
```

- 每个 batch 遍历 selection bitmap 的 set bits
- `HashMap<u32, i64>` 做 key → sum 累积
- 并行版本通过 `rayon::par_iter().fold()` 让每个线程维护独立的 HashMap
- 最终 `merge_maps` 遍历所有线程本地 HashMap 合并

### DataFusion GroupedHashAggregateStream

文件：`datafusion/physical-plan/src/aggregates/row_hash.rs`

核心结构 `GroupedHashAggregateStream` 包含：
- `group_values: Box<dyn GroupValues>` — 存储去重 group key 并映射到 group index
- `accumulators: Vec<Box<dyn GroupsAccumulator>>` — 每个 agg expr 一个
- `current_group_indices: Vec<usize>` — 每行的 group index
- 状态机：`ReadingInput → ProducingOutput → Done`

每批次执行流程 (`group_aggregate_batch`)：
1. `evaluate_group_by()` — 求值 GROUP BY 表达式
2. `group_values.intern(keys, &mut indices)` — 查找/分配 group index
3. `acc.update_batch(values, group_indices, ...)` — 向量化累积

GroupValues 实现（`aggregates/group_values/mod.rs`）：
- `GroupValuesPrimitive<T>` — 单列原始类型（Int32/Float64 等）
- `GroupValuesBoolean` — 布尔类型
- `GroupValuesBytes<O>` — 字符串/二进制
- `GroupValuesColumn` — 多列 GROUP BY
- `GroupValuesRows` — Arrow row 格式兜底

两阶段聚合模式：
1. `AggregateMode::Partial` — 各分区独立聚合，输出 `[key, partial_sum_state]`
2. `RepartitionExec(Partitioning::Hash([key], M))` — 按 key hash 重分区
3. `AggregateMode::FinalPartitioned` — 各分区合并对应 key 的 partial state

内存管理：`OutOfMemoryMode` 控制（`EmitEarly` / `Spill` / `ReportError`）。

Skip aggregation 优化：当 group 数 / 输入行数比 > 0.8 时，跳过聚合直接转 state。

**对照表**：

| 概念 | mini-vec-engine | DataFusion |
|---|---|---|
| Group key | `u32` 直接 | `Row` 编码（通用的字节数组比较） |
| Hash 表 | `std::HashMap` | `ahash::HashMap` + 自定义 `GroupTracker` |
| 累积器 | `*entry.or_insert(0) += val` | `Accumulator::update_batch(values)` trait |
| 并行策略 | rayon fold + merge | 分区 → Partial aggregate → Shuffle → Final |
| 合并 | `merge_maps` 单线程 | `Accumulator::merge_batch()` 可并行 |

## 5. 类型分发（Monomorphized Dispatch）

### mini-vec-engine

```rust
// Bitmap<const W: usize> — 编译期固定字数
// 产生 W=1, W=2, W=4, ... 的单态化实例
```

### DataFusion / arrow-rs

```rust
// downcast_primitive_array! 宏
downcast_primitive_array! {
    arr => match arr {
        // 对每个物理类型生成专用代码
        Int32Array => ...,
        Int64Array => ...,
        Float64Array => ...,
    }
}
```

Arrow-rs 的 `downcast_primitive_array!` 宏展开为 `match data_type { Int32 => ... Int64 => ... }`，让每个类型分支调用单态化的 kernel。这与我们的 `Bitmap<W>` 按字数单态化是同一种模式——**把运行时差异提升到编译期**。

## 6. 执行模型

| 维度 | mini-vec-engine | DataFusion |
|---|---|---|
| 执行方式 | 同步拉取（for loop over batches） | Async Stream（`poll_next` / `SendableRecordBatchStream`） |
| 并行框架 | rayon（work-stealing thread pool） | Tokio + rayon（混合 async + 同步 kernel） |
| 调度单位 | 每个 batch 一个任务 | Morsel（一批行作为一个调度单位） |
| 内存管理 | 无限制 | `MemoryPool` + spilling to disk |

**关键差异**：DataFusion 用 async Stream 模型让算子可以在 I/O 等待时让出执行权（cooperative multitasking），而我们的同步模型假设所有数据在内存中。这是从玩具引擎到生产引擎最大的架构差异。

## 7. 性能测量基础设施

| 工具 | mini-vec-engine | DataFusion |
|---|---|---|
| 计时 | TSC + lfence 双围栏 | `Instant::now()` / `metrics::ExecutionPlanMetrics` |
| 延迟报告 | HdrHistogram p50/p99/p999 | criterion + `Metrics` |
| 环境检测 | `/proc/self/status` context switches + `/proc/interrupts` IRQ | 无内置 |
| 性能分析 | `perf stat` (scripted) | `EXPLAIN ANALYZE` + `metrics` |

我们的 TSC 双围栏计时比 `Instant::now()` 精度高一个数量级（~20 cycles vs ~100+ cycles overhead），但代价是平台绑定（x86_64 + invariant TSC）。

## 8. 尚未覆盖的 DataFusion 核心机制

| 机制 | 描述 | 难度 |
|---|---|---|
| **Spilling** | 聚合超过内存限制时落盘，流式处理 | 高 |
| **Sort + Merge** | ORDER BY 的 External Sorter | 中 |
| **Join** | Hash Join / Sort-Merge Join | 高 |
| **Expression JIT** | 用 LLVM 或 Cranelift 编译表达式 | 极高 |
| **Parquet Scan** | 列式存储读取 + 谓词下推 + 投影下推 | 中 |
| **Dynamic Filtering** | 运行时在 filter 和 scan 之间传递过滤条件 | 高 |
| **Repartitioning** | Hash/Random RoundRobin 分区重分布 | 中 |

## 结论

mini-vec-engine 精确覆盖了向量化执行的核心概念（列式布局、selection bitmap、late materialization、并行 hash 聚合、两阶段合并），但与 DataFusion 的差距主要在三个方面：

1. **通用性**：DataFusion 处理任意类型（via `downcast_primitive_array!`）、任意算子组合（via `ExecutionPlan` trait）、任意 SQL（via Parser + Optimizer）
2. **可伸缩性**：DataFusion 支持 async 执行、内存管理、spilling、多节点分布
3. **SIMD 快路径**：arrow-rs 的 filter/take kernel 有手写 SSE/AVX 优化

下一步贡献方向：在 arrow-rs 或 DataFusion 的 physical expression 层面贡献 SIMD 优化或 filter kernel 微优化，直接放大我们的 bitmap + cache-line 对齐 + 测量纪律优势。

## 关键源码文件索引

| 组件 | 仓库 | 文件路径 |
|---|---|---|
| FilterExec | apache/datafusion | `datafusion/physical-plan/src/filter.rs` |
| Hash Aggregate | apache/datafusion | `datafusion/physical-plan/src/aggregates/row_hash.rs` |
| AggregateMode | apache/datafusion | `datafusion/physical-plan/src/aggregates/mod.rs` |
| GroupValues | apache/datafusion | `datafusion/physical-plan/src/aggregates/group_values/mod.rs` |
| Arrow filter kernel | apache/arrow-rs | `arrow-select/src/filter.rs` |
| Downcast macros | apache/arrow-rs | `arrow-array/src/cast.rs` |
| RepartitionExec | apache/datafusion | `datafusion/physical-plan/src/repartition/mod.rs` |
| Stream infrastructure | apache/datafusion | `datafusion/physical-plan/src/stream.rs` |
