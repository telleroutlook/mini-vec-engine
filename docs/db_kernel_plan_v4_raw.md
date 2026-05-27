# 进入数据库 / 分析型计算引擎内核的执行计划 v4（HFT 收尾后版本）

> 目标方向：DataFusion / DuckDB / ClickHouse 这类向量化分析引擎的内核优化。
> 起点：`golomb_vanguard`（OGR 搜索引擎）+ **已收尾的** `hft-latency-lab`（测量基础设施）。
> 核心判断：这不是"转行"，而是"换个名字继续做同一件事"。
>
> **v4 改动说明**：v3 把 HFT 项目的 2 周收尾排进了 §0.1；现在那 2 周已经完成，本版把
> "已完成的工作"压缩到顶部一段状态摘要里，正文焦点全面切到**前向 action items**。
> v3 的核心校准（"先建立看懂前沿的资格，再去碰前沿"）保留不动。

---

## 0. 已完成里程碑摘要（M0–M0.5）

**HFT 项目已收尾，进入冻结状态**，作为测量基础设施的"已验证存底"。具体交付：

- ✅ `hft-latency-lab` public repo 上线 + README
- ✅ Bench / Compare 改为真正的 per-message 计时（`parse_all_timed`）
- ✅ microarch 三个名实不符实验全部修正：
  - SIMD 实验现在使用真正的 AVX2 `_mm256_cmpeq_epi8` 批量比较，type_bytes buffer 在计时窗口外构造
  - branch 实验改名为 `branch_predictor_experiment`，明确说明测的是 BTB warmup 而非静态 hint
  - prefetch 实验改为 5 轮 A/B 交错，prefetch 调用移到 rdtsc 窗口之外
- ✅ 计时器 `calibrate_ghz` 改为两次 1 秒 pass + 0.5% 一致性检查（带 cpuid 检查 TODO）
- ✅ Histogram `from_cycles` 改用 `.round() as u64`（消除截断偏差）
- ✅ `EnvSnapshot` 增加 `/proc/interrupts` IRQ 总数跟踪 + 10k 阈值警告
- ✅ `docs/KNOWN_LIMITATIONS.md` 发布（10 节，覆盖计时器开销、批均值≠单尾、单 socket NUMA 限制、Zen 3 pext microcoded 先验等）
- 🔜 配套博客《Honest Falsification: Four Optimizations That Stopped Mattering on Zen 3》发布（待最后跑一次实验填入实际数字）

**唯一仍待启动的 DB kernel 准备动作**：在 GitHub 开 `mini-vec-engine` public repo（即使只放
一行 README）——这件事并入 Phase 0 第一天的 action item，见 §3。

---

## 1. 校准：你已经站在门里了

很多人转向数据库内核，要先花半年补底层。**你不用补，你已经写过了。** 你缺的不是能力，
而是"把已有能力翻译成行业黑话 + 熟悉一套内存布局标准（Arrow）"。

### 1.1 能力映射表：你已有代码 → 数据库内核概念

| 你写过的（具体到函数） | 在数据库内核里叫什么 | 在哪个项目里能看到 |
|---|---|---|
| `Bitmap<const W: usize>`（定长多字位图，AND/OR/NOT/popcount/ctz） | **Validity bitmap / Selection vector / Bitmap index** | Arrow null buffer；DuckDB `ValidityMask`；ClickHouse `ColumnVector` |
| `sum_smallest_unset` —— word 级 `trailing_zeros` + `x & (x-1)` 逐位扫描 | **向量化谓词求值 / filter kernel**，逐字扫描而非逐元素 | arrow-rs `filter` kernel |
| `shl_into` 的无分支跨字移位、边界处理 | **位级 SIMD 内核**，处理对齐与进位 | DuckDB / ClickHouse 手写 SIMD |
| `match words { 1 => find::<1>, ... }`（按 W 单态化分发） | **按物理类型单态化（monomorphized kernels）** | arrow-rs `downcast_primitive_array!` |
| `AlignedAtomicU32` + `compare_exchange_weak` CAS 循环 | **无锁聚合 / 原子合并** | DuckDB 并行聚合的原子合并 |
| `#[repr(align(64))]` 避免伪共享 | **缓存行对齐避免 false sharing** | 任何高性能并发哈希表 |
| `generate_stubs` —— 拆搜索树前几层喂给 rayon | **Morsel-driven parallelism / partitioning** | DuckDB/HyPer morsel 调度；DataFusion 分区 |
| `SYNC_INTERVAL` 周期同步 `global_best` | **减少跨核同步开销的批量同步策略** | 并行 join/agg 的本地缓冲 + 周期 flush |
| `naive.rs` 作金标准验证快路径 | **参考实现 vs 向量化实现的差分测试** | DuckDB SQL logic test；DataFusion sqllogictest |
| `#[cfg(feature="stats")]` 统计节点数 | **查询引擎 instrumentation / `EXPLAIN ANALYZE`** | DataFusion `metrics` |
| 静态下界 + 节点级动态下界剪枝 | **谓词下推 / 提前终止 / 代价估计** | 优化器 + 算子层 |
| **HFT 的 `timer.rs`**（TSC + lfence 双围栏 + 双 pass 一致性） | **micro-benchmark 计时基础设施** | 直接复制进 mini-engine 的 benchmark 套件 |
| **HFT 的 `histogram.rs`**（HdrHistogram 分位数） | **延迟分布报告**（永远不报均值） | 直接复制 |
| **HFT 的 `bench_env.rs`**（context switch + IRQ 噪声快照） | **bench 环境纯净度自检** | 直接复制 |

**结论**：你的差距是"广度"（一个完整引擎有解析、逻辑计划、优化器、物理执行、存储多层，
你目前精通最底层的物理执行 + 内核），不是"深度"。

---

## 2. 选主战场：先 DataFusion，后 DuckDB/ClickHouse

| 项目 | 语言 | 友好度 | 角色 |
|---|---|---|---|
| **Apache DataFusion** | Rust | ★★★★★ 母语 | **首选主攻** |
| **DataFusion Comet** | Rust + 少量 JVM | ★★★★ | good first issue 多，C2R 微优化阵地（见 §5） |
| **DuckDB** | C++17 | ★★★ | 第二阶段：架构标杆 |
| **ClickHouse** | C++20 | ★★ | 进阶/可选：SIMD 最硬核 |

**推荐路径**：DataFusion（母语建立全引擎心智 + 贡献建信誉）→ 用引擎心智啃 DuckDB 的 C++ →
ClickHouse 作为想专精 SIMD 时的终极目标。

---

## 3. 三块知识地基

### 3.1 Apache Arrow 内存布局（最重要，约 1 周）
- 列式 vs 行式，为什么列式对分析查询快。
- 一个 Arrow `Array` 的物理结构：**values buffer + validity bitmap + offsets**。
- **你的 `Bitmap<W>` 就是 validity bitmap 的近亲**，扫描方式和你的 `iter_set_bits` 一致。
- `RecordBatch` = 一组等长的列 = 向量化执行的基本单位。

### 3.2 向量化执行模型（约 1 周）
- 火山模型（逐行 `next()`）的问题：每行一次虚函数调用，无法 SIMD。
- 向量化：一次处理一批（1024/2048 行）——你 `sum_smallest_unset` 里"一个 word 处理 64 bit"的放大版。

### 3.3 核心论文 + CMU 15-721 课程

**论文**（优先级，与源码穿插读）：
**Roaring Bitmaps（与你 `Bitmap<W>` 最直接）→ Morsel-Driven Parallelism →
MonetDB/X100 → Compiled-vs-Vectorized → DuckDB 架构概览（进阶时读）**。

**课程**：CMU **15-721 Advanced Database Systems**（Pavlo，公开）
- **排程**：绑到 Phase 1（第 3–6 周），**每周固定 2 小时**，只看物理执行相关章节
  （Vectorization、Compilation、Parallel Hash Join、Query Execution & Scheduling）。
- query optimization / distributed 章节**跳过或留到 Phase 4**，别让它挤占造轮子时间。

---

## 4. 分阶段执行计划

约 4–6 个月节奏（每周 8–12 小时）。每阶段有可验收产出物。

> **时间线·谨慎提速**：你 OGR + HFT 底子硬，**读源码**可提速；
> 但**提速的是"读"，不是"建立心智模型"**。**Phase 2 造轮子那一步绝不能跳**。
>
> **Phase 0 不压缩**：即便底子硬，DataFusion 的 async + `Stream` 范式
> 跟你 OGR 的同步 rayon 范式差很远，第一次在 lldb 里单步进 `poll_next` 会被绕一下。
> Phase 0 留满 2 周，提前完成是 bonus，不要从计划上压它。

### Phase 0 — 环境与地图（第 1–2 周）
**目标**：能 build、能跑、能在 debugger 里单步走过一条 `SELECT ... WHERE ... GROUP BY`。

**Day 1 即刻动作**（5 分钟成本）：
- [ ] GitHub 开 public repo `mini-vec-engine`，README 一行字：
      "DataFusion-flavored toy vectorized engine, in active development."
      **从 commit 0 起所有进展都在外面**。commit history 即作品集。

**主体任务**：
- [ ] clone `apache/datafusion`，`cargo build`，用 `datafusion-cli` 跑一条带 filter + aggregate 的 SQL。
- [ ] 画一张"SQL → LogicalPlan → 优化 → PhysicalPlan → 执行"的数据流图（手画）。
- [ ] 打断点跟踪一个 `RecordBatch` 从 `TableScan` → `FilterExec` → `AggregateExec` 的流动。
- [ ] 完成 §3.1 Arrow 地基自检。
- [ ] **关键交付**：写 `mini-vec-engine/DESIGN.md` API 契约文档（半页 markdown，放进 repo）：
  - 输入：是 SQL 文本，还是硬编码查询？是 in-memory `RecordBatch`，还是读 parquet？
  - 输出：是 CLI 工具，还是库 + bench 程序？
  - **明确不做什么**：不做 SQL 解析？不做 optimizer？不做 spill？写清楚。
  - Phase 2 末尾长什么样的一段验收描述。

  **为什么强制这一步**：Phase 2 列了向量化扫描+谓词、filter、parallel agg、late
  materialization、two-phase agg、differential test、perf benchmark——做完全部需要砍 polish；
  但**接口边界不定**就会变成无底洞。这半页文档是 Phase 2 不失控的护栏。

**产出物**：
1. 笔记《DataFusion 一条查询的生命周期》+ 数据流图
2. `mini-vec-engine/DESIGN.md` API 契约文档
3. `mini-vec-engine` public repo 已上线

### Phase 1 — 读懂"内核层"源码（第 3–6 周）
**目标**：把你最擅长的物理执行/内核层吃到能复述。**主线是基本盘，别被前沿特性牵着走。**

**主线（必读）**：
- [ ] 精读 arrow-rs 的 `filter` kernel 和 `take` kernel —— 对照你的 `sum_smallest_unset`。
- [ ] 精读 `FilterExec` 和 `GroupedHashAggregateStream`。重点：
  - selection vector / 布尔掩码怎么从谓词求值出来（你 `valid = !forbidden` 的工业版）。
  - 并行聚合怎么做 partition + 合并（对照你的 `generate_stubs` + `global_best`）。
  - 顺带看它如何处理内存溢出（Spilling）——聚合超内存时落盘的逻辑,
    是真实引擎和玩具引擎的关键区别。
- [ ] 读 `downcast_primitive_array!` 等宏，理解"按物理类型单态化"。
- [ ] 找到 `metrics` 模块，看 `EXPLAIN ANALYZE` 怎么收集行数/耗时（对照你的 `#[cfg(feature="stats")]`）。

**靠前选读**：`RowConverter`（列式 → 可排序 Row 格式）—— 和排序、Join 直接相关，
是基本盘的边缘、值得在主线之后立刻读。

**并行任务·CMU 15-721 课程**：每周 2 小时 × 4 周，共 8 小时核心章节
（Vectorization → Compilation → Parallel Hash Join → Query Execution）。

**选读·加护栏的支线清单**（读完基本盘有余力再看）：
- **Variant 类型**（半结构化 JSON 的强类型列存表示）
- **REE（Run-End Encoded）数组**（游程编码压缩重复数据）
- ⚠️ 这两个是"观光支线"，不是"看懂物理执行层的必经之路"。Phase 1 主线永远是
  filter/take/HashAggregate 基本盘。

**产出物**：
1. 《DataFusion 物理执行层 vs 我的 OGR 引擎：技巧对照》——极好的社区自我介绍材料
2. CMU 15-721 核心章节笔记（自己看得懂即可，不强求公开）

### Phase 2 — 自己造一个迷你向量化引擎（第 7–11 周）
**目标**：把理论变成肌肉记忆。**这是整个计划最关键、不可跳过的一步。**

**Phase 2 Day 1：复用 HFT 基础设施**——直接把 `hft-latency-lab/src/` 里以下文件复制到
`mini-vec-engine/src/bench_infra/`：
- `timer.rs`（已包含双 pass 一致性检查）
- `histogram.rs`（已修 round 偏差）
- `latency_buf.rs`
- `bench_env.rs`（已包含 IRQ 跟踪）

**这件事直接省 1 周时间**，且让两个项目的方法论严丝合缝。HFT 项目此时已收尾冻结，
复制过来的代码就是干净版本。

按 Phase 0 写好的 `DESIGN.md` 范围实施。**最小可验收子集**（必做）：

```
SELECT key, SUM(val) FROM t WHERE val > C GROUP BY key
```

按列式 + 向量化（每批 2048 行）实现，至少包含：

1. **向量化扫描 + 谓词求值**：输入一批列，输出 selection bitmap（直接复用你 `Bitmap<W>` 的思路）。
2. **向量化 filter**：按 selection bitmap 压缩出存活行（对照 arrow `filter`）。
3. **并行 hash aggregate**：rayon 分 partition，每 partition 一个本地哈希表，原子/锁合并——
   套用你 `generate_stubs` + `AlignedAtomicU32` 的并行骨架。
4. **金标准测试**：naive 逐行实现（像你的 `naive.rs`），随机数据 differential testing。
5. **benchmark**：测 naive vs 向量化 vs 并行的吞吐 + 加速比 + **per-stage 延迟分布**
   （复用 HFT 的 `histogram.rs`）。

#### 两个让玩具引擎"显得专业"的杀手锏

**① Late Materialization（晚物化）—— 正好套你的 selection bitmap。**
- **做法**：不要一次解码所有列。**先只处理 filter 列（如 `val`）算出 selection bitmap，
  再按 bitmap 只选择性解码/读取存活行需要的其他列（如 `key`）。**
- **为什么对你天然契合**：你的引擎本来就产出 selection bitmap，这只是多走一步"按掩码晚解码"。
- **产出价值**：README 里直接展示"早物化 vs 晚物化"的吞吐对比。这是 2026 年 DataFusion
  争 ClickBench 榜首的核心手段之一的**玩具版**。

**② 两阶段并行聚合（Two-phase GroupBy）—— 采纳结构，但剥掉测不了的 NUMA 外衣。**
- **做法**：每个线程维护**线程本地哈希聚合表**，最后做**跨线程两阶段合并**。
- ⚠️**诚实校准**：评审建议包装成"NUMA-aware"。但你这台 **5600G 是单 socket、无 NUMA**，
  硬追会变成纸上谈兵、违背"诚实测量"的灵魂。
  **采纳"两阶段并行聚合"这个结构，但诚实地这样称呼它，并在 README 注明
  "NUMA 维度需多 socket 机器才能验证，本机仅验证 thread-local + 两阶段合并的收益"。**

#### benchmark 维度，与 HFT 线统一
- [ ] benchmark 除了吞吐，**加入 branch-miss / cache-miss 维度**（配合 Linux `perf`）。
- [ ] 工具中立：`criterion` 和 `divan` 都可试，**用你测着顺手的**。

**产出物**：
1. 公开 GitHub repo（Phase 0 起就在外面）+ README（含 late materialization 对比 + perf 维度 + 加速比）
2. 一篇技术博客《对照式造轮子：从 OGR 引擎到向量化数据库内核》
   —— 把 §1.1 能力映射表展开成 3000 字带数字、有图、有 perf benchmark 的实战复盘。
   参考你 HFT 收尾时那篇 honest-falsification 博客的格式和深度。

**Phase 2 是你转向数据库内核最有说服力的作品集。**

### Phase 3 — 首次开源贡献（第 12–16 周）
**目标**：在 DataFusion / Comet 提出至少 2 个 PR，建立社区信誉。

**Timeline 软化**：v3 原版写"≥2 个 merged PR"，但 review 周期不可控。改为：
**≥2 个 PR 已提交并在 review 中，其中 ≥1 个已 merged**。剩下的 PR 进 Phase 4 自然合入。

- [ ] **第一个 PR 从 good first issue 入手**（补 SQL 函数、修类型 bug、加测试），熟悉 PR 流程和 CI。
- [ ] **第二个 PR 瞄准你的强项——性能优化**，带 benchmark 数字。

#### Plan B 靶场：DataFusion Comet 的 C2R 路线（竞争没那么激烈）
- **背景**：Comet 在 native（Rust/DataFusion）端算完后，需把**列式数据转回行式（Columnar-to-Row,
  C2R）**给 JVM（Spark）消费。涉及极致的内存布局转换和字节序微操。
- **为什么适合你**：Comet 在 C2R 模块和 Spark 标量/聚合函数转译层有**大量"特定类型快路径
  specialization"的微优化需求**，字节序微操正是你的强项；且 **Comet 竞争通常比 DataFusion 主仓小**。
- **建议**：若 Phase 3 感到 DataFusion 主仓性能 issue 竞争激烈，**直接转到 Comet 的 C2R 模块
  或函数转译层认领 issue**。

#### Dynamic Filtering 相关贡献（方向对·严格降级定位）
- **方向对**：你的 `Bitmap<const W: usize>` 是天然的高性能、对齐缓存行的过滤结构候选。
- ⚠️**关键校准**：dynamic filtering 主干（Epic #15512）**已被 maintainer 做了 DataFusion
  50→52 好几个版本、深度耦合优化器与执行器**——它不是"等你来攻坚的新坑"，是成熟硬骨头。
  **新人绝不该碰主干 Epic。** 正确做法是找它的**叶子**：某个具体过滤内核的微优化、
  某个数据类型的快路径 specialization。
- **流程铁律**：动手前**先按社区规矩开 issue 讨论**——review 带宽是社区最稀缺资源。

**产出物**：≥2 个 PR 已提交并在 review 中，其中 ≥1 个 merged，至少一个是性能优化类（带 benchmark）。

### Phase 4 — 深入与扩展（第 17 周起，持续）
按兴趣二选一或并行：
- **深度路线**：在 DataFusion 挑一个子系统专精（join、聚合、parquet 扫描、表达式 codegen），
  成为常驻贡献者，目标 committer。
  - 走到这一步、且已有 merged PR 建立信誉后，**才适合参与 Late Materialization（Epic #20324）或
    Dynamic Filtering 这类成熟 Epic 的具体子任务**。先有资格，再碰前沿——顺序不能反。
- **广度路线**：把概念地图迁移到 **DuckDB（C++）**。补 C++ 只需 2–3 周（CMake + 模板 + RAII + lldb），
  读 `ColumnDataCollection`、`VectorOperations`、morsel 调度，对照你已会的东西。

---

## 5. 开源贡献的具体路线（DataFusion 实操）

- **找 issue**：`github.com/apache/datafusion/contribute`（自动列 good first issue）+
  `github.com/apache/datafusion-comet/contribute`（新手 issue 更多、更聚焦补 Spark 表达式/算子）。
- **认领**：在 issue 下评论单个单词 `take` 即可自分配，不需 maintainer 批准。
- **路线图**：搜 `label:roadmap` / `label:EPIC`。
  想做没现成 ticket 的功能时，**先开 issue 讨论再写代码**——官方明确建议，避免大 PR 白做。
- **沟通**：Slack / Discord（Comet 共用），定期视频会议（Community Sync）欢迎新人。
  进社区后积极参加每周/双周 Community Sync。
- **现实预期**：**review 带宽是最稀缺资源**，PR 要小、带测试、自解释。

**差异化策略**：多数新贡献者扎堆"加 SQL 函数"。你的独特价值在**性能内核 + 测量纪律**——
找标了 `performance` / `area:physical-expr` / SIMD 的 issue，用 benchmark 说话。
HFT 项目积累的 timer/histogram/perf 维度直接可用，这条赛道竞争者少、maintainer 重视、
最能放大你的 OGR + HFT 合并优势。

---

## 6. 资源清单（一页速查）

**源码（按入场顺序）**：`apache/datafusion`、`apache/arrow-rs`、`apache/datafusion-comet`；
`duckdb/duckdb`、`ClickHouse/ClickHouse`（C++ 阶段）。

**文档**：DataFusion contributor-guide 与 architecture 章节；Arrow columnar format 规范；
DataFusion 官方博客（dynamic filtering / parquet pushdown / limit pruning 等
系列文，是理解前沿 Epic 在做什么的最佳入口，**当读物，不当 Phase 1 任务**）。

**论文**（优先级）：Roaring → Morsel-Driven → MonetDB/X100 → Compiled-vs-Vectorized → DuckDB。

**课程**：CMU 15-721（Phase 1 主线，每周 2h × 4 周，只看物理执行章节）；15-445/645（不上时间线，仅备查）。

**工具**：`criterion` / `divan`（择一顺手的）、`perf` / `cargo flamegraph` / cachegrind（微观归因，
与 HFT 线统一）；C++ 阶段：CMake、`lldb`、Compiler Explorer。

**内部复用资产**：`hft-latency-lab` 的 `timer.rs` / `histogram.rs` / `latency_buf.rs` /
`bench_env.rs`（Phase 2 第一天复制到 `mini-vec-engine/src/bench_infra/`）；
HFT 那篇 honest-falsification 博客的写作模式（数据 + 诚实证伪 + 硬件特定结论）。

---

## 7. 里程碑与自检清单

> ✅ = 已完成；🔜 = 进行中；⏳ = 未开始

| 里程碑 | 验收标准 | 时点 | 状态 |
|---|---|---|---|
| M0 占位仓库 | `hft-latency-lab` public repo 已创建 | 第 0 天 | ✅ |
| M0.5 HFT 收尾 | HFT 方法论修完 + `KNOWN_LIMITATIONS.md` + 一篇博客发布 + 仓库冻结 | 第 -2 ~ 0 周 | ✅（博客 🔜） |
| M1 地图建成 | 徒手画 DataFusion 查询数据流 + Arrow 内存布局 + `DESIGN.md` API 契约定稿 + `mini-vec-engine` repo 上线 | 第 2 周 | ⏳ |
| M2 内核读通 | 口头复述 filter/aggregate kernel 实现并对照 OGR 代码 + CMU 15-721 核心 4 章看完 | 第 6 周 | ⏳ |
| M3 造轮子 | `mini-vec-engine` 公开发布，含 late materialization + differential test + perf benchmark；技术博客发布 | 第 11 周 | ⏳ |
| M4 首次贡献 | ≥1 个 PR 已提交并在 review 中 | 第 14 周 | ⏳ |
| M5 性能贡献 | ≥2 个 PR 在 review 中，≥1 个 merged，至少一个带 benchmark 的性能优化类（DataFusion 或 Comet C2R） | 第 16 周 | ⏳ |
| M6 专精/前沿 | 子系统常驻贡献者 / 参与成熟 Epic 子任务 / 啃 DuckDB C++ | 第 17 周+ | ⏳ |

---

## 8. 风险与对策

| 风险 | 对策 |
|---|---|
| C++ 阅读量大 | 先在 Rust 建概念地图，C++ 推迟到 Phase 4，届时只剩语法关 2–3 周 |
| 引擎全栈太大易迷失 | 死守物理执行/内核层，别一开始碰优化器/SQL 解析 |
| 社区 review 带宽有限、PR 卡住 | PR 做小、带测试和 benchmark、先开 issue 讨论；timeline 用"在 review"而非"已 merged"度量 |
| 只读不写停在"看懂" | Phase 2 造轮子是强制项，是"懂→能写→作品集"的唯一通道 |
| 被前沿 Epic 诱惑、跳过基本盘 | 闪亮 Epic 是 maintainer 成熟战场，不是新人入口；先用 Phase 1–2 建立"看懂前沿的资格"，Phase 4 才碰 Epic 子任务 |
| 为简历声称测不了的东西（如 NUMA） | 单 socket 机器诚实标注能测什么/不能测什么；违背"诚实测量"的作品集反而扣分 |
| Phase 2 范围失控变无底洞 | Phase 0 末尾必须产出 `DESIGN.md` API 契约文档；Phase 2 严格按契约最小集走通，杀手锏作为契约外的进阶题 |
| HFT 项目复活诱惑（"再加个实验"） | HFT 已冻结。**所有微架构追问转到 mini-vec-engine 里以 DataFusion 算子为靶子重测**，不在 HFT 仓库继续投入 |
| "等完美计划再动手"的拖延 | Phase 0 Day 1 即刻 action：开 `mini-vec-engine` repo——成本 5 分钟，剩下的让计划在执行中迭代 |

---

## v4 改动速查

| 改动 | 原因 | 落点 |
|---|---|---|
| 删除独立的 §-1 占位仓库章节 | HFT repo 已上线，合并进 §0 状态摘要 | §0 |
| 删除独立的 §0 HFT 项目串行排序章节 | HFT 已收尾冻结，浓缩为 §0 状态摘要的一段 bullet | §0 |
| 新增 §0 已完成里程碑摘要 | 让 v4 焦点在"前向 action items" | §0 |
| 能力映射表加粗显示 HFT 三个文件 | 强调 Phase 2 直接复用，已是既成事实 | §1.1 末三行 |
| Phase 0 吸收"开 mini-vec-engine repo"action 到 Day 1 | 不再独立列在文档顶部，避免重复 | Phase 0 开头 |
| 风险表新增"HFT 复活诱惑"条目 | 真实风险：修完 HFT 后还想继续加实验 | §8 |
| 里程碑表加状态列（✅/🔜/⏳） | 反映 M0/M0.5 已完成、其余待启动 | §7 |
| Phase 2 博客交付物明确"参考 honest-falsification 格式" | HFT 博客已成为内部写作模板 | Phase 2 产出物 |
| §6 资源清单加"HFT 博客写作模式" | HFT 那篇博客的成功路径可复用 | §6 末尾 |

---

### 一句话总结

**v3 → v4 的差别只是把已完成的工作从计划里抽出去**——HFT 项目作为测量基础设施和写作模板已经
打磨完毕，进入冻结引用状态；mini-vec-engine 的 day-1 action 收进 Phase 0；计划焦点全面切到
DataFusion 的 4–6 个月前向路线。

核心原则不变：**先建立看懂前沿的资格，再去碰前沿。** 已经站在门里，剩下的是把已有能力换个名字。
