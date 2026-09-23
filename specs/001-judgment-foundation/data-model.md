# Phase 1 Data Model: v0.1 Judgment Foundation

v0.1 无持久化存储；以下均为执行期内部实体。所有实体归属判断语义层或 provider 层，不携带 DuckDB 向量/连接类型（根规格 §9.1）。

## CanonicalState（serialize.rs）

输入行的规范化表示，同时是 v0.2 缓存 key 的完整比较内容。

- 来源：`STRUCT`（任意同构字段集）或 `VARCHAR`（原始文本，不包装）。
- STRUCT 编码：`canonical_version=1`；字段按名称排序；每字段带类型标签；区分 SQL NULL 字段与缺失字段；值域限定 BOOLEAN / 各宽度整数 / 有限 FLOAT/DOUBLE / VARCHAR / NULL。
- 拒绝：DATE/TIMESTAMP、嵌套 LIST/STRUCT、非有限浮点（读取时显式报错，FR-005）。
- VARCHAR：原文保留；空字符串是合法值，与 SQL NULL 区分。

## JudgmentRequest（judgment.rs）

一次对外请求的完整语义内容。

| 字段 | 说明 |
| --- | --- |
| `state` | CanonicalState |
| `kind` | `Noul`（jev_prob / jev_bool 共用）或 `Choice` |
| `criterion` / `question` | 非空文本（Noul 用 criterion；Choice 用 question）；构造前校验，空白即报错 |
| `choices` | Choice 专用：≥2 个、不重复、非空的 label 列表（原样，不修剪） |
| `request_key` | 请求关联身份；v0.1 恒为 `"r0"`（单问题请求），v0.2 扩展 `"r{i}"` |

生命周期：`created → sent → validated | failed`。`failed` 使 SQL 查询报错（错误脱敏）；不撤销已发生的外部请求。

## JudgmentResult（judgment.rs）

经校验的返回值，尚未映射到 SQL 类型。

- `Noul`：有限浮点且 ∈ [0,1]，越界/缺失/类型错即 `failed`。
- `Choice`：返回 label 必须属于 `choices` 精确成员（不解释顺序或分数）。
- `jev_bool` 转换：同一 Noul 结果按 `p >= 0.5`（含边界）取真，是独立的请求，不承诺与另一次 `jev_prob` 共享网络调用。

## ProviderContext（config.rs + provider/mod.rs）

scalar init 回调中按**每次执行**读取并冻结的配置快照，连接间隔离：

| 字段 | 来源 | 默认 |
| --- | --- | --- |
| `provider` | `duckjeu_provider` | `mock`；`typesafe` 为显式开启真实调用 |
| `api_url` | `duckjeu_api_url` | `https://api.typesafe.ai/v1/systemone` |
| `model` | `duckjeu_model` | `jev-latest` |
| `timeout_ms` | `duckjeu_timeout_ms` | 30000 |
| `max_response_bytes` | `duckjeu_max_response_bytes` | 1048576 |
| `credential` | 环境变量 `TYPESAFE_API_KEY`（`DUCKJEU_API_KEY` 覆盖） | 无；缺失且 provider=typesafe 时报错 |

凭据只存在于内存上下文与 Authorization header，不出现在 URL、错误信息、日志或任何可输出身份中。

## AcceptanceEvidence（examples/ + test/ 产物）

验收记录（FR-011、SC-004），不含凭据与敏感 state：

- 环境：OS/arch、Rust 版本、DuckDB 版本（锁定 v1.5.5）。
- 每次演示：provider、请求模型与实际响应 model（如 `jev-1.13.0`）、函数、输入行数、实际请求数、各请求耗时与总耗时、结果类型合规率。
- 真实服务不可用时记 `blocked`，不得标为完成。

## 行映射关系

`输入行 i → JudgmentRequest(request_key) → JudgmentResult → 输出向量位置 i`。顶层 NULL（state/criterion/question/choices）短路为输出 NULL 且不构造请求；空数据集零请求。混合 NULL/重复状态的样例必须跨 ≥1 个 DataChunk 边界验证对齐（根规格 §10）。
