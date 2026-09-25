# Phase 0 Research: v0.2–v0.4 Judgment Runtime

**日期**：2026-09-23。以下把已核实的协议/源码事实与计划推断分开。项目锁定 DuckDB v1.5.5、duckdb-rs/libduckdb-sys 1.10505.0；服务能力在每个里程碑真实验收前仍须复核。研究结论用于设计，不能代替实现或 live 验收。

## D1. 跨状态合批的真实边界

**Decision**：TypeSafe System One 的一个请求只有一个顶层 `state`；`questions` 的多个键只是同一 state 上的多个问题。v0.2 的候选路径是把多个不同输入行放入同一个结构化 state 的 `rows`，给每个行判断单独的 question key，并在问题指令中明确引用 `rows[i]`。这是**复合上下文合批**，不是原生的独立状态数组接口。只有在真实服务对非敏感样例证实结果关联、类型和值域可靠，且用户显式启用后，才可作为跨状态合批能力；如果复合上下文导致不可接受的语义差异，就必须标为未通过，不能把并发单行调用记作合批。

**Rationale**：[TypeSafe API](https://docs.typesafe.ai/api) 定义单一 `state` 与按键回显的 `answers`；[结构化状态](https://docs.typesafe.ai/concepts/state)支持数组/对象。把多个独立行放在 `rows` 中是本项目依据这两项能力的**推断**，官方没有保证它与逐行独立请求语义完全等价。v0.1 已把每个请求的 state 组织为单行 `rows[0]`，详见 [provider-protocol.md](../001-judgment-foundation/contracts/provider-protocol.md) 与 `src/provider/typesafe.rs`。TypeSafe [模型限制](https://docs.typesafe.ai/models)列出整请求 64K tokens、state 与最长问题合计 32K tokens；公开 schema 没有给出最大 question 数或 HTTP 字节数，因此不能当作无限容量。批量还要受本地请求字节上限和既有 1MiB 响应上限约束，超出时拆批。

**Alternatives considered**：只合并同一 state 的多个问题（符合原生协议，却不足以证明不同状态的 v0.2 目标）；并发发送单行请求（能降低等待，不能减少外部请求数）；代理端再逐行扇出（只减少 DuckJeu 到代理的往返，不减少真实服务调用，不能算真正合批）。

## D2. 连接级缓存与查询生命周期

**Decision**：用锁定版 DuckDB 的连接状态生命周期承载每连接的缓存、并发额度与查询观测；Rust 仍拥有全部判断、缓存和执行逻辑。若纯 C 扩展接口不足以持有连接结束回调，增加一个范围严格限定的 C++ 宿主桥接层，将 Rust 状态挂到 `ClientContextState`，在连接销毁时释放，并利用 `QueryBegin/QueryEnd` 聚合查询概况。实施的第一步是对锁定版做最小编译/加载探针；探针未通过前不得开启跨查询缓存，也不能用仅按 connection ID 的进程全局映射代替。

**Rationale**：DuckDB v1.5.5 的 [ClientContext](https://raw.githubusercontent.com/duckdb/duckdb/v1.5.5/src/include/duckdb/main/client_context.hpp) 持有 `registered_state`；[ClientContextState](https://raw.githubusercontent.com/duckdb/duckdb/v1.5.5/src/include/duckdb/main/client_context_state.hpp) 明确定义连接生命周期以及 QueryBegin/QueryEnd 回调。公开 [C API](https://duckdb.org/docs/lts/clients/c/api) 只提供 client context 读取和 connection ID，没有注册连接结束状态的接口。当前 `src/functions/mod.rs` 的 `ExecutionState` 属于一次 scalar 执行，不能承担跨查询缓存。只用 connection ID 的静态表缺少可靠的连接销毁通知，可能泄漏内存或跨连接误命中。

**Alternatives considered**：每次函数执行局部缓存（只能同次执行复用，不满足跨查询）；进程静态哈希表（连接 ID/数据库实例重用与回收有风险）；数据库级 ObjectCache（连接隔离和凭据边界不够清晰）。C++ 桥接只处理生命周期与函数注册接入，不承载判断语义。

## D3. 执行策略、并发与缓存语义

**Decision**：保留 `row` 默认模式。优化模式先将每个数据块的有效行转成完整判断身份，再去重与检查缓存，按服务能力分组、拆批，以连接共用额度发送。映射表一对多指向原始行，响应完整校验后写回；取消/错误释放额度，不自动重试。缓存按条目数、字节与 TTL 同时有界，明确清空接口；可变模型别名无法预先验证下一次仍绑定同一实际版本时，不跨查询复用。

**Rationale**：v0.1 的规范化字节和 request key 已具备完整比较/结果关联基础（`src/serialize.rs`、`src/judgment.rs`）。当前 `src/functions/mod.rs` 仍在行循环中调用 `provider.judge`，所以要把“收集行→执行多请求→回填”放在宿主读写之间。批次按服务与问题语义能力归组；TypeSafe 二元请求共享 state.condition，首期只在相同 criterion 等兼容条件下合批，避免改变提问方式。

**Alternatives considered**：按哈希直接命中（可能错配）；每个 chunk 独立并发上限（并行 chunk 会突破连接限制）；默认跨查询缓存（改变随机模型的新鲜度）；自动重试（费用与调用次数不可预测且违背现有契约）。

## D4. 共享判断核心与服务能力

**Decision**：v0.2 先使执行器和缓存规则不依赖 DuckDB 类型；v0.3 把 `serialize.rs`、`judgment.rs`、执行器、缓存/观测数据契约移入共享 Rust core，扩展保留向量、配置与连接生命周期。服务 adapter 实现明确的能力声明与协议转换，远端和本地/自托管服务走同一校验路径。

**Rationale**：当前 `src/serialize.rs`、`src/judgment.rs` 已独立于宿主；`src/provider/mod.rs` 的 `Provider::judge` 只接受单请求，需扩展为批量能力与携带元数据的响应，但 SQL 三函数无需改签名。相关边界见 [v0.1 设计评审](../001-judgment-foundation/design-review.md)。

**Alternatives considered**：把 DuckDB 向量带入共享 core（妨碍 v0.5 新宿主）；假定所有服务都支持批量/相同分数意义（与已知协议不符）；把可控 mock 当作第二个真实服务（不能满足 Spec 002 的验收）。

## D5. 模型、用量、时间和费用证据

**Decision**：TypeSafe 响应中的 `model` 与 `usage.input_tokens/output_tokens` 应随判断结果一起保留；请求模型与实际模型分开。客户端计时只测可观测的序列化与完整外部往返，服务未提供逐请求推理时间时该值与纯网络时间均为未知。费用由官方计价、真实用量和日期计算，不能用 SQL 行数或请求数推断 token 用量，不能把估值称作账单。

**Rationale**：[TypeSafe OpenAPI](https://api.typesafe.ai/openapi.json) 与 [官方 SDK 结果类型](https://github.com/typesafe-ai/typesafe-sdk-js/blob/main/src/types.ts)含 model 和 usage，未定义推理时长。TypeSafe [公开价格](https://docs.typesafe.ai/models)在本次查阅时为每百万输入 token 0.042 美元、输出免费；费率可能变化，执行验收时要重新查。当前 `src/provider/typesafe.rs` 只解析 answers、丢弃 model/usage；`test/sql/run_live_acceptance.py` 的独立探针可能把网络/JSON 错误折成 `model_reported: null`，既有验收记录不能证明服务未报告模型。

**Alternatives considered**：以端到端耗时推算推理时间（无法排除网络/排队）；以行数乘固定价格（计费单位错误）；默认缺失用量为零（低估费用）；以单独探针填充实际模型但不与各次判断绑定（可能误配）。

## D6. 固定数据与对照实验

**Decision**：生成包含可控重复率、NULL、不同问题和跨数据块边界的 1K/10K/100K 非敏感数据。每个规模运行逐行、预过滤、合批、缓存四种模式；分别保存 SQL、数据校验摘要、服务/模型、环境版本、配置、正确性、实际请求数、命中率、耗时与费用依据。大规模成本可先用本地可控服务验证计数与正确性；真实服务样例单独报告，不能以模拟计时声称真实吞吐。

**Rationale**：DuckDB 可能调整表达式执行，预过滤是否减少调用须实测；模型输出可能随机，真实服务的逐行与合批比较应看值域、标签成员、行关联及统计性差异，不能要求每次概率逐位相同。TypeSafe [并行问题示例](https://docs.typesafe.ai/cookbooks/parallel_questions)展示同一 state 多问题合批的成本优势，但不能外推为独立多行的固定降本比例。

**Alternatives considered**：只测延迟（无法发现调用数和错误命中）；只用 mock（无法验证真实协议/费用）；混用不同模型或数据（对照不可解释）。

## D7. 本地/自托管 JEV 兼容服务候选

**Decision**：v0.3 以源码固定在 `64a0b31ff343dca32142496cf9edca5a0174a18d` 的 [amithgc/local-jev](https://github.com/amithgc/local-jev/tree/64a0b31ff343dca32142496cf9edca5a0174a18d) 作为真实本地服务候选。验收先用较小的 `nli-deberta-large` 完成契约，资源允许时再用 `llm-qwen3.5-4b` 做质量对照。这是**源码和项目测试所支持的候选**，尚未经过 DuckJeu live 调用，不能据此标记 v0.3 完成。部署时固定服务源码并记录权重 revision；若服务无法锁定或确认权重版本，则禁用跨查询缓存、把实际权重版本标为未知，不把本地概率与 TypeSafe 概率直接比较。

**Rationale**：[服务 API 源码](https://github.com/amithgc/local-jev/blob/64a0b31ff343dca32142496cf9edca5a0174a18d/src/local_jev/api.py) 提供 `POST /v1/systemone`，按 question key 返回 typed answers 和实际模型；[兼容性测试](https://github.com/amithgc/local-jev/blob/64a0b31ff343dca32142496cf9edca5a0174a18d/tests/test_sdk_dropin.py) 使用未修改的 TypeSafe SDK。[引擎源码](https://github.com/amithgc/local-jev/blob/64a0b31ff343dca32142496cf9edca5a0174a18d/src/local_jev/engine.py) 将 noul 解释为 yes 概率，choice 返回提供的 label。其校准针对本地模型，不等同于 TypeSafe Jev；模型卡和项目评测仅是来源方声明，目标数据上的有效性须另行验证。项目 [README](https://github.com/amithgc/local-jev/blob/64a0b31ff343dca32142496cf9edca5a0174a18d/README.md)说明研究阶段、首次默认只下载约 0.87GB 的 NLI 模型；Qwen 需显式指定，下载约 9.32GB、运行约需 8.5GB 内存。代码为 [MIT](https://github.com/amithgc/local-jev/blob/64a0b31ff343dca32142496cf9edca5a0174a18d/LICENSE)，[Qwen3.5-4B 权重](https://huggingface.co/Qwen/Qwen3.5-4B)为 Apache-2.0。

**Integration limits**：该候选与 TypeSafe 一样采用单一顶层 state，不能单独证明跨状态合批。服务可能截断过大状态并通过响应头提示；DuckJeu 必须检测截断并拒绝将其当作完整判断。[choice 限制](https://github.com/amithgc/local-jev/blob/64a0b31ff343dca32142496cf9edca5a0174a18d/src/local_jev/questions.py)为最多 255 项，超限在请求前报能力错误。其模型加载若未固定不可变权重 revision，跨查询缓存仍须关闭。

**Alternatives considered**：先部署 Qwen，可直接做质量对照但显著增加首轮下载与内存需求；单靠远端 TypeSafe 或本地 HTTP stub 均不能满足“真实本地/自托管兼容服务”验收。任何新候选都需重新做协议、概率/类别及模型身份核验。
