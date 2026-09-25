# Contract: Provider Capabilities and Batched Judgment

本契约扩展 [v0.1 provider 协议](../../001-judgment-foundation/contracts/provider-protocol.md)，不改变公开 SQL 三函数。内部 provider 需提供 `capabilities()` 与 `judge_many(requests)`；即使走逐行路径，也返回统一的逐项关联结果和可用元数据。

## Capability 声明

| 能力 | TypeSafe 远端 | local-jev 候选 | mock |
| --- | --- | --- | --- |
| 二元 noul / choice | 已由 v0.1 live 验收 | 源码支持，DuckJeu live 待验 | 确定性本地测试 |
| 不同状态的原生独立请求数组 | 官方协议未提供 | 候选 API 未提供 | 可控模拟 |
| 不同状态的复合 state 合批 | 对 effective model `jev-1.13.0` 已通过真实 Noul/Choice 与 DuckJeu SQL 验收；其他模型未知 | 需单独验收；当前不宣称支持 | 可用于关联与计数测试 |
| 逐问题关联键 | `questions` / `answers` 键 | 兼容相同形状 | 明确模拟身份 |
| 服务报告实际模型 | 响应 schema 有 `model` | 候选返回 resolved model | 固定 mock 标识 |
| 用量 | 官方响应有输入/输出 token | 未验证，缺失记未知 | 非计费 |
| 服务推理时长 | 官方公开响应未提供 | 未验证，缺失记未知 | 非真实推理 |

声明值分 `verified`、`unsupported`、`unknown`；`unknown` 不能按支持使用。能力还含服务端候选上限、请求/响应限制和截断信号。选择适配器、模型或协议版本后，必须重新评估声明。

## 通用请求与响应

`judge_many` 输入一组已验证判断，每项有完整判断身份、独立 question key 和行目标集合；输出 `BatchResult`，按 key 给出类型化结果，并可附带请求模型、实际模型、usage、服务推理时间和协议警告。输出 key 集合必须与输入**完全相同**，且无重复；值仍按 v0.1 契约验证。任何一项失败或截断，整个批次为错误，不写缓存、不输出部分结果。

发送前检查服务能力、单批判断数、本地请求字节上限、服务声明的 token/选项上限及原有响应上限。服务未声明足够限制时采用保守拆批并对远端拒绝显式报错；不自动重试。网络请求数以真正发出的 HTTP 尝试计数，拆批后按实际数记录。

## TypeSafe 复合状态候选映射

TypeSafe 的真实格式仍只有一个顶层 `state`；以下映射由 DuckJeu 根据结构化 state 与多问题能力组合，**不是官方原生多状态接口**。该映射已对有效模型 `jev-1.13.0` 通过有限非敏感 live probes；其他模型不能据此视为已验证：

```json
{
  "model": "jev-latest",
  "state": {
    "rows": ["row A", "row B"],
    "condition": "the customer requests a refund"
  },
  "questions": {
    "r0": {
      "type": "noul",
      "instructions": "Evaluate whether state.condition holds for state.rows[0]."
    },
    "r1": {
      "type": "noul",
      "instructions": "Evaluate whether state.condition holds for state.rows[1]."
    }
  }
}
```

同批的二元请求首期要求相同 criterion，以继续使用单一 `state.condition`；类别请求首期按相同 question 与有序候选分组，逐 question 的 `criteria` 仍来自用户原始标签。不能把不同问题静默变成共享条件。响应中的 `answers.r0/r1` 各自校验并映射到对应原始行；键不依赖返回顺序。

因为每个问题都能看到同一复合 state 的其他行，结果可能与单行调用不同。已经使用非敏感数据完成行序、无关行插入、二元/类别、answer key 乱序及 DuckJeu SQL 请求计数验证；结果及适用模型见 [batch capability evidence](../evidence/batch-capability.md)。用户必须显式启用合批。适用模型以外的结果模型缺失或漂移时，DuckJeu 拒绝批次结果；改变模型或复合映射须重新验收。单请求承载两个不同判断且外部请求数下降，才算通过合批计数验收。

## 本地候选协议

v0.3 选择源码固定版 [local-jev](https://github.com/amithgc/local-jev/tree/64a0b31ff343dca32142496cf9edca5a0174a18d) 作为候选：`POST /v1/systemone`，请求/响应形状与 TypeSafe 相近，可共用 wire codec，但保留独立 provider 名称和能力记录。部署默认使用本机回环地址；服务可用 `LOCAL_JEV_API_KEY` 启用 Bearer 认证，DuckJeu 从独立受保护环境变量读取相应凭据，不复用 TypeSafe key。远端暴露与代理后的安全配置不作为本次本地验收证据。

候选最多 255 个 choice 标签，超出在发送前报能力错误。服务可能截断过大状态并通过 `x-local-jev-truncated: true` 响应头提示；adapter 必须识别该信号并拒绝截断结果。固定服务源码和模型权重 revision 后，才可把模型身份视为稳定缓存条件。其本地校准与远端 Jev 不相同，分数不跨服务比较或共享缓存。

## 与现有接口的兼容

`jev_prob`、`jev_bool`、`jev_choice` 的 SQL 签名、顶层 NULL、布尔 0.5 阈值、候选精确成员及脱敏错误保持不变。`row` 模式继续逐行调用；优化模式中一次判断可以回填多行。无法证明服务能力时，显式批处理请求失败；只启用去重或并发不算真实合批。
