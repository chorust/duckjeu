# Tasks: DuckJeu v0.2–v0.4 Judgment Runtime Evolution

**Input**: [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md), [data-model.md](data-model.md), [contracts/](contracts/), [quickstart.md](quickstart.md)

**Scope**: 依次交付 v0.2 的去重、真实合批与缓存，v0.3 的共享核心与真实本地服务，v0.4 的概况与四模式对照。每版只有在对应验收通过后才能在 `roadmap.md` 标为已实现。

**Task format**: `[P]` 表示该项可与同阶段其他标记项在不同文件并行；`[USn]` 对应 Spec 002 的用户故事。所有 live 验收都由显式开关触发，不将可控服务的测量冒充真实服务证据。

## Phase 1: Setup（基线与可行性门槛）

**Purpose**: 固定 v0.1 可回归基线，验证锁定版 DuckDB 的连接生命周期桥接可行性。

- [X] T001 在 `specs/002-judgment-runtime-evolution/evidence/v01-baseline.md` 记录 DuckDB/Rust/macOS 版本、`make release`、`make demo`、`make contract-test` 的结果及现有三函数、NULL/错误契约；失败先定位基线原因。
- [X] T002 用 `src/host/client_context_probe.cpp` 与 `build.rs` 做 DuckDB v1.5.5 `ClientContextState` 最小编译/加载探针，在 `specs/002-judgment-runtime-evolution/evidence/connection-probe.md` 记录连接关闭析构、QueryBegin/QueryEnd（含失败/取消）的实测行为；探针失败时停止跨查询缓存/概况实现并修订设计，禁止用进程全局 connection-ID 表替代。

**Checkpoint**: v0.1 基线已留证，连接生命周期设计有实测依据。

---

## Phase 2: Foundational（所有故事的前置边界）

**Purpose**: 建立宿主连接状态、执行配置和统一 provider 请求接口，维持现有 SQL 行为。

- [X] T003 将已通过的探针收敛为 `src/host/client_context.cpp`、`src/host/mod.rs` 和 `build.rs` 的薄桥接：Rust 状态由真实连接拥有，连接关闭释放，查询开始/结束回调可识别失败与取消；仅桥接生命周期与注册，不放判断规则。
- [X] T004 在 `src/host/connection.rs` 与 `src/lib.rs` 接入连接状态获取、查询生命周期标识和共享引用，确保同一查询的函数实例/数据块共享状态、不同查询和连接隔离，扩展初始化临时连接不被误当作用户连接。
- [X] T005 在 `src/config.rs` 注册和冻结 `duckjeu_execution_mode`、batch/max-request/max-inflight 设置，校验正数、模式冲突与 prepared statement 重跑后的新设置，非法配置在外发前报脱敏错误。
- [X] T006 在 `src/provider/mod.rs` 定义 `capabilities()`、逐项关联的 `judge_many()`、`verified/unsupported/unknown` 和请求/实际模型、用量、推理时间的可选元数据；未知能力不可当作支持。
- [X] T007 调整 `src/provider/mock.rs` 和 `src/provider/typesafe.rs` 实现 T006 的单请求兼容路径，保留 v0.1 的响应上限、超时、错误脱敏与不自动重试语义。
- [X] T008 在 `src/functions/mod.rs` 将函数执行状态接到 T004 的连接状态及 T005 的配置快照，保持 `row` 默认模式与现有三函数签名、NULL 短路和逐行输出行为。
- [X] T009 在 `test/sql/run_contract_tests.py` 回归连接隔离、配置冲突、prepared statement 配置重读及 v0.1 三函数契约，并将结果写入 `specs/002-judgment-runtime-evolution/evidence/foundation.md`。

**Checkpoint**: 默认逐行行为与 v0.1 一致；后续故事共享稳定的连接、配置和 provider 边界。

---

## Phase 3: User Story 1 — 重复输入仍准确归位，减少重复判断（P1；v0.2）🎯 MVP

**Goal**: 优化模式中以完整身份去重，把经校验的结果写回所有原始行；关闭优化时逐行行为保持不变。

**Independent Test**: 离线脚本对三函数输入重复行、跨 DataChunk 重复、不同问题/候选顺序、内部 NULL 与顶层 NULL，比较逐行结果和实际调用次数；哈希碰撞不能误合并。

### Tests

- [X] T010 [P] [US1] 在 `tests/judgment_identity.rs` 写完整身份相等/不等、编码版本、文本与数值、字段顺序、内部 NULL、候选顺序及强制哈希碰撞测试。
- [X] T011 [P] [US1] 在 `test/sql/run_runtime_contract_tests.py` 写三函数逐行/优化对照、跨 DataChunk 重复、空输入、顶层 NULL、非法输入及实际 provider 调用数断言。

### Implementation

- [X] T012 [US1] 在 `src/judgment.rs` 建立含规范化版本、状态类型与完整字节、问题语义、候选顺序、provider/endpoint/请求模型及凭据作用域的 `JudgmentIdentity`；哈希仅定位，判等比较完整内容，别名实际模型未知时不得跨上下文合并。
- [X] T013 [US1] 在 `src/serialize.rs` 显式输出可版本化的规范化字节，维持结构化字段顺序规范化及字段缺失、内部 NULL、同形异型值的区别。
- [X] T014 [US1] 在 `src/executor.rs` 实现同一查询内跨函数实例/DataChunk 的完整身份去重、已校验结果临时复用与 `JudgmentIdentity → RowTarget[]` 映射；`RowTarget` 包含函数调用实例、数据块和行位置，同一身份的进行中请求只提交一次，查询结束清理临时结果且不计为跨查询 cache 命中。
- [X] T015 [US1] 在 `src/functions/mod.rs` 将优化路径改为完整 DataChunk 收集、预校验、调用 T014、结果完整验证后逐行写回；`row` 路径保留，NULL 不进入执行器，错误不输出部分有效值。
- [X] T016 [US1] 在 `test/stub_server.py` 增加可重现的请求计数和延迟/错误脚本能力，供 T011 证明去重是减少真实 HTTP 尝试而非仅减少内部对象。
- [X] T017 [US1] 运行 `tests/judgment_identity.rs`、`test/sql/run_runtime_contract_tests.py` 的 US1 场景与原有 `test/sql/run_contract_tests.py`，将行归属、调用数及失败结果写入 `specs/002-judgment-runtime-evolution/evidence/us1.md`。

**Checkpoint**: US1 可独立演示去重收益与 100% 行归属；此时尚不宣称跨状态合批或跨查询缓存完成。

---

## Phase 4: User Story 2 — 有界地合批和并发执行（P1；v0.2）

**Goal**: 经真实服务验证后，一次外部请求承载多个不同判断；批次关联、大小与连接内并发均受控。

**Independent Test**: 可控服务检查乱序/缺失/重复/额外响应、请求数与连接峰值；真实服务对至少两个不同状态验证二元及类别关联、行序变化与无关行插入影响。能力失败时显式 `unsupported`。

### Tests and capability gate

- [X] T018 [P] [US2] 在 `test/sql/run_batch_live_acceptance.py` 写显式开关的 TypeSafe 复合 `rows` 探针，记录二元/类别、至少两个不同状态、乱序、无关行插入、逐行对照和真实 HTTP 次数；无凭据记 blocked，不写成通过。
- [X] T019 [P] [US2] 在 `src/executor_tests.rs`、`tests/provider_protocol.rs` 与 `test/sql/run_runtime_contract_tests.py` 覆盖可控 HTTP 多判断单往返、拆批、乱序/缺失/额外/类型错误、超时/取消、共享连接额度和失败不重试；详细结果见 `evidence/us2.md`。
- [X] T020 [US2] 在 `specs/002-judgment-runtime-evolution/evidence/batch-capability.md` 记录 T018 的真实语义与计数结论；Noul/Choice 及 DuckJeu 端到端通过，仅对报告 effective model `jev-1.13.0` 发布 verified；模型漂移时失败关闭。

### Implementation

- [X] T021 [US2] 在 `src/provider/typesafe.rs` 实现同 criterion 的二元请求、同 question 与有序 choices 的类别请求复合 state 构造、唯一 question key、响应逐项严格校验及模型元数据保留，并以 T020 的能力门槛控制启用；provider protocol 与 SQL 集成测试、live evidence 均通过。
- [X] T022 [US2] 不适用：T020 的 TypeSafe 真实探针通过，无需实现替代 provider；local-jev 仍明确保持跨状态合批 unsupported。
- [X] T023 [US2] 在 `src/executor.rs` 按已验证能力和同语义条件分组、按单批判断数/请求字节及服务限制拆批，发送前拒绝不支持的显式合批；完整校验每批 key 集合、值域及标签后才写回，部分失败整查询报错。
- [X] T024 [US2] 在 `src/host/connection.rs` 实现连接级共享并发额度和可取消等待，错误、超时或线程退出时自动归还 permit；不同连接额度互不干扰。
- [X] T025 [US2] 在 `src/functions/mod.rs` 将取消状态和连接 permit 传入 T023，确保并行数据块不能各自突破连接上限，已发生 HTTP 尝试保留计数且错误不自动重试。
- [X] T026 [US2] 在 `src/provider/mod.rs` 按真实验收结果发布 provider 协议版本及问题/批次能力，要求 `duckjeu_batch_enabled=true` 且能力非 `verified` 时请求前明确失败。
- [X] T027 [US2] 运行 runtime SQL、直接 TypeSafe live probe 和 DuckJeu extension live SQL；记录语义/上下文差异、4 个唯一判断对应 2 次 HTTP、profile 批次大小和离线连接峰值，详见 `evidence/us2.md`。

**Checkpoint**: US2 的真实合批验收通过，或有明确 `unsupported` 结论和未完成记录；并发可单独验证，不能代替合批。

---

## Phase 5: User Story 3 — 明确控制缓存的成本和新鲜度（P1；v0.2）

**Goal**: 显式开启时在同一连接跨查询复用已校验结果，内存和时间有界，凭据/模型/连接隔离。

**Independent Test**: 同一连接两次执行出现可核对命中；跨连接、凭据、服务/模型及配置切换零误命中；TTL、条目数、字节上限和 clear 均能触发重新请求。

### Tests

- [X] T028 [P] [US3] 在 `tests/cache_policy.rs` 写成功/失败入缓存、TTL 边界、条目与字节双上限、逐出顺序、完整身份比较和可变模型别名禁止跨查询命中测试。
- [X] T029 [P] [US3] 在 `test/sql/run_runtime_contract_tests.py` 扩展两连接、凭据/服务/模型/配置切换、重复查询、失败结果、清空函数与连接关闭后的缓存隔离断言。

### Implementation and v0.2 acceptance

- [X] T030 [US3] 在 `src/cache.rs` 实现只收纳完整校验成功结果的连接级内存缓存，按条目数、保守字节占用与正 TTL 同时约束，并提供清空和过期/逐出计数。
- [X] T031 [US3] 在 `src/host/connection.rs` 将 T030 绑定连接生命周期与凭据作用域，连接销毁清理；未能在下次请求前确认稳定实际模型版本的别名及本地未知权重禁止跨查询复用。
- [X] T032 [US3] 在 `src/executor.rs` 按同次去重、跨查询 cache lookup、发送、完整响应验证、cache insert 的顺序执行，分开统计去重行、命中判断、批次和实际外部请求。
- [X] T033 [US3] 在 `src/config.rs` 增加默认关闭的 cache 开关及正的条目数、字节数和 TTL 设置；`row` 模式启用 cache 显式报配置冲突，并在每次执行冻结设置。
- [X] T034 [US3] 在 `src/functions/cache_clear.rs` 与 `src/lib.rs` 注册 `duckjeu_cache_clear()`，只清空当前连接并返回已清条目数，确保清空调用不会影响其他连接。
- [X] T035 [US3] 在 `examples/make_sample_data.py` 生成固定种子、重复率、NULL、问题集合及跨 DataChunk 边界的 1K/10K/100K 非敏感数据，并输出数据校验摘要。
- [X] T036 [US3] 在 `test/bench/run_compare.py` 建立 v0.2 逐行/优化初版对照，记录三种规模的正确性、唯一判断数、实测 HTTP 次数、命中率与耗时；无法真实合批的模式标 `unsupported`。
- [X] T037 [US3] 运行 v0.2 离线契约、固定规模对照并整理 T027 真实服务证据，在 `specs/002-judgment-runtime-evolution/evidence/v02-acceptance.md` 汇总通过/未通过项；真实合批门槛未通过时不在 `roadmap.md` 标 v0.2 已实现。

**Checkpoint**: v0.2 验收包含去重、真实合批、连接并发、缓存边界及 1K/10K/100K 对照；失败项保持未完成。

---

## Phase 6: User Story 4 — 在可信服务之间选择判断来源（P2；v0.3）

**Goal**: 抽出无 DuckDB 依赖的 Rust 判断核心，真实接入 local-jev，保持两个服务的 SQL 契约、能力与模型身份清楚可查。

**Independent Test**: 无 DuckDB 环境可验证共享核心；TypeSafe 与固定源码 local-jev 各以三函数、每函数至少三行非敏感输入 live 验收；超出能力前置失败。

### Tests and service preparation

- [X] T038 [P] [US4] 在 `test/sql/run_local_acceptance.py` 写需显式开关的 local-jev 三函数多行、profile 模型/请求计数、255 choice 前置限制、可选认证、失败脱敏及未知权重缓存隔离验收；截断响应头由 `tests/provider_protocol.rs` 的本地 HTTP fixture 覆盖。
- [X] T039 [P] [US4] 在 `crates/judgment-core/tests/core_contract.rs` 写无 DuckDB 环境的规范化、身份、批次 key 校验、缓存、错误及 provider 能力测试。
- [X] T040 [US4] 按固定源码 `64a0b31ff343dca32142496cf9edca5a0174a18d` 部署并探测真实 local-jev，在 `specs/002-judgment-runtime-evolution/evidence/local-service.md` 记录源码、`nli-deberta-large` 权重 revision/未知、协议字段、认证及资源；若权重不可固定或确认，声明跨查询缓存不可用。

### Implementation

- [X] T041 [US4] 在 `Cargo.toml` 建立根扩展与 `crates/judgment-core/Cargo.toml` workspace，保持 DuckDB 依赖只在根 crate，core 可单独编译。
- [X] T042 [US4] 将 `src/serialize.rs` 和 `src/judgment.rs` 的宿主无关实现移至 `crates/judgment-core/src/serialize.rs`、`crates/judgment-core/src/judgment.rs`，在 `crates/judgment-core/src/lib.rs` 导出稳定类型；保持编码版本及验证语义。
- [X] T043 [US4] 将 `src/executor.rs` 与 `src/cache.rs` 的宿主无关规则移至 `crates/judgment-core/src/executor.rs` 和 `crates/judgment-core/src/cache.rs`，连接状态仍由宿主注入，core 不持有 DuckDB 对象或凭据原文。
- [X] T044 [US4] 在 `src/provider/typesafe.rs` 适配共享 core 的请求/批次结果，保留 response `model`、`usage` 原值及每个判断的关联，缺失元数据用未知表示。
- [X] T045 [US4] 在 `src/provider/localjev.rs` 实现真实 `POST /v1/systemone` adapter、独立 Bearer 凭据、二元/类别能力、上限 255 前置检查和 `x-local-jev-truncated` 失败处理；复用 core 校验但不假设两服务概率可比较。
- [X] T046 [US4] 在 `src/provider/mod.rs` 增加 `selfhosted` 选择及服务/模型/协议版本能力清单，local-jev 跨状态 batch 未经单独 live 验证时保持 `unsupported`。
- [X] T047 [US4] 在 `src/config.rs` 增加 `selfhosted` 回环地址默认值与 `DUCKJEU_LOCAL_JEV_API_KEY` 独立受保护读取；服务/模型切换使旧缓存不可命中，错误与 SQL 设置不泄漏凭据。
- [X] T048 [US4] 运行 `crates/judgment-core/tests/core_contract.rs`、原有 SQL/live 验收与 `test/sql/run_local_acceptance.py`，在 `specs/002-judgment-runtime-evolution/evidence/v03-acceptance.md` 记录两服务各三函数至少三行、能力差异、模型/权重身份及未知值；只有真实验收通过才在 `roadmap.md` 标 v0.3 已实现。

**Checkpoint**: core 可脱离 DuckDB 验证，远端与真实本地服务均符合三函数契约；模型差异如实记录。

---

## Phase 7: User Story 5 — 看清时间、费用与优化收益（P2；v0.4）

**Goal**: 查询结束后读到脱敏、可追溯的执行概况；固定数据的四模式、三规模对照可复现。

**Independent Test**: 成功/失败/取消查询的 JSON 符合 profile 契约；未知推理/纯网络/费用为 null；1K/10K/100K 各有逐行、预过滤、合批、缓存记录，stub 与 live 证据分开。

### Tests

- [X] T049 [P] [US5] 在 `test/sql/run_profile_contract_tests.py` 写 profile schema、最近已结束含判断查询、跨函数聚合、失败/取消计数、读取不覆盖、跨连接隔离及敏感文本零泄漏断言。
- [X] T050 [P] [US5] 在 `crates/judgment-core/tests/metrics_contract.rs` 写去重/命中/真实 HTTP 尝试计数、并行耗时不可相加、缺失用量/价格/失败请求使费用未知的测试。

### Implementation and evidence

- [X] T051 [US5] 在 `crates/judgment-core/src/metrics.rs` 实现逐项 `QueryCounts`、批量规模直方图、模型/用量聚合及脱敏版本化 profile；DuckDB `QueryBegin`/`QueryEnd` 生命周期由宿主 `ActiveQuery` 注入，外部请求只统计已进入 provider 调用的网络尝试。
- [X] T052 [US5] 在 `src/provider/typesafe.rs` 与 `src/provider/localjev.rs` 向 profile 传递服务报告模型和用量；TypeSafe/local-jev 未报告可信推理时长时保持 null。
- [X] T053 [US5] 在 `src/host/connection.rs` 用 QueryBegin/QueryEnd 汇聚同一查询的所有判断调用，记录查询墙钟时间和完成/失败/取消状态，仅保留本连接最近一次含判断查询快照。
- [X] T054 [US5] 在 `src/functions/profile.rs`、`src/lib.rs` 与 `src/config.rs` 注册默认关闭的 `duckjeu_profile_enabled` 和 `duckjeu_last_profile()`；无记录返回 NULL，读取本身不覆盖，关闭详细计时时保留 v0.2 基本计数。
- [X] T055 [US5] 在 `crates/judgment-core/src/metrics.rs` 区分序列化耗时、完整 HTTP 往返与服务报告推理时间；无直接证据的纯网络/推理拆分保持 null，并注明时间可能重叠。
- [X] T056 [US5] 在 `crates/judgment-core/src/metrics.rs` 实现依据币种、计费单位、单价、来源、读取日期和全部可能计费请求的真实用量计算估值；缺依据返回 null 与原因，费率实施时复核，不称作账单。
- [X] T057 [US5] 在 `test/bench/run_compare.py` 建立固定种子 1K/10K/100K 的逐行、预过滤、合批能力前置检查及缓存对照，保存 SQL、数据摘要、正确性、请求数、命中率、耗时和费用依据；预过滤效果以实测调用数为准。
- [X] T058 [US5] 在 `specs/002-judgment-runtime-evolution/evidence/v04-acceptance.md` 保存可控服务及真实服务结果，标明不支持能力与未知计价，并运行 profile 与回归契约；local-jev 合批在前置检查显式标记 unsupported。

**Checkpoint**: 可核查每个指标的来源；真实和可控服务结果分离，未知值不伪装为测量值。

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: 公开文档、回归命令和最终状态与实测行为一致。

- [X] T059 [P] 在 `README.md` 与 `README_CN.md` 写执行模式、真实合批能力与复合上下文差异、cache 新鲜度/隔离、selfhosted 限制、profile 与费用未知语义及显式 live 开关。
- [X] T060 [P] 在 `Makefile` 增加 runtime-contract、batch-live、batch-extension-live、local-live、profile-contract、bench 对应入口；真实服务目标需显式 gate，默认离线测试不调用真实收费服务。
- [X] T061 对照 `specs/002-judgment-runtime-evolution/quickstart.md` 执行基线、离线、两个真实服务及四模式验收，回填实际命令/结果并修正过时操作说明。
- [X] T062 将已验收的公开行为与约束同步到根 `spec.md`，核对 `roadmap.md` 的 v0.2/v0.3/v0.4 状态仅反映已通过的门槛，并在 `specs/002-judgment-runtime-evolution/evidence/final-review.md` 留下范围与未完成项。

T061 的基线、离线命令、TypeSafe 直接探针、DuckJeu extension live batch 探针、local-jev 三函数 live SQL 及 local-jev 固定种子 1K/10K/100K 对照均已完成；真实对照只访问本机服务，不产生远端请求费用。完整数值、正确性结果和限制见 `evidence/benchmark-local-live.json` 与 `evidence/v04-acceptance.md`。TypeSafe 多规模性能及账单金额未测试；local-jev 的权重 revision 未知、跨查询缓存关闭、跨状态合批 unsupported。

---

## Dependencies & Execution Order

### Phase dependencies

1. **Setup → Foundational**：T002 的 DuckDB v1.5.5 生命周期探针是 T003–T004、T031、T053 的硬门槛；失败时先修订可验证的宿主接入设计。
2. **Foundational → US1**：默认逐行回归通过后才引入优化执行路径。
3. **US1 → US2、US3**：批次关联与缓存都复用完整身份和行目标；US2 与 US3 在 foundation/US1 稳定后可由不同人并行开发，涉及 `src/executor.rs`、`src/host/connection.rs` 的改动须串行集成。
4. **US1 + US2 + US3 → v0.2**：T037 还依赖 T020/T027 的真实跨状态合批证据；仅有 stub、并发或同 state 多问题不足以标版。
5. **v0.2 → US4 → v0.3**：core 搬迁需以已稳定的执行/缓存契约为基线；local-jev 的源码/模型证据及远端、本地 live 结果是 T048 门槛。
6. **v0.3 → US5 → v0.4**：profile 使用已有连接与 provider 元数据；T058 要求完整四模式三规模报告。Polish 在相应里程碑验收后完成，T062 最后执行。

### Within-story order and parallel opportunities

| Story | Can proceed together | Must follow |
| --- | --- | --- |
| US1 | T010（Rust 身份契约）与 T011（SQL 对照） | T012–T016 后完成 T017 |
| US2 | T018（真实服务探针）与 T019（离线批次契约） | T020 决定 T021/T022 的可用能力；T023–T026 后完成 T027 |
| US3 | T028（缓存策略）与 T029（SQL 隔离） | T030–T036 后完成 T037；T037 还依赖 US2 真实证据 |
| US4 | T038（本地 live 脚本）与 T039（core 契约） | T040 与 T041–T047 完成后执行 T048 |
| US5 | T049（SQL profile）与 T050（core 指标） | T051–T057 后完成 T058 |
| Polish | T059（文档）与 T060（命令入口） | T061 验收后执行 T062 |

`[P]` 仅表示文件层面可并行；写同一文件的任务及依赖真实服务结论的任务顺序执行。测试任务写在实现任务前，实施时先确认其可检出缺失行为，再实现对应功能。

## Implementation Strategy

1. **MVP**：T001–T017。先交付 US1 的正确去重及逐行兼容；它可单独验证，但不代表整个 v0.2 完成。
2. **v0.2**：完成 US2 的真实合批门槛，再完成 US3 的缓存边界和 1K/10K/100K 初版对照；T037 根据验收更新 roadmap。
3. **v0.3**：core 迁移与真实 local-jev 并行准备，T048 验收通过后更新 roadmap。
4. **v0.4**：加入可信概况与四模式对照，T058 验收后更新 roadmap，最后做文档/全程回归。

**Completion rule**: `roadmap.md` 只记录已经完成的里程碑。真实服务不可用或能力不通过时，保留 evidence 中的 blocked/unsupported 和未完成任务，不用可控服务结果替代。
