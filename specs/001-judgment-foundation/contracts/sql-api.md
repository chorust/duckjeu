# Contract: SQL API（duckjeu 扩展 v0.1）

来源：根规格 [../../spec.md](../../spec.md) §2/§3 与特性规格 [../spec.md](../spec.md)。参数顺序与既有示例为固定契约。

## 标量函数

三个函数均注册为 **volatile**（模型调用有成本且可能非确定）+ **special NULL handling**（扩展自行处理 NULL）。

### `jev_prob(state ANY, criterion VARCHAR) → DOUBLE`

provider 对二元 criterion 成立的概率。返回保证：有限值且 ∈ [0,1]，否则查询报错。

### `jev_bool(state ANY, criterion VARCHAR) → BOOLEAN`

同一二元概率按 `p >= 0.5`（含边界）转布尔。业务门槛请用 `jev_prob` 显式比较。v0.1 为独立调用，不承诺与 `jev_prob` 共享请求。

### `jev_choice(state ANY, question VARCHAR, choices VARCHAR[]) → VARCHAR`

返回 `choices` 中恰好一个 label（精确成员判断）。不把选项顺序或分数解释为置信度。

## state 编码

- `STRUCT` → CanonicalState（canonical_version=1，字段按名称排序、带类型标签、区分 NULL/缺失）；支持 BOOLEAN、整数、有限浮点、VARCHAR、NULL；DATE/TIMESTAMP/嵌套 LIST/STRUCT/非有限浮点 → 查询报错。
- `VARCHAR` → 原始文本（空字符串合法）。
- 其他类型 → 不支持，显式报错。

## NULL 与空输入契约

| 输入 | 行为 |
| --- | --- |
| 顶层 state / criterion / question / choices 为 SQL NULL | 结果 NULL，**不调用 provider** |
| choices 内部含 NULL、重复、<2 项 | 查询报错 |
| criterion / question 为空或仅空白 | 查询报错（提交该行前） |
| STRUCT 内部 NULL 字段 | 按 NULL 保留 |
| 空数据集 | 零请求 |

## 错误契约

超时、鉴权失败、HTTP 错误、响应缺失/多余/类型错/值越界/未知 label、provider 不支持的能力 → SQL 查询失败，错误信息脱敏（不含凭据、不含原始敏感 state、不含远端响应正文；只保留状态码与固定提示）。不静默重试、不伪造默认结果。

## Session 配置（`duckdb_register_config_option`，连接级隔离）

| 配置项 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `duckjeu_provider` | VARCHAR | `mock` | `mock` / `typesafe`（显式开启真实调用） |
| `duckjeu_api_url` | VARCHAR | `https://api.typesafe.ai/v1/systemone` | 可指向本地 stub |
| `duckjeu_model` | VARCHAR | `jev-latest` | 模型标识 |
| `duckjeu_timeout_ms` | BIGINT | 30000 | 单次请求总超时 |
| `duckjeu_max_response_bytes` | BIGINT | 1048576 | 响应体上限 |

配置在每次执行时读取（prepared statement 在 `SET` 后重跑取得新值）；同库不同连接互不影响。凭据仅从环境变量 `TYPESAFE_API_KEY`（或覆盖项 `DUCKJEU_API_KEY`）读取，**无 SQL 设置途径**。

## 示例

```sql
LOAD 'build/release/duckjeu.duckdb_extension';

-- 离线 mock（默认）
SELECT jev_bool(message, 'the customer requests a refund') FROM tickets;

-- 真实调用（显式开启）
SET duckjeu_provider = 'typesafe';
SELECT jev_choice(
    struct_pack(price := price, load := load, wind := wind),
    'which market regime best describes this state?',
    ['normal', 'scarcity', 'oversupply']
) AS regime
FROM read_parquet('examples/data/market.parquet');
```

## 文档义务（FR-010）

README 必须声明：模型判断存在不确定性、数据会外发至第三方、调用产生费用、DuckDB 可能优化或重复执行表达式（不承诺“结果行数 = 请求数”）。
