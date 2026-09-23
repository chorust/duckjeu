# Tasks: v0.1 Judgment Foundation（DuckDB Rust 扩展）

**Input**: Design documents from `/specs/001-judgment-foundation/`

**Prerequisites**: plan.md、spec.md、research.md、data-model.md、contracts/、quickstart.md

**Tests**: 本规格明确要求测试（FR-011、SC-002/003/004、research.md D7 三层测试），各故事均包含测试任务，先写测试确认失败再实现。

**Organization**: 任务按 user story 分组，每个故事可独立实现与验证。

## Format: `[ID] [P?] [Story] Description`

- **[P]**: 可并行（不同文件、无未完成依赖）
- **[Story]**: [US1]–[US4]，对应 spec.md 的 User Story 1–4

## Path Conventions

单 crate cdylib 项目：`src/`、`test/`、`examples/` 位于仓库根（见 plan.md Project Structure）。

---

## Phase 1: Setup（项目初始化）

**Purpose**: 可编译、可加载的扩展骨架

- [X] T001 创建 Cargo 项目骨架：`Cargo.toml`（crate-type cdylib，duckdb ~1.10505.0，features `loadable-extension,vscalar`，依赖 ureq 3.x rustls + serde_json）、`src/lib.rs` 扩展入口桩、`src/functions/mod.rs`、`src/serialize.rs`、`src/judgment.rs`、`src/provider/mod.rs`、`src/config.rs` 空模块
- [X] T002 添加 extension-ci-tools git submodule（固定 revision `a285602bf745`），编写 `Makefile`：`make release` = `cargo build --release` + `append_extension_metadata.py --abi-type C_STRUCT_UNSTABLE`，产物输出 `build/release/duckjeu.duckdb_extension`
- [X] T003 冒烟验证：注册一个最小标量函数，本机 DuckDB CLI v1.5.5 以 `-unsigned` 成功 `LOAD` 并调用（落实 research.md D1/D2 探针结论；release + LTO 构建行为在此确认，属遗留风险项）
- [X] T004 [P] 配置 `rustfmt.toml` 与 clippy（`cargo clippy -- -D warnings` 纳入 Makefile `check` target）

**Checkpoint**: `make release` 通过，`duckdb -unsigned` 可加载扩展。

---

## Phase 2: Foundational（判断语义层与宿主桥接基座）

**Purpose**: 所有 user story 共用的契约与桥接；判断语义层零 DuckDB/网络依赖

**⚠️ CRITICAL**: 本阶段未完成前不得开始任何 user story

- [X] T005 [P] 实现 `src/serialize.rs`：CanonicalState 编码（canonical_version=1、字段按名称排序、类型标签、NULL/缺失区分；支持 BOOLEAN/各宽度整数/有限浮点/VARCHAR/NULL；VARCHAR state 原文保留；DATE/TIMESTAMP/嵌套类型/非有限浮点显式报错），纯 Rust 无 DuckDB 依赖（contracts/sql-api.md state 编码节）
- [X] T006 [P] 实现 `src/judgment.rs`：JudgmentRequest（state、kind Noul/Choice、criterion/question、choices、request_key）、JudgmentResult 及响应校验规则（noul ∈ [0,1] 有限；choice 精确成员；answers 缺失/多余/类型错即失败；p>=0.5 含边界阈值），纯 Rust（data-model.md）
- [X] T007 [P] 实现 `src/provider/mod.rs`：Provider trait（接收 JudgmentRequest + ProviderContext，返回 Result<JudgmentResult>）与 ProviderContext 结构（provider/api_url/model/timeout_ms/max_response_bytes/credential），不含 DuckDB 类型
- [X] T008 [P] 实现 `src/provider/mock.rs`：确定性 mock（canonical state + 问题内容 hash 派生 [0,1] 值与 choice label），走与真实 adapter 相同的 JudgmentResult 校验路径
- [X] T009 实现 `src/config.rs`：`duckdb_register_config_option` 注册 5 个配置项（duckjeu_provider/api_url/model/timeout_ms/max_response_bytes，默认值见 contracts/sql-api.md）；scalar init 回调经 client context 按每次执行读取并冻结为 ProviderContext；凭据仅从 `TYPESAFE_API_KEY`（`DUCKJEU_API_KEY` 覆盖）读取
- [X] T010 实现 `src/functions/mod.rs` 宿主桥接共享层：libduckdb-sys FFI 读取 ANY 参数（STRUCT 字段遍历 / VARCHAR）、special NULL handling（顶层 NULL → 输出 NULL 不构造请求）、DataChunk 行 i → 输出向量位置 i 映射、输入校验与 `duckdb_scalar_function_set_error` 脱敏错误写回
- [X] T011 cargo test 基座：serialize/judgment/mock 的纯 Rust 单测（canonical 编码排序与 NULL/缺失区分、越界概率拒绝、未知 label 拒绝、阈值含边界、mock 确定性），全部通过

**Checkpoint**: 判断语义层 `cargo test` 独立通过（无 DuckDB 进程、无网络）；宿主桥接可编译。

---

## Phase 3: User Story 1 — 对数据行取得可信的类型化判断结果 (Priority: P1) 🎯 MVP

**Goal**: `jev_prob(state, criterion)` 对表/Parquet 行返回 [0,1] 概率，mock 离线可用，typesafe 真实可用

**Independent Test**: 加载扩展后对样例 Parquet 执行 `jev_prob`，mock 下结果全部 ∈ [0,1]；stub 下精确值按行对齐；混合 NULL/重复状态跨 DataChunk 边界无错配

### Tests for User Story 1

- [X] T012 [P] [US1] 编写 `test/stub_server.py`：Python http.server 脚本化 HTTP stub（正常响应、越界概率、缺失/多余 answer、401、超时、超大响应），响应格式遵循 contracts/provider-protocol.md
- [X] T013 [P] [US1] 编写 `test/sql/run_contract_tests.py` 的 jev_prob 部分：Python duckdb 1.5.5 加载扩展，覆盖文本/STRUCT 状态、顶层与内部 NULL、概率边界 0/1、空数据集零请求、≥5000 行跨 chunk 行对齐、stub 精确值校验（contracts/sql-api.md 示例）

### Implementation for User Story 1

- [X] T014 [US1] 实现 `src/provider/typesafe.rs`：ureq blocking + rustls POST `{api_url}`（Bearer header、timeout_global、body limit(n)、不自动重试、HTTP 错误上抛且 body 截断 ≤300 字符、凭据不入错误信息）；noul 请求构造（criterion 入 `state.condition`、question key `"r0"`）与 answers 严格校验（恰含 `"r0"`、有限值 ∈ [0,1]）
- [X] T015 [US1] 实现 `src/functions/prob.rs` 并在 `src/lib.rs` 注册 `jev_prob(state ANY, criterion VARCHAR) → DOUBLE`（volatile + special NULL handling）：桥接层读参 → 空白 criterion 拒绝 → CanonicalState → JudgmentRequest → provider 调用 → 结果写入对应行；接入 T009 配置（provider=mock/typesafe 分派，typesafe 缺凭据报错）
- [X] T016 [US1] 跑通 T012/T013 全部测试（2026-09-23 `cargo test` 与离线契约测试通过；stub 已覆盖 typed answer 格式）

**Checkpoint**: US1 独立可用——mock 与 stub 路径 jev_prob 全测试通过。

---

## Phase 4: User Story 2 — 使用布尔值和限定类别完成分析 (Priority: P1)

**Goal**: `jev_bool`（p≥0.5 阈值）与 `jev_choice`（候选集内 label）可用

**Independent Test**: 对样例数据执行 `jev_bool`/`jev_choice`；stub 下验证阈值含边界、choice 返回精确候选成员、未知 label 报错

### Tests for User Story 2

- [X] T017 [P] [US2] 扩展 `test/sql/run_contract_tests.py`：jev_bool 阈值用例（p=0.5 为 true、p 越界报错）、jev_choice 用例（合法 label、未知 label、缺失/多余 answer、<2 选项、重复选项、选项含 NULL、顶层 choices NULL → 结果 NULL 零请求）
- [X] T018 [P] [US2] 扩展 `test/stub_server.py`：choice 响应脚本（合法 label、未知 label）

### Implementation for User Story 2

- [X] T019 [US2] 扩展 `src/provider/typesafe.rs`：choice 问题构造（criteria `{label: null}` 映射，label 原样不修剪）与校验（返回值精确属于候选集）
- [X] T020 [US2] 实现 `src/functions/bool.rs` 并注册 `jev_bool`（同一 noul 请求路径，独立调用不共享请求，p>=0.5 含边界）
- [X] T021 [US2] 实现 `src/functions/choice.rs` 并注册 `jev_choice(state ANY, question VARCHAR, choices VARCHAR[]) → VARCHAR`（构造前校验：≥2 项、不重复、非空、无内部 NULL）
- [X] T022 [US2] 跑通 T017/T018 全部测试（2026-09-23 `make contract-test` 通过；choice 请求与 typed answer 响应已覆盖）

**Checkpoint**: US1+US2 均独立可用——三个函数 mock/stub 路径全通过。

---

## Phase 5: User Story 3 — 安全配置并诊断失败 (Priority: P1)

**Goal**: 连接级配置隔离、凭据安全、全部错误路径显式失败且脱敏

**Independent Test**: 同库两连接不同配置互不污染；prepared statement 在 SET 后取新值；401/超时/超大响应/无效响应全部报错且错误信息零凭据零敏感 state

### Tests for User Story 3

- [X] T023 [P] [US3] 扩展 `test/sql/run_contract_tests.py` 配置隔离用例：两连接分别 SET 不同 api_url/model 验证隔离；SET 后重跑 prepared statement 取新值；provider=typesafe 缺凭据时报错（落实 research.md D5 探针结论为回归测试）
- [X] T024 [P] [US3] 扩展 `test/sql/run_contract_tests.py` 错误脱敏用例：401（stub 回显伪造凭据串时断言错误信息不含该串）、超时（duckjeu_timeout_ms 小值触发）、超大响应（duckjeu_max_response_bytes 小值触发）、错误 body 截断 ≤300 字符

### Implementation for User Story 3

- [X] T025 [US3] 审查并补全 `src/provider/typesafe.rs` 与 `src/functions/mod.rs` 的错误路径：所有 set_error 文案不含凭据与原始敏感 state；HTTP 错误 body 截断；超时/大小上限生效；不静默重试
- [X] T026 [US3] 跑通 T023/T024 全部测试（2026-09-23 `make contract-test` 通过）

**Checkpoint**: 全部错误路径显式失败且脱敏；配置隔离有回归测试。

---

## Phase 6: User Story 4 — 在既有契约上逐步扩展 (Priority: P2)

**Goal**: 验证 §9 架构边界落实，七类演进方向设计评审通过（SC-005）

**Independent Test**: `cargo test` 在无 DuckDB 进程、无网络下通过（判断语义层独立）；设计评审对照根规格 §9.2 表 7/7 项确认职责与边界

- [X] T027 [P] [US4] 验证并修正分层依赖：`src/serialize.rs`、`src/judgment.rs`、`src/provider/` 不引用 libduckdb-sys 类型；`src/functions/` 不含 provider 协议细节（可用 `cargo test` + 依赖审查确认）
- [X] T028 [US4] 撰写设计评审记录 `specs/001-judgment-foundation/design-review.md`：对照根规格 §9.2 逐项说明 v0.2 批处理/缓存、v0.3 多服务、v0.4 profiling、v0.5 PG、v0.6 集合判断、v1.0 JDL、后续研究的职责归属、兼容性约束与本期边界（request_key、canonical_version、ProviderContext 等预留点）

**Checkpoint**: SC-005 达成，基础契约不随 provider 切换改变。

---

## Phase 7: Polish & 验收

**Purpose**: 交付物完整、真实服务验收、文档义务

- [X] T029 [P] 编写 `examples/make_sample_data.py`（生成非敏感 tickets.parquet 与 market.parquet）与 `examples/demo.sql`（quickstart.md 离线演示内容）
- [X] T030 [P] 编写 `README.md`：构建加载说明（锁定 DuckDB v1.5.5、macOS arm64、`-unsigned`）、模型判断不确定性、数据外发、调用费用、不承诺结果行数=请求数（FR-010）
- [X] T031 编写 `test/sql/run_live_acceptance.py`：`TYPESAFE_API_KEY` + `DUCKJEU_LIVE_TEST=1` 双条件触发，三函数各 ≥3 行非敏感输入，记录实际 provider/model、结果类型合规率、行数、请求数、耗时；不可用记 blocked 不标完成（SC-004）
- [X] T032 执行 quickstart.md 全流程验证（构建加载、离线 3/3 示例、契约测试全绿、live 验收），输出验收记录：`make demo`、`cargo test`、`make contract-test`（53/53）及 TypeSafe live acceptance 三函数各 3 行均通过；记录见 `build/acceptance/live-20260923T100542.json`（实际模型名未取得，请求数按逐行调用推导）
- [X] T033 [P] unsafe FFI 边界 code review：`src/functions/mod.rs`、`src/config.rs` 中 libduckdb-sys 调用点的生命周期与错误处理（research.md 遗留风险）
- [X] T034 同步根文档：roadmap.md 当前状态更新为 v0.1 实现与验收结果

**Checkpoint**: FR-011 交付物齐备；SC-001~005 全部验收或有明确 blocked 记录。

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: 无依赖，立即开始
- **Foundational (Phase 2)**: 依赖 Phase 1——**阻塞所有 user story**
- **US1/US2/US3 (Phase 3–5)**: 依赖 Phase 2；US1 先行（typesafe adapter 基座），US2 扩展同文件（typesafe.rs），US3 复用前两者的错误路径，建议顺序 US1 → US2 → US3
- **US4 (Phase 6)**: 依赖 US1–US3 代码成形（分层验证对象为最终实现）
- **Polish (Phase 7)**: 依赖全部故事

### Within Each Story

测试任务先写并确认失败 → 实现 → 测试通过 → 进入 checkpoint。

### Parallel Opportunities

- T001–T004 中 T004 与其余并行
- Phase 2：T005/T006/T007/T008 四文件并行；T009/T010 依赖骨架但彼此并行；T011 依赖 T005–T008
- US1：T012 与 T013 并行；US2：T017 与 T018 并行；US3：T023 与 T024 并行
- US4：T027 与 T028 并行；Polish：T029/T030/T033 并行

---

## Parallel Example: User Story 1

```bash
# 先并行写测试（确认失败）：
Task: "编写 test/stub_server.py HTTP stub"
Task: "编写 test/sql/run_contract_tests.py 的 jev_prob 契约测试"

# 再实现（T014 与 T015 不同文件但 T015 调用 T014，可并行起草、顺序联调）：
Task: "实现 src/provider/typesafe.rs noul 请求与校验"
Task: "实现 src/functions/prob.rs 并注册 jev_prob"
```

---

## Implementation Strategy

### MVP First（仅 US1）

1. Phase 1 Setup + Phase 2 Foundational → 基座就绪
2. Phase 3 US1 → mock/stub 下 jev_prob 全测试通过 → 可演示离线 MVP
3. **STOP and VALIDATE**：独立验证 US1 后继续

### Incremental Delivery

US1（prob）→ US2（bool/choice）→ US3（安全与错误）→ US4（架构评审）→ Polish（验收与文档）。每个故事结束于可独立验证的 checkpoint。

---

## Notes

- [P] = 不同文件、无未完成依赖；[USx] 对应 spec.md User Story x
- 真实服务（TypeSafe）请求/响应细节以 contracts/provider-protocol.md 为准；与服务实际行为冲突时更新该文档
- research.md 遗留风险（release+LTO、unsafe 边界、真实 API 错误语义、Linux/Windows）分别由 T003、T033、T031、README 平台声明承接
- 完成每个任务或逻辑组后提交
