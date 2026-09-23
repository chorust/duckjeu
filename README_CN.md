# duckjeu

[English](README.md) | 简体中文

`duckjeu` 是一个 DuckDB 扩展：逐行对表或 Parquet 数据运行 JEV（Judgment-Enabled Vector，模型判断），得到类型化结果，再用普通 SQL 进行分析和决策。

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
| `duckjeu_provider` | VARCHAR | `mock` | `mock` 或 `typesafe`；设为 `typesafe` 会显式开启真实调用 |
| `duckjeu_api_url` | VARCHAR | `https://api.typesafe.ai/v1/systemone` | 服务端点，也可指向本地 stub 测试服务 |
| `duckjeu_model` | VARCHAR | `jev-latest` | 模型标识 |
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
DuckDB 宿主层        src/lib.rs, src/functions/, src/config.rs
  ↓ 函数注册、NULL 处理、chunk 行映射、输出写回、连接配置
判断语义层           src/serialize.rs, src/judgment.rs
  ↓ 类型化状态、稳定编码、请求身份、响应校验、阈值规则（不依赖 DuckDB/网络）
Provider 层          src/provider/{mod,mock,typesafe}.rs
  ↓ 服务协议、认证、能力与响应转换（不写回 DuckDB 向量）
```

判断语义层可在没有数据库进程、没有网络的条件下独立验证（`cargo test`）。后续 roadmap 的批处理与缓存、多 provider、profiling、PostgreSQL 宿主、集合判断和 JDL 都复用这一分层。详细约束见 [spec.md 第 9 节](spec.md#9-面向-roadmap-的架构约束)。

## 平台与限制

- 仅在 macOS arm64 + DuckDB v1.5.5 上验证；Linux/Windows 尚未验证。
- v0.1 逐行同步调用，无缓存、无合批、无并发控制；请在目标环境实测性能。
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
make contract-test   # 使用本地 HTTP stub 的 SQL 契约与错误路径测试（53 项）
```

SQL 契约测试会在项目 venv 中安装 `duckdb==1.5.5`。覆盖文本与 STRUCT 状态、NULL、空数据集、概率边界、布尔阈值、类别成员校验、5000 行跨 DataChunk 行对齐、连接级配置、prepared statement 重跑，以及 provider 错误路径。

真实服务验收需显式开启，会发送真实请求：

```bash
export DUCKJEU_API_KEY=...          # 或 TYPESAFE_API_KEY
DUCKJEU_LIVE_TEST=1 make live-test
```
