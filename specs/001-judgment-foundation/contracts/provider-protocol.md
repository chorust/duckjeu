# Contract: Provider Protocol（v0.1）

v0.1 两个 adapter 经同一判断契约验证：mock（离线、确定性）与 typesafe（TypeSafe System One HTTP）。协议细节以官方文档（docs.typesafe.ai，2026-09-22 检索）与 recodelabs 源码（revision `378d36ff`）交叉确认为准；实现时如与服务实际行为冲突，以服务实际行为为准并更新本文档。

## 1. TypeSafe System One（typesafe adapter）

### 请求

- `POST {duckjeu_api_url}`，默认 `https://api.typesafe.ai/v1/systemone`
- Header：`Authorization: Bearer $TYPESAFE_API_KEY`（凭据仅出现于此）
- Body（JSON）：

```json
{
  "model": "jev-latest",
  "state": { "...": "由 CanonicalState 映射；noul 的 criterion 并入 state.condition" },
  "questions": { "r0": { "...": "见下" } }
}
```

- `questions` 的 key 由调用方指定，响应 `answers` 按同 key 回显——原生请求关联机制。v0.1 每请求单行单问题，key 恒为 `"r0"`；v0.2 批量扩展为 `"r{i}"`。

### 问题类型映射

| SQL 函数 | question | 内容 |
| --- | --- | --- |
| `jev_prob` / `jev_bool` | `noul` | criterion 放入 `state.condition`；instructions 引用 `rows[0]`（与 recodelabs 一致；具体 prompt 文本为实现细节，由 live 验收覆盖） |
| `jev_choice` | `choice` | `questions.r0.criteria` 为 `{label: null}` 映射，label 来自 `choices` 原样；instructions 包含调用方的 `question`，要求依据 `rows[0]` 选择最能回答该问题的单个 label |

### 响应与校验

```json
{
  "model": "jev-1.13.0",
  "answers": { "r0": { "type": "noul", "noul": 0.83 } }
}
```

全部满足才接受，任一不满足即查询报错：

1. `answers` 恰好包含请求的全部 key（v0.1 即 `"r0"`），无缺失、无多余；
2. `noul` answer 的 `type` 为 `"noul"`，且 `noul` 字段为 [0,1] 内有限数值；
3. `choice` answer 的 `type` 为 `"choice"`，且 `choice` 字段为候选集内的字符串；
4. 响应中的实际 `model` 记入验收记录。

Choice answer 还可包含 `confidence` 和按候选标签索引的 `probabilities`；DuckJeu v0.1 只读取 `choice` 标签。

### 传输限制

- 超时：`duckjeu_timeout_ms`（ureq `timeout_global`），默认 30000ms；
- 响应体上限：`duckjeu_max_response_bytes`（body `limit(n)`），默认 1MiB；
- HTTP 非 2xx 显式上抛；远端正文**不进入错误信息**（服务端可能回显凭据），只保留状态码与可操作提示；
- 不自动重试；凭据不出现在 URL、错误信息或日志。

## 2. Mock provider（默认）

- 进程内实现，零网络依赖，供 demo 与单元测试；
- 确定性：由 canonical state + 问题内容（criterion/question/choices）的 hash 派生 [0,1] 值；choice 按 hash 选候选 label；
- 与真实 adapter 走同一 JudgmentRequest → JudgmentResult 校验路径，保证契约一致；
- 脚本化精确值（越界概率、未知 label、缺失 answer 等错误路径）测试不走 mock，走本地 HTTP stub + typesafe adapter。
