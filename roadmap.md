# DuckJeu Roadmap

项目从 DuckDB Rust 扩展开始，逐步发展为 Judgment Runtime。

## v0.1

**当前状态（2026-09-23）**：**v0.1 / Spec 001 验收完成；评审修正已复验**。修正覆盖 STRUCT 内部 NULL 的字段类型校验、HTTP 非成功状态处理、demo 的 Python 依赖，以及 live acceptance 的凭据优先级。最新 `make demo`、`cargo test --workspace`（65 tests）、`make contract-test`（64/64）通过；TypeSafe live acceptance 中 `jev_prob`、`jev_bool`、`jev_choice` 均对 3 行非敏感样例返回有效结果，行映射、概率范围和类别成员校验通过。实测耗时分别为 2.837s、2.916s、2.737s。请求模型为 `jev-latest`，实际模型名未取得（`model_reported: null`）；每个函数 3 次请求为逐行执行的预期值，未与服务端计费记录核对。完整记录：[live-20260923T100542.json](build/acceptance/live-20260923T100542.json)。实现计划见 [specs/001-judgment-foundation](specs/001-judgment-foundation/plan.md)。

先做最小可运行的 DuckDB 扩展。

**目标**：在 `read_parquet(...)` 的 SQL 查询中调用 JEV，对结构化状态输出概率与类别，再用普通 SQL 做过滤和决策。模型判断不是已验证事实。

**公开 API**：`jev_prob(state, criterion)`、`jev_bool(state, criterion)`、`jev_choice(state, question, choices)`。扩展名称为 `duckjeu`，用 Rust 实现，保留一个可控的 mock provider 用于测试。

**完成标准**：固定版本 DuckDB 能加载扩展，Parquet 示例可调用真实 JEV provider 并返回与行对齐的类型化结果；NULL、无效输入、超时与 provider 失败有测试。
  - 已满足：固定版本加载、三个函数的 Parquet 离线示例、NULL/无效输入/超时/provider 失败测试、行对齐，以及真实 TypeSafe 三函数端到端验收。


架构准备要求：v0.1 即分离宿主映射、判断语义、provider 协议、执行策略与连接配置，具体约束见 [spec.md 第 9 节](spec.md#9-面向-roadmap-的架构约束)。后续能力按下列阶段交付。

## 后续里程碑

**历史实现快照（2026-09-23）**：当时 TypeSafe batch live probe 尚未取得响应，因此里程碑均未验收；该快照已被下方 2026-09-24 状态更新 supersede。

**原验收记录（2026-09-25 本轮评审前）**：v0.2 原验收在 DuckDB v1.5.5 / macOS arm64 通过。TypeSafe Noul/Choice 真实复合探针及 DuckJeu 扩展 SQL 端到端均通过；实际模型为 `jev-1.13.0`，扩展 SQL 对 4 个唯一判断发出 2 个 batch 请求。`make check`、workspace Rust 69 项、基础 SQL 64/64、runtime SQL 33/33 和 profile SQL 14/14 均通过；workspace Clippy `-D warnings` 通过。固定种子 1K/10K/100K stub batch 对照也通过。2026-09-24 review 修正后的 TypeSafe 三函数 live acceptance 通过（[live evidence](build/acceptance/live-20260924T144622.json)）。profile SQL stub 确认 row 模式接受 50KB 输入、optimized 空白 criterion 失败会刷新 profile 并记录输入行，以及 self-hosted 显式 `jev-latest` 原样进入请求和 profile。TypeSafe 与 local-jev 三函数 live SQL 均已通过；local-jev 固定源码验收的峰值 RSS 为 703.1 MiB、峰值 memory footprint 为 1.51 GiB、无 swap；实际加载权重 revision 未知并关闭跨查询缓存，详见 [local-service evidence](specs/002-judgment-runtime-evolution/evidence/local-service.md)。local-jev 固定种子 1K/10K/100K 真实服务对照现已通过：100K row/optimized/prefilter 分别发出 95,080/9,999/47,595 次 HTTP；batch 在请求前以 unsupported 失败，未知权重 revision 下缓存为零命中；费用未知且未发生远端 API 调用。完整证据见 [v0.2 acceptance](specs/002-judgment-runtime-evolution/evidence/v02-acceptance.md)、[v0.3 acceptance](specs/002-judgment-runtime-evolution/evidence/v03-acceptance.md)、[v0.4 acceptance](specs/002-judgment-runtime-evolution/evidence/v04-acceptance.md) 和 [Spec 002 evidence](specs/002-judgment-runtime-evolution/evidence/)。

**当前状态（2026-09-25 评审后）**：v0.2 与 v0.4 有待修复项，原验收证据保留，整体完成状态待复验。逐行查询在慢速 provider 调用期间被中断时，取消标志要到 `QueryEnd` 才传播，可能继续发送该 chunk 的请求；需在执行期间检查 DuckDB 中断状态并验证后续请求停止。profile 当前仅凭 `typesafe` provider 名称及用量套用公开费率，没有核查自定义 `duckjeu_api_url` 或模型的计价来源；需在来源未核实时将费用标为未知。v0.3 的既有 provider 验收记录不受这两项评审发现影响。

- **v0.2 — 批处理与缓存（原验收通过，取消传播待修复、复验）**：完整身份去重、通用批次关联/拆分、连接并发控制、有界缓存均通过离线契约；TypeSafe `jev-1.13.0` effective model 的 Noul/Choice 复合语义与 DuckJeu SQL 端到端请求数验收通过。合批显式 opt-in，其他或模型漂移 fail closed；请求字节上限只约束 optimized 模式，row 模式 50KB 输入已通过专项 SQL 验收。固定种子 1K/10K/100K stub 对照全部正确。
- **v0.3 — Provider 抽象（已验收）**：共享 Rust judgment core 可独立编译测试；TypeSafe 与固定源码 local-jev 均通过三函数 live SQL。local-jev 报告 `nli-deberta-large`，资源数据为 703.1 MiB 峰值 RSS、1.51 GiB 峰值 memory footprint、零 swap；实际加载权重 revision 未知，因此跨查询缓存关闭。local-jev 跨状态 batch 继续标为 unsupported，证据见 [local-service evidence](specs/002-judgment-runtime-evolution/evidence/local-service.md)。
- **v0.4 — Profiling（原验收通过，计价来源待修复、复验）**：profile SQL contract 验证输入校验失败会刷新最近 profile 并记录尝试行；1K/10K/100K row、optimized、prefilter、batch、cache stub 对照通过，stub batch 分别实测 8/71/710 次 HTTP。真实 local-jev 四模式三规模结果见 [live benchmark](specs/002-judgment-runtime-evolution/evidence/benchmark-local-live.json)：100K row/optimized/prefilter 为 95,080/9,999/47,595 次 HTTP；batch 明确 unsupported 且前置失败、cache 因权重 revision 未知而零命中。真实本地服务费用、TypeSafe 多规模性能均未知/未测，不以 stub 时延外推。
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
