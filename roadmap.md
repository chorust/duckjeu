# DuckJeu Roadmap

项目从 DuckDB Rust 扩展开始，逐步发展为 Judgment Runtime。

## v0.1

**当前状态（2026-09-23）**：**v0.1 / Spec 001 验收已完成；评审修正已应用，待复验**。本次修正覆盖 STRUCT 内部 NULL 的字段类型校验、HTTP 非成功状态处理、demo 的 Python 依赖，以及 live acceptance 的凭据优先级。此前 `make demo`、`cargo test`、mock/stub SQL 契约验收（53/53）通过；TypeSafe live acceptance 中 `jev_prob`、`jev_bool`、`jev_choice` 均对 3 行非敏感样例返回有效结果，行映射、概率范围和类别成员校验通过。实测耗时分别为 2.837s、2.916s、2.737s。请求模型为 `jev-latest`，实际模型名未取得（`model_reported: null`）；每个函数 3 次请求为逐行执行的预期值，未与服务端计费记录核对。完整记录：[live-20260923T100542.json](build/acceptance/live-20260923T100542.json)。实现计划见 [specs/001-judgment-foundation](specs/001-judgment-foundation/plan.md)。

先做最小可运行的 DuckDB 扩展。

**目标**：在 `read_parquet(...)` 的 SQL 查询中调用 JEV，对结构化状态输出概率与类别，再用普通 SQL 做过滤和决策。模型判断不是已验证事实。

**公开 API**：`jev_prob(state, criterion)`、`jev_bool(state, criterion)`、`jev_choice(state, question, choices)`。扩展名称为 `duckjeu`，用 Rust 实现，保留一个可控的 mock provider 用于测试。

**完成标准**：固定版本 DuckDB 能加载扩展，Parquet 示例可调用真实 JEV provider 并返回与行对齐的类型化结果；NULL、无效输入、超时与 provider 失败有测试。
  - 已满足：固定版本加载、三个函数的 Parquet 离线示例、NULL/无效输入/超时/provider 失败测试、行对齐，以及真实 TypeSafe 三函数端到端验收。


架构准备要求：v0.1 即分离宿主映射、判断语义、provider 协议、执行策略与连接配置，具体约束见 [spec.md 第 9 节](spec.md#9-面向-roadmap-的架构约束)。后续能力按下列阶段交付，当前不提前实现。

## 后续里程碑（均未实现）

- **v0.2 — 批处理与缓存**：利用 DuckDB DataChunk 合并请求、去重、控制并发并增加有界缓存。以 1K/10K/100K 行测量耗时、请求数与缓存命中率，并对照逐行调用验证正确性。
- **v0.3 — Provider 抽象**：抽出共享 Rust judgment core；根据实际协议支持远端 JEV 和经过验证的本地/自托管 JEV 兼容服务，明确模型版本和分数语义。
- **v0.4 — Profiling**：记录序列化、网络、推理、总耗时与费用估计，固定数据集和实验环境，检查预过滤、batch 和缓存效果。
- **v0.5 — PostgreSQL**：用 Rust + pgrx 复用 core；在自管理 PG 上验证同等基础 API，研究缓存、后台 worker、权限和 executor 的差异。
- **v0.6 — 集合 judgment**：设计 window、group、compare；区分整段状态判断与逐行预测再聚合，选择通过真实场景验证的算子。
- **v1.0 — JDL**：从两个数据库的共同需求抽象 Judgment Definition Language，定义版本化 schema、criterion、输出类型、provider/model、阈值和评测信息。
- **后续研究**：Judgment Store、物化列、近似索引、streaming、calibration/drift、蒸馏以及 Python/agent 接口，分别立项。

## v0.1 开发顺序与边界

1. 验证 DuckDB Rust 扩展模板的加载、函数注册、STRUCT/VARCHAR 读取能力，固定编译和运行版本。
2. 打通 Parquet → struct_pack → canonical state → mock/真实 JEV → 概率输出向量。
3. 补齐 bool/choice、NULL、无效输入、超时及 provider 错误的测试。
4. 交付构建加载说明、非敏感样例 Parquet、SQL 和真实服务端到端演示。

v0.1 不做 JDL、PG、复杂缓存、background worker、窗口算子、planner 改造或自动策略执行。记录远端数据外发和费用；Rust 的性能优势需以真实测试验证。

## References

- [recodelabs/duckdb-jev](https://github.com/recodelabs/duckdb-jev)：参考 DuckDB JEV 扩展的 DataChunk 批处理、缓存及并发请求实现。
- [colliber/duckdb-jev](https://github.com/colliber/duckdb-jev)：参考 JEV judgment 到 DuckDB SQL 类型的映射和函数接口设计。

开发时以各仓库当前源码和许可证为准，对照验证 API、性能和行为后再确定 DuckJeu 的实现。
