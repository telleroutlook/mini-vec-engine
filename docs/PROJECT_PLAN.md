# mini-vec-engine 项目计划

> 目标方向：DataFusion / DuckDB / ClickHouse 这类向量化分析引擎的内核优化。
> 起点：`golomb_vanguard`（OGR 搜索引擎）+ 已收尾的 `hft-latency-lab`（测量基础设施）。
> 核心判断：这不是"转行"，而是"换个名字继续做同一件事"。

---

## 已完成里程碑摘要（M0–M0.5）

**HFT 项目已收尾，进入冻结状态**，作为测量基础设施的"已验证存底"。具体交付：

- ✅ `hft-latency-lab` public repo 上线 + README
- ✅ Bench / Compare 改为真正的 per-message 计时（`parse_all_timed`）
- ✅ microarch 三个名实不符实验全部修正
- ✅ 计时器 `calibrate_ghz` 改为两次 1 秒 pass + 0.5% 一致性检查
- ✅ Histogram `from_cycles` 改用 `.round() as u64`
- ✅ `EnvSnapshot` 增加 `/proc/interrupts` IRQ 总数跟踪 + 10k 阈值警告
- ✅ `docs/KNOWN_LIMITATIONS.md` 发布
- 🔜 配套博客发布（待最后跑一次实验填入实际数字）

---

## 能力映射表：已有代码 → 数据库内核概念

| 你写过的（具体到函数） | 在数据库内核里叫什么 | 在哪个项目里能看到 |
|---|---|---|
| `Bitmap<const W: usize>`（定长多字位图） | Validity bitmap / Selection vector / Bitmap index | Arrow null buffer；DuckDB `ValidityMask` |
| `sum_smallest_unset` — word 级 trailing_zeros + 逐位扫描 | 向量化谓词求值 / filter kernel | arrow-rs `filter` kernel |
| `shl_into` 的无分支跨字移位 | 位级 SIMD 内核 | DuckDB / ClickHouse 手写 SIMD |
| `match words { 1 => find::<1>, ... }` 按W单态化分发 | 按物理类型单态化（monomorphized kernels） | arrow-rs `downcast_primitive_array!` |
| `AlignedAtomicU32` + CAS 循环 | 无锁聚合 / 原子合并 | DuckDB 并行聚合的原子合并 |
| `#[repr(align(64))]` 避免伪共享 | 缓存行对齐避免 false sharing | 高性能并发哈希表 |
| `generate_stubs` — 拆搜索树喂给 rayon | Morsel-driven parallelism / partitioning | DuckDB/HyPer morsel 调度 |
| `SYNC_INTERVAL` 周期同步 `global_best` | 减少跨核同步的批量同步策略 | 并行 join/agg 本地缓冲 + 周期 flush |
| `naive.rs` 作金标准验证快路径 | 参考实现 vs 向量化实现的差分测试 | DuckDB SQL logic test |
| `#[cfg(feature="stats")]` 统计节点数 | 查询引擎 instrumentation / EXPLAIN ANALYZE | DataFusion `metrics` |
| **HFT 的 `timer.rs`**（TSC + lfence 双围栏） | micro-benchmark 计时基础设施 | 已复制到 `src/bench_infra/` |
| **HFT 的 `histogram.rs`**（HdrHistogram 分位数） | 延迟分布报告 | 已复制到 `src/bench_infra/` |
| **HFT 的 `bench_env.rs`**（context switch + IRQ） | bench 环境纯净度自检 | 已复制到 `src/bench_infra/` |

---

## 选主战场

| 项目 | 语言 | 角色 |
|---|---|---|
| **Apache DataFusion** | Rust | **首选主攻** |
| **DataFusion Comet** | Rust + JVM | good first issue 多，C2R 微优化阵地 |
| **DuckDB** | C++17 | 第二阶段：架构标杆 |
| **ClickHouse** | C++20 | 进阶/可选：SIMD 最硬核 |

---

## 三块知识地基

### 3.1 Apache Arrow 内存布局（约 1 周）
- 列式 vs 行式，为什么列式对分析查询快
- Arrow Array 物理结构：values buffer + validity bitmap + offsets
- `Bitmap<W>` 就是 validity bitmap 的近亲
- `RecordBatch` = 一组等长的列 = 向量化执行的基本单位

### 3.2 向量化执行模型（约 1 周）
- 火山模型（逐行 `next()`）的问题：每行一次虚函数调用，无法 SIMD
- 向量化：一次处理一批（1024/2048 行）

### 3.3 核心论文 + CMU 15-721 课程
- **论文优先级**：Roaring Bitmaps → Morsel-Driven → MonetDB/X100 → Compiled-vs-Vectorized → DuckDB
- **CMU 15-721**：每周 2 小时 × 4 周，只看物理执行章节

---

## 风险与对策

| 风险 | 对策 |
|---|---|
| C++ 阅读量大 | 先在 Rust 建概念地图，C++ 推迟到 Phase 4 |
| 引擎全栈太大易迷失 | 死守物理执行/内核层，不碰优化器/SQL 解析 |
| 社区 review 带宽有限 | PR 做小、带测试和 benchmark、先开 issue 讨论 |
| 只读不写停在"看懂" | Phase 2 造轮子是强制项 |
| 被前沿 Epic 诱惑 | 先建"看懂前沿的资格"，Phase 4 才碰 Epic 子任务 |
| 为简历声称测不了的东西 | 诚实标注能测什么/不能测什么 |
| Phase 2 范围失控 | `DESIGN.md` API 契约文档作护栏 |
| HFT 项目复活诱惑 | HFT 已冻结，微架构追问转到 mini-vec-engine |

---

## 资源清单

**源码**：`apache/datafusion`、`apache/arrow-rs`、`apache/datafusion-comet`；`duckdb/duckdb`、`ClickHouse/ClickHouse`

**论文**：Roaring → Morsel-Driven → MonetDB/X100 → Compiled-vs-Vectorized → DuckDB

**课程**：CMU 15-721（Phase 1，每周 2h × 4 周）

**工具**：`criterion`、`perf`、`cargo flamegraph`、cachegrind

**内部复用**：`hft-latency-lab` 的 `timer.rs` / `histogram.rs` / `latency_buf.rs` / `bench_env.rs`（已在 `src/bench_infra/`）
