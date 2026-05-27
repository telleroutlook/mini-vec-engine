# 里程碑与执行状态

> ✅ = 已完成 | 🔜 = 进行中 | ⏳ = 未开始
>
> 最后更新：2026-05-28

| 里程碑 | 验收标准 | 时点 | 状态 |
|---|---|---|---|
| M0 占位仓库 | `hft-latency-lab` public repo 已创建 | 第 0 天 | ✅ |
| M0.5 HFT 收尾 | HFT 方法论修完 + KNOWN_LIMITATIONS + 博客发布 + 仓库冻结 | 第 -2~0 周 | ✅（博客 🔜） |
| M1 地图建成 | 徒手画 DataFusion 查询数据流 + Arrow 内存布局 + `DESIGN.md` + `mini-vec-engine` repo 上线 | 第 2 周 | ✅ |
| M2 内核读通 | 口头复述 filter/aggregate kernel + CMU 15-721 核心 4 章看完 | 第 6 周 | ⏳ |
| M3 造轮子 | `mini-vec-engine` 公开发布，含 late materialization + differential test + perf benchmark + 技术博客 | 第 11 周 | ✅ |
| M4 首次贡献 | ≥1 个 PR 已提交并在 review 中 | 第 14 周 | ⏳ |
| M5 性能贡献 | ≥2 个 PR 在 review 中，≥1 个 merged，至少一个带 benchmark 的性能优化 | 第 16 周 | ⏳ |
| M6 专精/前沿 | 子系统常驻贡献者 / 参与成熟 Epic 子任务 / 啃 DuckDB C++ | 第 17 周+ | ⏳ |

---

## Phase 0 — 环境与地图（第 1–2 周）

**状态**：✅ 已完成

- [x] Day 1: GitHub 开 public repo `mini-vec-engine`
- [x] clone `apache/datafusion`，`cargo build`，用 `datafusion-cli` 跑 SQL
- [x] 画"SQL → LogicalPlan → 优化 → PhysicalPlan → 执行"数据流图
- [x] 打断点跟踪 RecordBatch 从 TableScan → FilterExec → AggregateExec 的流动
- [x] 完成 Arrow 内存布局自检
- [x] 完善 `DESIGN.md` API 契约文档

**产出物**：
1. 笔记《DataFusion 一条查询的生命周期》+ 数据流图
2. `DESIGN.md` API 契约文档
3. `mini-vec-engine` public repo 已上线

---

## Phase 1 — 读懂"内核层"源码（第 3–6 周）

**状态**：⏳ 未开始

---

## Phase 2 — 造迷你向量化引擎（第 7–11 周）

**状态**：✅ 已完成

- [x] 复制 HFT 基础设施到 `src/bench_infra/`
- [x] 向量化扫描 + 谓词求值（selection bitmap）
- [x] 向量化 filter（按 bitmap 压缩存活行）
- [x] 并行 hash aggregate（rayon 分 partition + 原子合并）
- [x] 金标准测试（naive 逐行 + 随机数据 differential testing）
- [x] benchmark（naive vs 向量化 vs 并行）
- [x] Late Materialization（先处理 filter 列，再按 bitmap 晚解码其他列）
- [x] 两阶段并行聚合（线程本地哈希表 + 两阶段合并）

**实现清单**：
- `src/engine/mod.rs` — RecordBatch, AggResult, QueryParams, SelectionBitmap
- `src/engine/data_gen.rs` — 随机数据生成（可配置行数、基数、值域、种子）
- `src/engine/naive.rs` — 逐行参考实现
- `src/engine/aggregate.rs` — evaluate_predicate, aggregate_selected, merge_maps
- `src/engine/vectorized.rs` — early/late materialization 两种变体
- `src/engine/parallel.rs` — rayon fold + 线程本地哈希表 + 两阶段合并
- `src/bitmap.rs` — Bitmap<W> 多字位图（修复了 iter_set_bits 零字 bug）
- `benches/engine_bench.rs` — criterion 吞吐 + 选择率扫描
- `tests/differential_test.rs` — 8 组差分测试（不同数据形状）
- `.github/workflows/ci.yml` — fmt + clippy + test + bench compile

---

## Phase 3 — 首次开源贡献（第 12–16 周）

**状态**：⏳ 未开始

---

## Phase 4 — 深入与扩展（第 17 周起）

**状态**：⏳ 未开始
