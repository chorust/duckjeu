# Implementation Plan: v0.1 Judgment Foundation（DuckDB Rust 扩展）

**Branch**: `001-judgment-foundation`（spec-kit 分支标识；实际 git 分支保持 `main`） | **Date**: 2026-09-22 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/001-judgment-foundation/spec.md`（对应根规格 [../../spec.md](../../spec.md)）

## Summary

交付 DuckJeu v0.1：一个 Rust 实现的 DuckDB loadable extension，提供 `jev_prob(state, criterion)`、`jev_bool(state, criterion)`、`jev_choice(state, question, choices)` 三个 SQL 标量函数，对表或 Parquet 数据的逐行结构化/文本状态取得类型化判断结果。技术路线经本机探针扩展验证（见 [research.md](research.md)）：duckdb-rs 1.10505.0 骨架 + libduckdb-sys FFI 补齐 ANY 参数、STRUCT 遍历、session 配置等能力；确定性 mock provider + TypeSafe System One 真实 adapter；自实现 canonical state 编码；session 级配置 + 环境变量凭据；v0.1 逐行同步调用，保留 request_id/行位置契约以支撑 v0.2 批处理。

## Technical Context

**Language/Version**: Rust（edition 2021，rust ≥ 1.85，本机 1.92.0 已验证）；目标 DuckDB 锁定 v1.5.5

**Primary Dependencies**: duckdb-rs 1.10505.0（features `loadable-extension,vscalar`）+ libduckdb-sys FFI；ureq 3.x（blocking + rustls）；serde_json。不引入 C++ shim、async runtime、OpenSSL。

**Storage**: N/A（扩展本身无持久状态；v0.1 无缓存）

**Testing**: 三层——`cargo test`（纯 Rust 契约）；Python duckdb 1.5.5 驱动 + 本地脚本化 HTTP stub 的 SQL 集成测试；`TYPESAFE_API_KEY` + `DUCKJEU_LIVE_TEST=1` 双条件触发的 live 验收。

**Target Platform**: macOS arm64（首个验证平台）；Linux/Windows 构建不承诺，README 只声明已验证平台。

**Project Type**: 数据库扩展（DuckDB loadable extension，cdylib + extension-ci-tools 元数据打包，`--abi-type C_STRUCT_UNSTABLE`）

**Performance Goals**: 非本期目标；v0.1 逐行同步调用，不承诺吞吐。验收仅记录实际请求数与耗时（FR-011）。1K/10K/100K 基准属 v0.2。

**Constraints**: 单次请求默认超时 30000ms、响应体上限 1MiB；不静默重试；凭据仅环境变量；错误脱敏；函数 volatile + special NULL handling；真实调用需 `SET duckjeu_provider='typesafe'` 显式开启。

**Scale/Scope**: 3 个 SQL 函数、2 个 provider adapter、5 个 session 配置项、~6 个源码模块；测试覆盖全部 spec 错误路径与 ≥1 个跨 DataChunk 边界的行对齐样例。

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

`.specify/memory/constitution.md` 仍为未填充模板，无自定义 gates。以根规格 [spec.md 第 9 节](../../spec.md) 的架构约束作为实际 gate，逐项核对：

- **宿主层边界**：`src/functions/` 只做注册/NULL/chunk 映射/写回；DuckDB 向量与错误类型不进入判断语义层。✅
- **判断语义层独立**：`src/judgment.rs` + `src/serialize.rs` 无 DuckDB/网络依赖，`cargo test` 可独立运行。✅
- **Provider 层单一职责**：`src/provider/` trait + mock + typesafe adapter，只发请求与校验响应，不写回向量。✅
- **执行层**：行位置 ↔ question key（`"r0"`，v0.2 扩展为 `"r{i}"`）的关联保留在请求构造路径。✅
- **配置与凭据**：scalar init 回调按执行读取 session 配置，封装为不可变 ProviderContext；无全局可变配置；凭据不入 SQL/日志。✅
- **契约验证分层**：三层测试不把真实网络作为前置。✅
- **v0.1 边界**：无缓存、无合批、无多宿主、无 JDL；research.md 遗留风险已列明。✅

无违规，无需 Complexity Tracking。

## Project Structure

### Documentation (this feature)

```text
specs/001-judgment-foundation/
├── plan.md              # 本文件
├── research.md          # Phase 0：D1–D8 决策与验证证据
├── data-model.md        # Phase 1：内部实体与生命周期
├── quickstart.md        # Phase 1：构建、加载、离线/真实演示
├── contracts/
│   ├── sql-api.md           # SQL 函数与配置契约
│   └── provider-protocol.md # TypeSafe 协议子集与 mock 语义
└── checklists/
    └── requirements.md  # 规格质量检查（16/16 通过）
```

### Source Code (repository root)

```text
src/
├── lib.rs             # 扩展入口；注册三个函数 + duckjeu_* 配置项
├── functions/
│   ├── mod.rs         # 共享：ANY 参数读取、special NULL、行映射、错误写回
│   ├── prob.rs        # jev_prob → DOUBLE
│   ├── bool.rs        # jev_bool → BOOLEAN（同一 noul 路径，p>=0.5）
│   └── choice.rs      # jev_choice → VARCHAR
├── serialize.rs       # CanonicalState：类型标签、字段排序、NULL/缺失区分（judgment 层）
├── judgment.rs        # JudgmentRequest/Result、响应校验、阈值规则（judgment 层）
├── provider/
│   ├── mod.rs         # Provider trait 与 ProviderContext
│   ├── mock.rs        # 确定性 mock（hash 派生）
│   └── typesafe.rs    # ureq HTTP adapter + 协议校验
└── config.rs          # duckjeu_* 配置读取；TYPESAFE_API_KEY 环境变量
extension-ci-tools/    # git submodule（固定 a285602bf745，打包元数据脚本）
examples/              # 非敏感样例 Parquet 生成脚本 + demo SQL
test/
├── sql/               # Python duckdb 驱动的契约/错误路径 SQL 测试
└── stub_server.py     # 脚本化 HTTP stub（正常/越界/未知 label/缺失/401/超时/超大）
Cargo.toml             # cdylib；duckdb ~1.10505.0, features loadable-extension,vscalar
Makefile               # build（cargo build --release + append_extension_metadata.py）
README.md              # 构建加载说明、不确定性/外发/费用声明（FR-010）
```

**Structure Decision**: 单 crate cdylib。模块划分直接落实根规格 §4 与 §9.1：宿主层（`lib.rs`、`functions/`、`config.rs`）、判断语义层（`serialize.rs`、`judgment.rs`，零 DuckDB/网络依赖）、provider 层（`provider/`）。判断语义层可作为纯 Rust 代码被 `cargo test` 独立验证，未来 v0.5 换宿主时整层复用。

## Complexity Tracking

无（Constitution Check 无违规）。
