# Implementation Plan: DuckJeu v0.2–v0.4 Judgment Runtime

**Branch**: `main`（当前实际分支；setup-plan 的 feature 标识为 `002-judgment-runtime-evolution`，未创建分支） | **Date**: 2026-09-23 | **Spec**: [spec.md](spec.md)

**Input**: [Spec 002](spec.md)、根目录 [spec.md](../../spec.md) §9/§11、[roadmap.md](../../roadmap.md) 和已验收的 [Spec 001](../001-judgment-foundation/spec.md)。本命令只完成 Phase 0–1 的研究与设计，不交付 v0.2–v0.4 实现。

## Summary

在现有 DuckDB 三个判断函数的契约上，分三版交付：v0.2 在 DataChunk 中收集有效行、以完整身份去重、按经验证的服务能力合批、限制连接内并发并提供显式开启的有界跨查询缓存；v0.3 将判断/执行契约抽为共享 Rust core，接入真实本地 JEV 兼容服务并记录模型及能力；v0.4 提供脱敏执行概况和 1K/10K/100K 固定数据对照。TypeSafe 只有一个顶层 state，跨状态合批的复合上下文方案须经过真实语义验证，不能预先宣称完成。

## Technical Context

**Language/Version**: Rust 2021（项目要求 Rust ≥1.85）；锁定版 DuckDB 内部生命周期需要时使用极薄 C++ 桥接；Python 3 用于样例和验收脚本。

**Primary Dependencies**: 现有 `duckdb = ~1.10505.0` / libduckdb-sys、`serde_json`、`ureq 3`；C++ 桥接只依赖 DuckDB v1.5.5 对应头/ABI，构建工具在最小探针后锁定。共享 core 不依赖 DuckDB。

**Storage**: 判断缓存和连接观测仅驻内存、连接关闭即释放；验收报告写入版本可追踪的脱敏 JSON/Markdown 文件。无持久化判断存储。

**Testing**: 纯 Rust 判断/身份/缓存契约；锁定版 DuckDB SQL + 脚本化 HTTP 服务的关联、并发、取消和错误路径；真实 TypeSafe 与本地兼容服务的独立 live 验收；固定规模性能对照。运行命令见 [quickstart.md](quickstart.md)；本规划阶段不运行实现测试。

**Target Platform**: 先保持已验证的 macOS arm64 + DuckDB v1.5.5、`C_STRUCT_UNSTABLE`；其他系统需单独构建/验收。local-jev 模型运行还取决于本机内存与权重下载。

**Project Type**: DuckDB 可加载 Rust 扩展与独立可复用的 Rust 判断核心；可选真实网络服务。

**Performance Goals**: 对重复输入，实际判断数等于唯一判断数；对至少两个不同判断，已验证合批服务的外部请求数小于唯一判断数；连接峰值外部并发不超过设置值。1K、10K、100K 行各记录耗时、请求数与命中率，不凭空设定固定加速倍数。

**Constraints**: 保留 v0.1 三函数签名、NULL、概率/类别、超时、响应大小、脱敏及不自动重试规则；缓存双容量限制和 TTL、凭据/连接隔离、可变模型别名失效；真实服务外发需显式启用；服务未提供的推理/费用证据以未知表示。

**Scale/Scope**: 三个连续里程碑、三种既有判断形式、一个远端与一个真实本地兼容服务、一个连接级缓存/观测状态、1K/10K/100K 四模式实验。PostgreSQL、集合 judgment、JDL 和持久化 store 不在本计划。

## Constitution Check

*GATE: 在 Phase 0 前检查，Phase 1 后复核。*

`.specify/memory/constitution.md` 仍为未填写模板，没有可执行的批准原则或额外门槛；不将其中示例文本视为正式规则。项目有效约束来自根规格 §9/§11、AGENTS.md 和 Spec 002：

| 约束 | 设计落实 | Phase 0 / Phase 1 |
| --- | --- | --- |
| DuckDB 宿主与判断/服务语义分离 | 宿主只做向量、配置和连接生命周期；共享 core 持有身份、校验与执行策略 | 通过 / 通过 |
| 行与响应准确关联、错误脱敏 | 完整身份、question key 与原始行目标；任何缺失/重复/越界失败 | 通过 / 通过 |
| 真实服务能力不得用模拟替代 | TypeSafe 复合状态和 local-jev 分别有 live 验收门槛 | 通过 / 通过 |
| 连接/凭据隔离与内存有界 | 连接生命周期状态、双容量和 TTL、无持久状态 | 通过 / 通过 |
| 状态如实同步 | 规划完成后 roadmap 标为“规划完成，未实现”，只有实现验收通过才升级 | 通过 / 通过 |

无待批准的 constitution 违例。真实跨状态合批、local-jev 运行与 C++ 桥接仍是**实施验证门槛**，不是已经完成的事实；失败时不能标记相应里程碑完成。

## Phase 0: Research Decisions

完整证据、替代方案及能力缺口见 [research.md](research.md)：

1. TypeSafe 官方协议只有一个顶层 state；多行复合 state 是待 live 验证的推断。批次按相同问题语义组合，必须减少真实外部往返。
2. 公开 C API 没有可确认的连接状态析构回调；以 DuckDB v1.5.5 `ClientContextState` 的连接生命周期为准，用薄 C++ 桥接拥有 Rust 状态。先做最小编译/加载与关闭连接探针。
3. 优化模式从完整身份和一对多行目标出发；连接内共用并发额度；缓存默认关闭、大小/时间有界，别名模型无法稳定绑定时不跨查询命中。
4. v0.3 使用源码固定版 local-jev 作为真实本地候选；它的模型与校准不同于 TypeSafe，需单独 live 验收。
5. TypeSafe 响应的 model/usage 应纳入 provider 结果元数据；服务推理时间未见公开字段，费用只按当时官方计价与真实用量估算。

## Phase 1: Design

### v0.2 — 执行、合批、缓存

先作两个窄探针：① 锁定 DuckDB 版本的 `ClientContextState` 能否由扩展安全取得、随连接销毁并在 QueryEnd 记录失败/取消；② TypeSafe 复合 rows 的二元/类别请求能否在真实服务上通过关联与上下文影响检查。探针结果记录在实施证据中。前者失败则禁止跨查询缓存/概况；后者失败则 TypeSafe 的 batch 能力保持不可用，改用另一个经过真实验证的 batch 服务后才可验收 v0.2。

宿主层把当前逐行 `evaluate_row` 拆为“读完整 DataChunk 并构造目标 → 交执行器 → 写回”。保持原行索引与 scalar 调用实例身份，顶层 NULL 不进入执行器。执行器对规范化状态、问题、服务/模型、编码版本和凭据作用域建立完整比较键；先处理同次去重，再检查连接缓存，余下请求按能力和限制打包。TypeSafe 首期只合并相同 criterion 的二元请求，类别请求按相同 question 和有序 choices 分组；批次响应按 question key 校验后回填。并发额度属于连接状态而非单个 chunk；取消/异常确保额度释放，批次部分失败整体查询失败。

缓存由连接状态拥有，按条目数和字节双上限及 TTL 淘汰；只收纳已验证结果。连接关闭释放；配置或凭据上下文改变后旧项不可命中。可变别名即使上一次报告模型版本，只要下一次请求前无法证明仍指向同一版本，就不允许跨查询复用。配置与清空接口见 [sql-runtime.md](contracts/sql-runtime.md)，实体和状态转换见 [data-model.md](data-model.md)。

### v0.3 — 共享核心与本地服务

将宿主无关的状态编码、判断身份/校验、执行器及缓存规则移动到 `crates/judgment-core`；扩展留在根 crate，继续持有 DuckDB 向量、配置和 C++ 连接桥接。Provider trait 返回批次中逐项结果及模型/用量元数据，能力明确区分 `verified`、`unsupported`、`unknown`。现有 TypeSafe adapter 和新的 local-jev adapter 复用验证路径，但分别声明协议上限、截断信号、认证和分数语义。首个 local-jev 验收固定源码、记录权重 revision，用较小的 `nli-deberta-large` 真实完成三函数各至少三行；资源允许时再对照 `llm-qwen3.5-4b`。无法确认权重版本时禁用该服务的跨查询缓存。

### v0.4 — 观测与对照

连接状态利用查询开始/结束回调聚合所有相关判断调用；仅保存最近一次含判断查询的脱敏概况。客户端测量序列化与完整外部往返，服务实际模型/用量从每次响应取；无法证实的推理与纯网络耗时为 null。计价输入必须注明单位、来源和时间；缺少用量或价格时费用未知。概况契约见 [profile-report.md](contracts/profile-report.md)。

固定数据生成器给出重复率、NULL、问题集合与校验摘要。独立验收脚本对 1K/10K/100K 运行逐行、预过滤、合批、缓存；可控服务验证正确性和请求计数，真实服务另外报告性能/用量，不混写成同一证据。预过滤由实际调用数证明，不依赖 SQL 文本顺序推断。

### Interface Contracts and Validation

- [SQL 设置与辅助函数](contracts/sql-runtime.md)：兼容旧函数，新增优化配置、服务选择、缓存清空和概况读取。
- [Provider 合批/能力](contracts/provider-batch.md)：真实/推断能力、完整响应关联、TypeSafe 复合 state、local-jev 限制。
- [概况 JSON](contracts/profile-report.md)：字段、计数口径、时间与费用来源、脱敏规则。
- [快速验收](quickstart.md)：构建、离线契约、两类 live 服务和四模式对照的运行顺序与预期。

## Project Structure

### Documentation (this feature)

```text
specs/002-judgment-runtime-evolution/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── checklists/requirements.md
└── contracts/
    ├── sql-runtime.md
    ├── provider-batch.md
    └── profile-report.md
```

### Source Code (repository root; planned)

```text
Cargo.toml                     # v0.3 workspace: extension + shared core
build.rs                       # v0.2 minimal DuckDB v1.5.5 host bridge build
src/
  lib.rs                      # extension entry and SQL registration
  config.rs                   # session settings and protected credentials
  functions/                  # DuckDB input/output and chunk targets only
  host/                       # scoped connection lifecycle bridge
  provider/
    mod.rs                    # capability and batch protocol
    typesafe.rs               # hosted wire format
    localjev.rs               # validated local candidate
crates/judgment-core/
  src/
    serialize.rs              # canonical state
    judgment.rs               # typed request/response and validation
    executor.rs               # dedup, grouping, row association
    cache.rs                  # bounded cache policy
    metrics.rs                # counters, timing and cost data
examples/                     # fixed non-sensitive data and SQL scenarios
test/sql/                     # SQL/stub and live acceptance
test/bench/                   # reproducible 1K/10K/100K comparison
```

**Structure Decision**: 先在现有单 crate 内形成独立执行边界，再于 v0.3 抽出 core，避免 v0.2 同时承担完整工程重组。C++ 文件只管理 DuckDB 连接状态及查询回调，所有判断规则仍留在 Rust；使用已锁定的 DuckDB 版本并先验证桥接可加载。

## Post-Design Constitution Check

按上表五项重新检查：宿主边界、结果关联、真实服务验收、连接/凭据隔离和状态同步均有对应设计及验收路径。没有 constitution 违例或未定的产品范围；剩余服务/桥接事实均以明确的实施探针和失败门槛处理。Phase 1 设计完成，可进入 `$speckit-tasks`。
