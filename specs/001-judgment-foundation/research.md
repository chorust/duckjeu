# Phase 0 Research: DuckJeu v0.1 Judgment Foundation

调研日期：2026-09-22。所有 NEEDS CLARIFICATION 已解决。Rust 绑定关键结论由本机编译并加载的探针扩展验证（`/tmp/duckjeu-plan-probe`，未入库）；provider 协议以官方文档与 reference 源码交叉确认。

## D1: Rust 扩展技术路线

**Decision**: 使用 `duckdb-rs` 1.10505.0（`loadable-extension` + `vscalar` feature）作为扩展骨架，对 duckdb-rs 高层 API 未覆盖的能力直接使用其 `libduckdb-sys` FFI：ANY 类型参数、STRUCT 字段遍历、volatile、special NULL handling、scalar init 回调与 session 配置读取。不引入 C++ 桥接。

**Rationale**: 探针扩展（纯 Rust + libduckdb-sys，约 100 行）在本机编译、打包并以 DuckDB CLI 1.5.5 实际加载，验证了规格要求的全部高风险能力：

- ANY 参数同时接受 `struct_pack(...)`、VARCHAR、NULL（`DUCKDB_TYPE_ANY` 注册成功）；
- STRUCT 子字段遍历、内部 NULL 区分（`struct_pack(a:=2, b:=NULL::INTEGER, c:=3)` 正确跳过 NULL 字段）；
- 5000 行跨 chunk 行映射无错位（DuckDB C API 在执行前 `input.Flatten()`，见 duckdb v1.5.5 `src/main/capi/scalar_function-c.cpp:202`）；
- `duckdb_register_config_option` 注册 session 级配置，scalar init 回调经 `duckdb_scalar_function_init_get_client_context` + `duckdb_client_context_get_config_option` 在**每次执行时**读取当前值：prepared statement 在 `SET` 变更后重跑取得新值（21 → 31）；
- 同一 database 两个连接的配置互不污染（Python duckdb 1.5.5 实测 141 / 199）。

**Alternatives considered**: (a) 仅用 duckdb-rs 高层 `VScalar` trait——不支持 ANY 参数、init 回调与 config option 注册，放弃；(b) 复制 reference 仓库的 C++ 实现——与 spec 指定的 Rust 路线冲突，且两者均无 provider 抽象，放弃；(c) C++ shim + Rust core——增加 FFI 复杂度而无收益，探针证明不需要。

**Source revisions**: duckdb-rs `d2598d3bd052` (main)，extension-template-rs `abb7b2f3f954`，libduckdb-sys crate 1.10505.0，DuckDB 源码 tag v1.5.5。

## D2: 目标版本与打包方式

**Decision**: 锁定 DuckDB v1.5.5（本机 CLI `d8cdaa33fd`），macOS arm64 为首个验证平台。构建用 `cargo build`，打包用 extension-ci-tools 的 `append_extension_metadata.py`（submodule 固定 `a285602bf745`），`--abi-type C_STRUCT_UNSTABLE`（duckdb-rs 依赖 unstable C API，二进制仅保证与目标版本兼容）。本地加载用 `duckdb -unsigned` / `allow_unsigned_extensions`。

**Rationale**: 与 extension-template-rs 的 Makefile 约定一致（`USE_UNSTABLE_C_API=1`、`TARGET_DUCKDB_VERSION=v1.5.5`），探针已按此路径成功加载。spec 要求锁定版本，unstable ABI 与之一致。

**Alternatives considered**: 稳定 C API 打包——duckdb-rs 当前要求 unstable API，不可用；社区扩展仓库分发——属后续发行工作，v0.1 不做。

## D3: 真实 provider 协议（TypeSafe System One）

**Decision**: v0.1 真实 adapter 对接 TypeSafe System One HTTP API：

- `POST {api_url}`，默认 `https://api.typesafe.ai/v1/systemone`，`Authorization: Bearer $TYPESAFE_API_KEY`；
- 请求体 `{model, state, questions}`；`questions` 的 key 由调用方指定，响应 `answers` 按同 key 回显——作为原生请求关联机制；
- v0.1 每请求单行单问题，question key 取 `"r0"`，响应必须恰好包含该 key；未来批量扩展为 `"r{i}"`（recodelabs 已验证该模式）；
- `jev_prob`/`jev_bool` → `noul` 问题（criterion 放入 `state.condition`，instructions 引用 `rows[0]`，与 recodelabs 一致；具体 prompt 文本属实现细节，由 live 验收覆盖）；`jev_choice` → `choice` 问题（`criteria` 为 `{label: null}` 映射，放在 `questions.r0.criteria`）；
- 响应校验：`answers` 中的值为按问题类型标记的对象；`noul` 读取 `{"type":"noul","noul": number}` 中 [0,1] 内有限数，`choice` 读取 `{"type":"choice","choice": label}` 中的候选标签；answers 缺失、多余或类型错误即失败；
- 默认 model `jev-latest`，响应中的实际 `model`（如 `jev-1.13.0`）记入验收记录。

**Rationale**: [TypeSafe 官方 JS SDK wire types](https://github.com/typesafe-ai/typesafe-sdk-js/blob/main/src/types.ts) 定义 `NoulResponse` 为 `{type: "noul", noul: number}`，`ChoiceResponse` 为 `{type: "choice", choice, confidence, probabilities}`；此前项目契约误按裸数字/字符串解析，真实服务验收已证实与当前响应不兼容。recodelabs 源码（固定 revision `378d36ff`）仍用于确认请求格式及其单行组批模式。

**Alternatives considered**: Vercel AI Gateway 代理（2026-09-21 起支持 Jev）——`api_url` 可配置即可兼容，不作为单独 provider；自托管/本地 Jev——v0.3 范围。

## D4: HTTP 客户端与安全限制

**Decision**: 使用 ureq 3.x（blocking，rustls TLS）+ serde_json。超时用 `timeout_global`；响应大小用 body `limit(n)`；不自动重试（spec 明确 v0.1 不静默重试）；HTTP 状态错误显式上抛；错误 body 截断到 300 字符；凭据只放 Authorization header，不出现在 URL、错误信息或日志。

**Rationale**: ureq 已在 duckdb-rs 依赖树中（template 的 Cargo.lock 含 ureq 3.3），无 OpenSSL 依赖；`timeout_global` 与 body `limit()` 在 ureq 3.4.2 源码确认存在。

**Alternatives considered**: reqwest+tokio——在 DuckDB 标量函数回调内引入 async runtime 复杂度过高；curl 子进程——不可嵌入、错误语义差。

## D5: 配置与凭据

**Decision**: session 级自定义配置项（`duckdb_register_config_option`，探针已验证）：

| 配置项 | 类型 | 默认 | 含义 |
| --- | --- | --- | --- |
| `duckjeu_provider` | VARCHAR | `mock` | `mock` 或 `typesafe`；设为 `typesafe` 即显式开启真实调用 |
| `duckjeu_api_url` | VARCHAR | 官方端点 | endpoint（可指向本地 stub 做测试） |
| `duckjeu_model` | VARCHAR | `jev-latest` | 模型标识 |
| `duckjeu_timeout_ms` | BIGINT | 30000 | 单次请求总超时 |
| `duckjeu_max_response_bytes` | BIGINT | 1048576 | 响应体上限 |

凭据仅从环境变量 `TYPESAFE_API_KEY` 读取（`DUCKJEU_API_KEY` 作为覆盖项），不提供 SQL 设置凭据的途径。配置在 scalar init 回调中按执行读取并封装为不可变的 ProviderContext 传入执行路径，连接间互不共享。

**Rationale**: 满足 FR-009（连接隔离、显式开启、凭据不入 SQL/日志）；探针验证了 per-execution 读取与连接隔离。

**Alternatives considered**: DuckDB secret manager（colliber 方案）——需额外 C API 表面与 `CREATE SECRET` 语法，v0.1 用环境变量已满足约束，留作后续增强；全局静态配置——违反连接隔离，否决。

## D6: Canonical state 编码

**Decision**: 自实现 canonical JSON（不依赖 DuckDB `to_json`）：带类型标签、字段按名称排序、区分 SQL NULL 与缺失字段、拒绝非有限数值；VARCHAR state 作为原始文本不经 JSON 包装。编码版本 `canonical_version=1` 显式保留，供 v0.2 缓存 key 使用。v0.1 支持 BOOLEAN、各宽度整数、FLOAT/DOUBLE（有限值）、VARCHAR、NULL；DATE/TIMESTAMP/嵌套 LIST/STRUCT 等不支持类型在读取时明确报错。

**Rationale**: spec 要求稳定类型化编码，reference 仓库的 `to_json` 桥接不区分 NULL/缺失且格式随 DuckDB 版本变化；canonical bytes 同时是未来去重/缓存的完整比较内容（不能只信 hash）。

**Alternatives considered**: Arrow IPC 编码——跨层开销大且可读性差；复用 recodelabs `to_json(rec)`——不满足 spec。

## D7: Mock 与测试策略

**Decision**: 三层测试：

1. `cargo test`：canonical 编码、请求/响应校验、mock provider、布尔阈值、choice membership 等纯 Rust 契约；
2. SQL 集成测试：Python duckdb 1.5.5 驱动加载扩展，配合本地 HTTP stub（Python `http.server`，脚本化响应：正常、越界概率、未知 label、缺失/多余 answer、401、超时、超大响应）覆盖全部 SQL 错误路径与行对齐；
3. Live 验收：`TYPESAFE_API_KEY` + `DUCKJEU_LIVE_TEST=1` 双条件触发，三种函数各至少三行非敏感输入，记录实际 model、行数、请求数与耗时。

in-process mock provider 确定性返回（由 canonical state + 问题内容的 hash 派生 [0,1] 值、按 hash 选 choice label），供零依赖 demo 与单元测试；脚本化精确值测试一律走 HTTP stub。

**Rationale**: 与两个 reference 仓库的分层一致（mock API + live test 分离），但补上它们缺失的响应严格校验用例。SQL 层测试必须走真实 HTTP adapter 路径才能覆盖超时/大小限制等行为，in-process mock 无法替代。

**Alternatives considered**: 仅用 in-process mock——无法测试 HTTP 错误与限制；sqllogictest——对脚本化 stub 响应表达力不足，作为补充而非主力。

## D8: 执行与错误语义

**Decision**: 三个函数注册为 volatile + special NULL handling（自行处理 NULL：顶层 NULL → NULL 且不调用）。v0.1 逐行同步调用，每次调用构造带唯一 question key 的单问题请求。任何输入校验、协议、超时、鉴权或响应校验失败都经 `duckdb_scalar_function_set_error` 使查询报错，错误信息脱敏。`jev_bool` 与 `jev_prob` 走同一 noul 请求路径，各自独立调用（v0.1 无缓存，不承诺共享请求）。

**Rationale**: 直接落实 spec §3 与 FR-003/006/007/010；reference 仓库的历史修复（hash 冲突、worker 异常、线程回收）均已转化为 v0.1 的校验要求与 v0.2 的验收清单。

## 遗留风险（移交实现阶段验证）

- duckdb-rs 高层 API 与直接 FFI 混用的 unsafe 边界需 code review；探针只覆盖 INTEGER/STRUCT/VARCHAR，全类型矩阵由 cargo test 覆盖。
- 探针为 debug 构建；release + LTO 的加载与符号行为需在验收时确认。
- 真实 API 的限流/错误码表未在文档中穷尽，live 验收需记录实际错误语义；超时与大小上限的默认值可能在 live 验收后调整。
- Windows/Linux 构建未验证，README 只声明已验证平台。
