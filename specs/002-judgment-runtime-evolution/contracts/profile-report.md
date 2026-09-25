# Contract: DuckJeu Execution Profile（v0.4）

`duckjeu_last_profile()` 返回版本化 JSON 文本。以下是字段结构示意；数值由实际执行填入，未知值必须为 JSON `null` 并写明原因。此函数只读取当前连接最近一次含判断的已结束查询；不会覆盖该记录。

```json
{
  "schema_version": 1,
  "status": "completed",
  "provider": "typesafe",
  "requested_model": "jev-latest",
  "reported_models": [],
  "counts": {
    "input_rows": 0,
    "null_rows": 0,
    "valid_rows": 0,
    "unique_judgments": 0,
    "deduplicated_rows": 0,
    "cache_hit_judgments": 0,
    "provider_invocations": 0,
    "external_requests": 0,
    "batched_requests": 0,
    "batch_size_histogram": {},
    "failed_requests": 0
  },
  "timing_ms": {
    "query_wall": null,
    "serialization_sum": null,
    "external_roundtrip_sum": null,
    "server_inference_sum": null,
    "pure_network_sum": null
  },
  "usage": {
    "input_tokens": null,
    "output_tokens": null,
    "missing_response_count": 0
  },
  "cost_estimate": {
    "amount": null,
    "currency": null,
    "billing_unit": null,
    "unit_rate": null,
    "rate_source": null,
    "rate_observed_at": null,
    "unknown_reason": "pricing_or_usage_unavailable"
  }
}
```

## 计数规则

- `input_rows` 为判断函数实际处理的行数，不假定等于查询最终返回行数；多次调用函数的同一数据行计多次。顶层 NULL 包含于 `null_rows`，不产生判断。
- `valid_rows` 是已构造有效判断的行数；`unique_judgments` 是本查询完整判断身份去重后的数量。`deduplicated_rows = valid_rows - unique_judgments`，因输入错误中断时可只统计已处理部分。
- `cache_hit_judgments` 只统计从前一次查询的有效缓存中取得的唯一判断；同一查询内去重不算命中。`provider_invocations` 包括 mock、本地和远端适配器调用；`external_requests` 只包括实际发生的网络请求/尝试，不能由行数推算。
- `batched_requests` 只计单次外部请求承载两个以上不同判断的调用；`batch_size_histogram` 以实际发送的每批判断数为键。失败后已经发送的请求仍进入 `external_requests` 和 `failed_requests`。
- `reported_models` 为服务响应报告的去重模型标识列表；列表空表示没有可信的实际标识，不能用 `requested_model` 代填。

## 时间与费用规则

- `query_wall` 来自含判断查询的开始到结束；失败/取消也记录已发生的区间。`serialization_sum` 和 `external_roundtrip_sum` 分别为各调用的累计时间，多个并行调用的和可能大于 `query_wall`。
- `external_roundtrip_sum` 包含网络、排队和服务处理；无法据此分离纯网络与推理。`server_inference_sum` 只在服务提供可信逐请求时长且全部所需请求有值时填入，否则为 `null`。`pure_network_sum` 没有直接证据时为 `null`。
- `usage` 来自服务真实响应的计费单位，按实际外部请求累计；有任一可能计费的请求缺失用量时，费用估算为未知。缓存命中不新增服务用量，失败请求可能计费。
- `cost_estimate` 只有在费率、单位、币种和全部所需用量可核查时填金额；注明费率来源及读取时间。它是估值，不能称为服务账单。TypeSafe 当前公开的是输入 token 费率和输出免费规则，实施时须重新核对最新官方价格。
- **当前实现偏差（2026-09-25 评审发现）**：`duckjeu_last_profile()` 目前仅按 `provider=typesafe` 和完整用量套用公开费率，没有核查 `duckjeu_api_url` 指向的服务或实际模型的计价来源。自定义端点即使是本地 stub，也可能返回非空 USD 金额；该金额不能作为可信费用估计。修复前需由使用者核实端点和模型计价，修复后应按上述规则将无依据的费用标为未知。
- 报告不含原始 state、标准/问题、候选、凭据或远端错误正文。服务地址也只在用户明确允许时以脱敏形式呈现。

## 验收映射

本契约支持 Spec 002 的 FR-008、FR-014–FR-017 和 SC-005–SC-006。1K/10K/100K 对照报告另记录数据集、运行环境、预过滤条件和模式；不得把一次运行的概况误当成完整实验结论。
