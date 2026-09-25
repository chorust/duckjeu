# duckjeu

[English](README.md) | 简体中文

`duckjeu` 是一个 DuckDB 扩展：对表或 Parquet 数据运行 JEV（Judgment-Enabled Vector，模型判断），得到类型化结果，再用普通 SQL 进行分析和决策。默认逐行执行，也可显式开启优化模式。

```sql
SELECT jev_bool(message, 'the customer requests a refund') FROM tickets;

SELECT jev_choice(
           struct_pack(price := price, load := load, wind := wind),
           'which market regime best describes this state?',
           ['normal', 'scarcity', 'oversupply']
       ) AS regime
FROM read_parquet('market.parquet');
```

> **模型判断不是已验证事实。** v0.1 不保证业务校准。请把结果当作模型信号，根据使用场景设置门槛；需要明确阈值时使用 `jev_prob`。
>
> **数据外发与费用：** 设置 `duckjeu_provider='typesafe'` 后，状态、criterion/question 和选项标签会发送到配置的服务，真实调用可能产生费用。DuckDB 可能重复执行表达式，查询失败也不会撤销已经发生的请求。先用 SQL 过滤候选行；未经授权不要发送敏感数据。

## 公开 API（v0.1）

| 函数 | 参数 | 返回 | 契约 |
| --- | --- | --- | --- |
| `jev_prob(state, criterion)` | STRUCT 或 VARCHAR 状态；非空 VARCHAR criterion | DOUBLE / NULL | 二元 criterion 成立的概率，范围为 [0,1] |
| `jev_bool(state, criterion)` | 同上 | BOOLEAN / NULL | 对二元概率按 `p >= 0.5`（含边界）转换；每次独立调用，不与 `jev_prob` 共享请求 |
| `jev_choice(state, question, choices)` | 状态、非空 VARCHAR 问题、至少两个不重复的 VARCHAR 选项 | VARCHAR / NULL | 根据 `question` 返回 `choices` 中的单个标签 |

- **STRUCT 状态**会稳定规范化：字段按名称排序、带类型标签，并区分 SQL NULL 与缺失字段。支持 BOOLEAN、整数、DECIMAL、HUGEINT、有限 FLOAT/DOUBLE、VARCHAR 和 NULL。DATE/TIMESTAMP、嵌套 LIST/STRUCT、BLOB 等不支持类型会明确报错。
- **VARCHAR 状态**按原始文本发送，不做包装。
- 顶层 **NULL**（`state`、`criterion`、`question` 或 `choices`）返回 NULL 且不调用 provider；STRUCT 内部 NULL 字段按 NULL 保留。
- 空数据集不产生请求。

## 安装和配置

要求：macOS arm64（首个验证平台）、Rust ≥ 1.85、DuckDB CLI **v1.5.5**。Python 3 用于 demo 和测试目标。

初始化打包工具并编译扩展：

```bash
git submodule update --init          # extension-ci-tools 打包脚本
make release
```

扩展产物为 `build/release/duckjeu.duckdb_extension`。v0.1 未签名，加载时需允许 unsigned 扩展：

```bash
duckdb -unsigned -cmd "LOAD '$PWD/build/release/duckjeu.duckdb_extension'"
```

macOS hardened runtime 要求 `LOAD` 使用绝对路径。也可将扩展复制到 `~/.duckdb/extensions/v1.5.5/osx_arm64/` 后运行 `LOAD duckjeu`。

`duckdb-rs` 使用 unstable C API，扩展采用 `C_STRUCT_UNSTABLE` ABI，**只保证与 DuckDB v1.5.5 兼容**。

以下配置为连接级配置，不同连接互不共享，且每次执行都会读取当前值：

| 配置项 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `duckjeu_provider` | VARCHAR | `mock` | `mock`、`typesafe` 或 `selfhosted`；后两者会显式开启服务调用 |
| `duckjeu_api_url` | VARCHAR | `https://api.typesafe.ai/v1/systemone` | 服务端点，也可指向本地 stub 测试服务 |
| `duckjeu_local_jev_api_url` | VARCHAR | `http://127.0.0.1:8765/v1/systemone` | `selfhosted` 的默认端点；可由 `duckjeu_api_url` 覆盖 |
| `duckjeu_model` | VARCHAR | 空值；按 provider 解析有效模型 | 模型标识；所有 provider 都保留显式设置的值 |
| `duckjeu_timeout_ms` | BIGINT | `30000` | 单次请求总超时 |
| `duckjeu_max_response_bytes` | BIGINT | `1048576` | 响应体大小上限 |

凭据在启动 DuckDB 前从环境变量读取，不能通过 SQL 设置，也不会写入错误或日志。优先使用 `DUCKJEU_API_KEY`，否则读取 `TYPESAFE_API_KEY`：

```bash
export DUCKJEU_API_KEY=...          # 或 export TYPESAFE_API_KEY=...
```

## 使用方式

加载扩展后，显式启用真实 provider：

```sql
SET duckjeu_provider = 'typesafe';
SELECT jev_prob(message, 'the customer requests a refund') FROM tickets;
```

### 真实服务 CLI 展示

[真实演示 SQL](examples/live_demo.sql) 使用每个函数 3 行非敏感样例调用真实 provider，并展示顶层 NULL 行为。最多发起 9 次判断请求；provider 会按量计费，DuckDB 也可能重复执行表达式。

在仓库根目录设置 API key 后运行：

```bash
export DUCKJEU_API_KEY=...          # 或 TYPESAFE_API_KEY
make release venv
.venv/bin/python examples/make_sample_data.py
mkdir -p build
sed "s|@@EXTENSION@@|$PWD/build/release/duckjeu.duckdb_extension|" examples/live_demo.sql > build/live_demo.sql
duckdb -unsigned -box -c ".read build/live_demo.sql"
```

一次真实服务调用得到以下输出；后续运行的概率和标签可能不同：

```text
┌───────┬────────────────────┐
│  id   │ refund_probability │
│ int32 │       double       │
├───────┼────────────────────┤
│     1 │               0.98 │
│     2 │               0.01 │
│     3 │               0.04 │
└───────┴────────────────────┘
┌───────┬─────────────┐
│  id   │ refund_like │
│ int32 │   boolean   │
├───────┼─────────────┤
│     1 │ true        │
│     2 │ false       │
│     3 │ false       │
└───────┴─────────────┘
┌────────┬────────────┐
│ price  │   regime   │
│ double │  varchar   │
├────────┼────────────┤
│  95.75 │ oversupply │
│  98.25 │ oversupply │
│  101.5 │ normal     │
└────────┴────────────┘
┌────────────┬──────────────┐
│ null_state │ null_choices │
│   double   │   varchar    │
├────────────┼──────────────┤
│       NULL │ NULL         │
└────────────┴──────────────┘
```

超时、鉴权失败、HTTP 错误、响应缺失或多余、类型错误、越界概率、未知标签、provider 不支持相关能力都会使查询报错。v0.1 不会静默重试，也不会把失败转成 0、false 或空标签。HTTP 错误只显示状态码和固定提示，不包含凭据、原始状态或远端响应正文。

## 架构

```
DuckDB 宿主层        src/lib.rs, src/functions/, src/host/, src/config.rs
  ↓ 函数注册、向量读写、查询生命周期、连接配置
共享判断核心         crates/judgment-core/src/
  ↓ 状态编码、判断身份、校验、执行、缓存、概况聚合（不依赖 DuckDB）
Provider 适配层      src/provider/{mock,typesafe,localjev}.rs
  ↓ 服务协议、认证、能力与响应转换
```

共享核心可在没有数据库进程、没有网络的条件下独立验证（`cargo test`）。批处理、缓存、多 provider 和 profiling 已按此分层实现；PostgreSQL 宿主、集合判断和 JDL 仍属后续里程碑。详细约束见 [spec.md 第 9 节](spec.md#9-面向-roadmap-的架构约束)。

## v0.2 执行优化

连接级优化使用 DuckDB 内部 `ClientContext` 桥接，必须针对准确的 DuckDB v1.5.5 源码构建：

```bash
export DUCKDB_SOURCE_DIR=/path/to/duckdb-v1.5.5
export DUCKDB_SOURCE_VERSION=v1.5.5
make release
```

`duckjeu_execution_mode` 默认是 `row`。设为 `optimized` 后，同一查询内的相同判断会去重并保留原始行映射。`duckjeu_max_inflight` 默认 4。显式合批配置为 `duckjeu_batch_enabled`（默认 `false`）、`duckjeu_batch_max_judgments`（16）和 `duckjeu_max_request_bytes`（262144）；请求字节上限只在 `optimized` 模式生效。TypeSafe 跨状态合批已在响应实际模型为 `jev-1.13.0` 时通过真实 Noul/Choice 与 DuckJeu SQL 验收；请求别名 `jev-latest` 仅在仍解析为该模型时有效。其他或漂移的模型会失败关闭。合批仍需显式开启。复合状态会让每个问题看到其他行，输出可能不同于单行上下文；见[能力验收证据](specs/002-judgment-runtime-evolution/evidence/batch-capability.md)。

**已知取消限制：** 当前到查询清理阶段才会把中断传给等待请求额度的任务。逐行查询在慢速 provider 调用期间被中断后，仍可能继续发送该数据块剩余行的请求才停止；已发送的请求无法撤销。

跨查询缓存默认关闭。仅在 `optimized` 模式下将 `duckjeu_cache_enabled=true` 才启用；默认限制为 10,000 条、估算内存 64 MiB、TTL 60,000 ms。缓存按连接和凭据作用域隔离。`duckjeu_cache_clear()` 只清理当前连接，并返回清除的条目数。TypeSafe 缓存要求使用 `jev-1.13.0` 这类版本化模型标识，且响应报告的模型一致。`jev-latest` 等可变别名不会跨查询复用。缓存仅保存在内存，只存放完整校验成功的结果。

`selfhosted` provider 适配 local-jev 的 `/v1/systemone` 协议。未显式设置 `duckjeu_model` 时，本地默认模型为 `nli-deberta-large`；TypeSafe 和 mock 默认使用 `jev-latest`。显式设置的模型值（包括 `jev-latest`）会原样发送。可选凭据只从 `DUCKJEU_LOCAL_JEV_API_KEY` 读取；截断响应和超过 255 个候选会失败。模型权重 revision 未知时禁用跨查询缓存；跨状态合批在单独验证前标为不支持。local-jev 服务需自行启动，DuckJeu 不会下载权重或启动服务。

`duckjeu_profile_enabled` 默认 `false`。判断查询前设为 `true` 可记录序列化时间和客户端往返时间；之后执行 `SELECT duckjeu_last_profile()` 读取当前连接最近一次已结束的判断查询，包括失败或取消的查询。版本化 JSON 包含聚合计数、耗时、用量和费用估计，不包含状态、问题、候选、端点、凭据或 provider 错误正文。缺失用量、可能计费的失败请求及不支持的服务端计时均以 `null` 和原因表示。独立的本地 stub benchmark 将费用记为未知。

**已知计价限制：** 当前只要 provider 配置为 `typesafe` 且用量完整，profile 就会套用公开的 TypeSafe 输入 token 费率，未核查端点或模型的计价。若 `duckjeu_api_url` 指向其他服务或 stub，返回的 USD 金额没有可靠计价依据；在核实来源前应视为未验证。预期规则见[概况契约](specs/002-judgment-runtime-evolution/contracts/profile-report.md)。

离线 runtime 契约和固定规模 benchmark 只访问本地脚本服务：

```bash
make runtime-contract
make profile-contract
make bench
```

TypeSafe 直接协议探针和经 DuckJeu 扩展的低流量 SQL 探针分别有显式 gate；后者对四个合成判断只发两个请求：

```bash
DUCKJEU_BATCH_LIVE_TEST=1 make batch-live
DUCKJEU_BATCH_EXTENSION_LIVE_TEST=1 make batch-extension-live
```

以上两项均于 2026-09-24 通过：直接协议探针共 8 次 HTTP；DuckJeu 扩展 SQL 探针以 2 次请求完成 4 个判断，并核对结果映射和 profile 计数。首次运行遇到 Python CA 信任配置问题，安装/配置 `certifi` 后通过；首次尝试没有取得判断响应。详情见[探针证据](specs/002-judgment-runtime-evolution/evidence/batch-capability.md)。

local-jev 需在本机另行启动；只有显式设置 `DUCKJEU_LOCAL_JEV_TEST=1` 才会由 `make local-live` 发送验收请求。

当前状态见 [roadmap](roadmap.md) 和 [Spec 002 evidence](specs/002-judgment-runtime-evolution/evidence/)。既有 v0.2–v0.4 验收记录覆盖所述环境，但 v0.2 取消传播和 v0.4 计价来源仍待修复、复验。local-jev 真实 1K/10K/100K 对照通过；实际加载权重 revision 未知，因此关闭跨查询缓存；跨状态合批不支持，费用未知；TypeSafe 多规模性能尚未测试。

## 平台与限制

- 仅在 macOS arm64 + DuckDB v1.5.5 上验证；Linux/Windows 尚未验证。
- TypeSafe 多状态合批仅对实际模型 `jev-1.13.0` 验证通过；selfhosted 已用 `nli-deberta-large` 完成 live SQL 验收，跨状态合批仍不支持。
- `jev_bool` 的 0.5 阈值是固定契约，不代表业务校准。
- 详见 [provider 协议](specs/001-judgment-foundation/contracts/provider-protocol.md)和 [SQL 契约](specs/001-judgment-foundation/contracts/sql-api.md)。

## 开发与测试

离线 `make demo` 使用 mock provider，不访问外部服务；必要时会创建样例 Parquet 和项目 venv：

```bash
make demo
```

运行 Rust 和 SQL 契约套件：

```bash
make test            # Rust 契约测试，无需数据库或网络
make contract-test   # 使用本地 HTTP stub 的 v0.1 SQL 契约与错误路径测试（64 项）
make runtime-contract # 优化执行、合批前置校验、缓存和连接契约（本地 stub）
make profile-contract # 脱敏查询概况 SQL 契约（本地 stub）
```

SQL 契约测试会在项目 venv 中安装 `duckdb==1.5.5`。覆盖文本与 STRUCT 状态、NULL、空数据集、概率边界、布尔阈值、类别成员校验、5000 行跨 DataChunk 行对齐、连接级配置、prepared statement 重跑，以及 provider 错误路径。runtime 套件只访问本地 stub，覆盖去重、显式合批能力检查、有界缓存、凭据轮换和连接隔离。

真实服务验收需显式开启，会发送真实请求：

```bash
export DUCKJEU_API_KEY=...          # 或 TYPESAFE_API_KEY
DUCKJEU_LIVE_TEST=1 make live-test
```
