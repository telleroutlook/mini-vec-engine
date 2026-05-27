# 里程碑与执行状态

> ✅ = 已完成 | 🔜 = 进行中 | ⏳ = 未开始
>
> 最后更新：2026-05-28

| 里程碑 | 验收标准 | 时点 | 状态 |
|---|---|---|---|
| M0 占位仓库 | `hft-latency-lab` public repo 已创建 | 第 0 天 | ✅ |
| M0.5 HFT 收尾 | HFT 方法论修完 + KNOWN_LIMITATIONS + 博客发布 + 仓库冻结 | 第 -2~0 周 | ✅（博客 🔜） |
| M1 地图建成 | 徒手画 DataFusion 查询数据流 + Arrow 内存布局 + `DESIGN.md` + `mini-vec-engine` repo 上线 | 第 2 周 | ⏳ |
| M2 内核读通 | 口头复述 filter/aggregate kernel + CMU 15-721 核心 4 章看完 | 第 6 周 | ⏳ |
| M3 造轮子 | `mini-vec-engine` 公开发布，含 late materialization + differential test + perf benchmark + 技术博客 | 第 11 周 | ⏳ |
| M4 首次贡献 | ≥1 个 PR 已提交并在 review 中 | 第 14 周 | ⏳ |
| M5 性能贡献 | ≥2 个 PR 在 review 中，≥1 个 merged，至少一个带 benchmark 的性能优化 | 第 16 周 | ⏳ |
| M6 专精/前沿 | 子系统常驻贡献者 / 参与成熟 Epic 子任务 / 啃 DuckDB C++ | 第 17 周+ | ⏳ |

---

## Phase 0 — 环境与地图（第 1–2 周）

**目标**：能 build、能跑、能在 debugger 里单步走过一条 `SELECT ... WHERE ... GROUP BY`。

- [ ] Day 1: GitHub 开 public repo `mini-vec-engine`（✅ 已完成）
- [ ] clone `apache/datafusion`，`cargo build`，用 `datafusion-cli` 跑 SQL
- [ ] 画"SQL → LogicalPlan → 优化 → PhysicalPlan → 执行"数据流图
- [ ] 打断点跟踪 RecordBatch 从 TableScan → FilterExec → AggregateExec 的流动
- [ ] 完成 Arrow 内存布局自检
- [ ] 完善 `DESIGN.md` API 契约文档

**产出物**：
1. 笔记《DataFusion 一条查询的生命周期》+ 数据流图
2. `DESIGN.md` API 契约文档
3. `mini-vec-engine` public repo 已上线

**状态**：⏳ 未开始（仓库骨架已搭建，核心任务待启动）

---

## Phase 1 — 读懂"内核层"源码（第 3–6 周）

**目标**：把物理执行/内核层吃到能复述。

**主线（必读）**：
- [ ] 精读 arrow-rs 的 `filter` kernel 和 `take` kernel
- [ ] 精读 `FilterExec` 和 `GroupedHashAggregateStream`
- [ ] 读 `downcast_primitive_array!` 等宏
- [ ] 找到 `metrics` 模块，看 `EXPLAIN ANALYZE` 收集机制

**并行任务**：
- [ ] CMU 15-721 课程：每周 2h × 4 周（Vectorization → Compilation → Parallel Hash Join → Query Execution）

**产出物**：
1. 《DataFusion 物理执行层 vs OGR 引擎：技巧对照》
2. CMU 15-721 核心章节笔记

**状态**：⏳ 未开始

---

## Phase 2 — 造迷你向量化引擎（第 7–11 周）

**目标**：把理论变成肌肉记忆。**不可跳过。**

**Day 1**：
- [ ] 复制 HFT 基础设施到 `src/bench_infra/`（✅ 已完成）

**最小可验收子集**（必做）：

目标查询：`SELECT key, SUM(val) FROM t WHERE val > C GROUP BY key`

- [ ] 向量化扫描 + 谓词求值（selection bitmap）
- [ ] 向量化 filter（按 bitmap 压缩存活行）
- [ ] 并行 hash aggregate（rayon 分 partition + 原子合并）
- [ ] 金标准测试（naive 逐行 + 随机数据 differential testing）
- [ ] benchmark（naive vs 向量化 vs 并行 + per-stage 延迟分布）

**杀手锏**：
- [ ] Late Materialization（先处理 filter 列，再按 bitmap 晚解码其他列）
- [ ] 两阶段并行聚合（线程本地哈希表 + 两阶段合并，诚实标注 NUMA 限制）

**Benchmark 维度**：
- [ ] 吞吐（rows/s）
- [ ] branch-miss / cache-miss（配合 Linux `perf`）
- [ ] 延迟分布（p50/p99/p999，复用 histogram.rs）

**产出物**：
1. 公开 GitHub repo + README（含对比数据 + perf 维度）
2. 技术博客《对照式造轮子：从 OGR 引擎到向量化数据库内核》

**状态**：⏳ 未开始

---

## Phase 3 — 首次开源贡献（第 12–16 周）

**目标**：≥2 个 PR 已提交并在 review 中，其中 ≥1 个已 merged。

- [ ] 第一个 PR：good first issue（补 SQL 函数、修类型 bug、加测试）
- [ ] 第二个 PR：性能优化，带 benchmark 数字

**Plan B**：DataFusion Comet C2R 路线（竞争较小）

**产出物**：≥2 个 PR，≥1 个 merged，至少一个带 benchmark 的性能优化类

**状态**：⏳ 未开始

---

## Phase 4 — 深入与扩展（第 17 周起）

**深度路线**：挑一个子系统专精（join/聚合/parquet/codegen），目标 committer
**广度路线**：迁移到 DuckDB（C++），补 C++ 2-3 周

**状态**：⏳ 未开始

---

## 开源贡献路线

- **找 issue**：`github.com/apache/datafusion/contribute` + `github.com/apache/datafusion-comet/contribute`
- **认领**：在 issue 下评论 `take`
- **路线图**：搜 `label:roadmap` / `label:EPIC`
- **沟通**：Slack / Discord + Community Sync
- **差异化**：找 `performance` / `area:physical-expr` / SIMD 的 issue，用 benchmark 说话
