# Contract: DuckJeu SQL Runtime（v0.2–v0.4 计划）

三种公开判断函数及其参数、返回类型、NULL 和错误规则保持 [v0.1 SQL 契约](../../001-judgment-foundation/contracts/sql-api.md) 不变。以下是新增的连接级设置和辅助函数，实施前以锁定的 DuckDB 版本验证注册及生命周期。

## 执行模式与限制（v0.2）

| 设置 | 类型 | 默认 | 约束与语义 |
| --- | --- | --- | --- |
| `duckjeu_execution_mode` | VARCHAR | `row` | `row` 保留逐行行为；`optimized` 启用同次执行去重及受控并发 |
| `duckjeu_batch_enabled` | BOOLEAN | `false` | 仅 `optimized` 可启用；服务未验证跨状态合批时明确报错 |
| `duckjeu_batch_max_judgments` | BIGINT | `16` | 每批最多判断数，必须 >0；还受服务协议/请求大小限制 |
| `duckjeu_max_request_bytes` | BIGINT | `262144` | 单次请求体大小上限，必须 >0；不等同服务 token 上限 |
| `duckjeu_max_inflight` | BIGINT | `4` | 当前连接所有数据块共享的同时进行外部请求上限，必须 >0 |
| `duckjeu_cache_enabled` | BOOLEAN | `false` | 仅 `optimized` 可启用跨查询缓存；默认不复用旧判断 |
| `duckjeu_cache_max_entries` | BIGINT | `10000` | 当前连接条目数上限，必须 >0 |
| `duckjeu_cache_max_bytes` | BIGINT | `67108864` | 当前连接缓存字节上限，必须 >0 |
| `duckjeu_cache_ttl_ms` | BIGINT | `60000` | 正的有效期；不保证模型别名稳定 |

`duckjeu_max_request_bytes` 只限制 `optimized` 执行产生的请求；`row` 模式保留原有输入大小行为。`duckjeu_model` 未显式设置时，TypeSafe/mock 使用 `jev-latest`，selfhosted 使用 `nli-deberta-large`；显式设置的模型标识始终原样保留。

`row` 模式忽略优化选项时应显式提示配置冲突，避免用户误以为缓存或合批已生效。执行开始时冻结配置；prepared statement 在 `SET` 后重跑读取新值。无效限制在外部请求前报错。优化不会自动重试或把错误转为默认值。

`SELECT duckjeu_cache_clear();` 返回当前连接清除的条目数（BIGINT）。它是有副作用的辅助函数，应单独执行；不会清除其他连接。连接关闭时缓存与额度一起释放。

## 服务选择（v0.3）

`duckjeu_provider` 保留 `mock` 和 `typesafe`，增加 `selfhosted`，需显式设置。现有 `duckjeu_api_url`、`duckjeu_model`、超时和响应上限仍为连接级。新服务的认证按其已验证协议接入，凭据不得出现在 SQL 设置、查询、错误或指标中；若认证机制与 TypeSafe 不同，新增独立受保护凭据来源并在 provider 契约中声明。

`selfhosted` 首期对接本机回环地址上的 local-jev。服务如通过 `LOCAL_JEV_API_KEY` 启用 Bearer 认证，扩展从独立的 `DUCKJEU_LOCAL_JEV_API_KEY` 环境变量读取同一密钥；不复用 TypeSafe 凭据。`selfhosted` 只有在真实服务的二元与类别契约均验证后才可作为 v0.3 完成证据。若部署无法保证与远端相同的语义，应在能力表和运行概况中明确标识；不同服务的数值不可默认比较。

## 执行概况（v0.4）

`duckjeu_profile_enabled`（BOOLEAN，默认 `false`）控制详细阶段计时。无论此项是否开启，v0.2 所需的去重、批次、外部请求和缓存计数仍应可用于验收。

`SELECT duckjeu_last_profile();` 返回当前连接**最近一次含 DuckJeu 判断的已结束查询**的 JSON 文本；没有记录时返回 NULL。读取概况本身不覆盖这份记录。失败或取消的查询也保留已发生调用的脱敏计数，并在 `status` 中标明。字段详见 [profile-report.md](profile-report.md)。

概况中的查询总耗时来自宿主查询生命周期；序列化和外部往返是客户端测量，服务推理只在服务报告时出现。各时间值可能重叠，不允许相加为总耗时。费用估计缺少计价、单位或用量时为未知；不把它解释为账单。

## 示例（实现后）

```sql
SET duckjeu_execution_mode = 'optimized';
SET duckjeu_batch_enabled = true;
SET duckjeu_batch_max_judgments = 16;
SET duckjeu_max_inflight = 4;
SET duckjeu_profile_enabled = true;

SELECT jev_prob(state, 'the market is experiencing scarcity')
FROM read_parquet('examples/data/market.parquet');

SELECT duckjeu_last_profile();
```

如果当前服务尚未通过跨状态合批验证，应将 `duckjeu_batch_enabled` 设为 `false`；去重与并发仍可单独使用。跨查询缓存还需显式设置 `duckjeu_cache_enabled = true`，并满足稳定模型身份要求。
