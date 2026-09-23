# 设计评审：面向 roadmap 的架构边界（SC-005）

日期：2026-09-22。对象：v0.1 实现（`src/`）与根规格 [spec.md 第 9 节](../../spec.md#9-面向-roadmap-的架构约束)。
结论：§9.2 的七类演进方向 **7/7** 有对应职责、兼容性约束与本期边界；基础 SQL 契约不随 provider 切换改变。

## 一、§9.1 当前边界的落实情况

| 边界 | 落实位置 | 评审结论 |
| --- | --- | --- |
| DuckDB 宿主层 | `src/lib.rs`、`src/functions/`、`src/config.rs` | 只做函数注册、配置接入、ANY 参数读取、NULL 处理、chunk 行映射与输出写回；`duckdb_*` 类型不出现在其他模块 |
| 判断语义层 | `src/serialize.rs`、`src/judgment.rs` | 无 DuckDB、无网络依赖，`cargo test` 可在无数据库进程条件下独立验证（16 项测试） |
| Provider 层 | `src/provider/{mod,mock,typesafe}.rs` | 只做协议、认证与响应转换；不写回向量；mock 与真实 adapter 共用同一 `JudgmentResult` 校验路径 |
| 执行层 | `src/functions/mod.rs::evaluate_row` + `JudgmentRequest::request_key` | 行位置与请求身份（`r0`）显式保留；v0.1 逐行同步，未来合批/乱序不改变结果所属行 |
| 配置与凭据 | `src/config.rs` | session 级配置按执行读取并冻结为不可变 `ProviderContext`；无全局可变配置；凭据仅环境变量 |
| 契约验证 | `tests/`、`test/sql/` | 三层测试（Rust 契约 / SQL+stub / live）互不依赖真实网络 |

评审中确认的两处实现修正（已修复）：

1. `read_state` 在 STRUCT 读取失败时用 `?` 提前返回，会泄漏 `duckdb_logical_type`；改为在单一出口统一销毁。
2. 远端响应正文曾进入错误信息，服务端回显凭据时会泄漏（SC-003 要求零泄漏）；改为只保留状态码与固定提示。

## 二、§9.2 七类演进方向逐项评审

| Roadmap | v0.1 已建立的演进边界 | 本期未交付（明确延期） |
| --- | --- | --- |
| **v0.2 批处理与缓存** | 输入行位置与请求身份独立（`evaluate_row(row)` 与 `request_key`）；canonical bytes 是完整比较内容而非 hash，且带 `canonical_version=1`；请求上下文含问题类型、标准/问题、候选、provider/model，不会把不同判断误视为同一判断 | 合批、去重、并发限制、有界缓存、1K/10K/100K 基准 |
| **v0.3 Provider 抽象** | 判断语义层不依赖 DuckDB 与单一服务协议；`ProviderContext` 显式保留 provider/model/endpoint/超时/大小上限；概率与类别语义分别校验 | 抽取共享 Rust core、本地/自托管 JEV 服务、多服务并行验证 |
| **v0.4 Profiling** | 序列化（`serialize.rs`）、调用（`provider/`）、校验（`judgment.rs`）、写回（`functions/`）职责可区分；`ExecutionState` 是放置计时/计数观测的自然位置 | 分阶段耗时、费用估计、系统性性能分析 |
| **v0.5 PostgreSQL** | 判断语义层与 provider 层可整体复用；宿主差异（连接、事务、向量、错误映射）集中在 `functions/` 与 `config.rs` | pgrx 接入、自管理 PG 验证、权限、缓存与 worker 设计 |
| **v0.6 集合 judgment** | 现有逐行语义明确（每行一次判断、结果按行写回）；整组判断不会被误当作逐行结果聚合 | window/group/compare 场景与算子 |
| **v1.0 JDL** | 内部类型（`QuestionKind`、criterion/question、choices、`JudgmentResult`）可识别，但**不宣称**为稳定 JDL 格式；阈值语义（`p >= 0.5`）固定且文档化 | 版本化 schema、criterion 定义语言、provider/model 与评测信息 |
| **后续研究** | 保存当前契约与演进边界，未为未验证场景预建框架 | Store、物化、索引、streaming、校准/漂移、蒸馏、Python/agent 接口各自立项 |

## 三、基础契约不随 provider 改变

三个 SQL 函数的签名、NULL 语义、概率值域、类别成员规则与错误语义均由 `judgment.rs` 定义，与 provider 无关；
provider 只影响“如何取得结果”。切换 `duckjeu_provider` 不改变 SQL 契约，测试中 mock 与 typesafe 走同一校验路径即为证据。

## 四、本期明确的非目标（与根规格 §7 一致）

JDL、PostgreSQL、复杂缓存、background worker、窗口算子、planner 改造、自动策略执行、性能结论承诺。
