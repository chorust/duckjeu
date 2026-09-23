# Reference 源码核查与设计取舍

核查日期：2026-09-22。Luna worker 只读检查了两个仓库的 README、源码、测试、许可证与相关提交。以下结论来自源码审阅，未编译或执行参考项目，不能证明本项目的 Rust 绑定或真实服务可用性。本次未复制参考实现代码。

## 固定来源与许可证

| 仓库 | 核查 revision | 许可证 |
| --- | --- | --- |
| [recodelabs/duckdb-jev](https://github.com/recodelabs/duckdb-jev/tree/378d36ffba2b7dd8cc3e1a20a673768748892bbe) | `378d36ffba2b7dd8cc3e1a20a673768748892bbe`，v0.1.0 | MIT，LICENSE 标注 Recode Labs |
| [colliber/duckdb-jev](https://github.com/colliber/duckdb-jev/tree/fad67eb4a1def91fcf1efbc96bdf5e6e11208644) | `fad67eb4a1def91fcf1efbc96bdf5e6e11208644` | MIT，LICENSE 标注 DuckDB Foundation 2018–2025 |

采用源码或其他受版权保护的材料时，保留适用的许可证和版权声明，并核查所采用文件及依赖自身的许可。仓库许可证不是对外部服务协议或服务可用性的证明。

## recodelabs：批处理和结果回填值得借鉴

- [jev_extension.cpp](https://github.com/recodelabs/duckdb-jev/blob/378d36ffba2b7dd8cc3e1a20a673768748892bbe/src/jev_extension.cpp#L103)：按问题、类型和选项分组，按行内容去重，保留 `targets` 回填输入位置。去重保存完整内容以校验 hash 冲突。适合 v0.2；v0.1 先保留行位置与请求身份边界。该项目没有可直接复用的独立 request_id 契约。
- [函数注册](https://github.com/recodelabs/duckdb-jev/blob/378d36ffba2b7dd8cc3e1a20a673768748892bbe/src/jev_extension.cpp#L317)：宏调用内部 JSON 函数，状态经过 `to_json`。可参考桥接思路，但不能据此认定满足 DuckJeu 的稳定类型化编码。底层函数标记 VOLATILE，NULL 行跳过调用，这与本项目执行语义方向一致。
- [jev_state.hpp](https://github.com/recodelabs/duckdb-jev/blob/378d36ffba2b7dd8cc3e1a20a673768748892bbe/src/include/jev_state.hpp#L34)：缓存依附数据库 ObjectCache 并使用 mutex，但缓存无界，内存估计无效。可借鉴隔离和并发保护；不能直接满足 v0.2 有界缓存要求。
- [jev_client.cpp](https://github.com/recodelabs/duckdb-jev/blob/378d36ffba2b7dd8cc3e1a20a673768748892bbe/src/jev_client.cpp#L101)：绑定 TypeSafe System One 协议，具有超时、重试、退避和错误截断；每 chunk 创建线程，局部并发还会乘上 DuckDB 的执行线程数。没有独立 provider/core 边界，也没有响应大小上限和完整概率、标签、响应数量校验。
- [mock_api.py](https://github.com/recodelabs/duckdb-jev/blob/378d36ffba2b7dd8cc3e1a20a673768748892bbe/test/mock_api.py) 和 [SQL 测试](https://github.com/recodelabs/duckdb-jev/blob/378d36ffba2b7dd8cc3e1a20a673768748892bbe/test/sql/jev.test)：可借鉴调用计数、空值、行序、冲突和错误路径。HTTP mock 不等于真实服务验收。

相关历史修正涉及 hash 冲突、worker 异常捕获、线程启动失败后的回收、失败请求计数。未来并发实现需覆盖这些故障，不能只测正常路径。

## colliber：类型映射和凭据处理值得借鉴

- [绑定和返回类型](https://github.com/colliber/duckdb-jev/blob/fad67eb4a1def91fcf1efbc96bdf5e6e11208644/src/jev_extension.cpp#L82)：`jev_choice(state, MAP{...})` 生成 ENUM，`jev_score` 返回 DOUBLE，`jev_ask` 可生成类型化 STRUCT；criteria 受绑定时常量约束。可借鉴结果类型与未知标签的显式验证，但这些签名不兼容 DuckJeu 已定的三函数契约，不能替换现有 API。
- [jev_secret.cpp](https://github.com/colliber/duckdb-jev/blob/fad67eb4a1def91fcf1efbc96bdf5e6e11208644/src/jev_secret.cpp#L16)：使用 DuckDB secret 并将 key 标记 redact。借鉴凭据保护与配置缺失早失败原则；本期具体配置机制仍须验证 Rust 绑定支持。
- [执行路径](https://github.com/colliber/duckdb-jev/blob/fad67eb4a1def91fcf1efbc96bdf5e6e11208644/src/jev_extension.cpp#L274)：chunk 内逐行请求、最多 16 个线程；合并的是同一行的多个问题，**不是跨行 DataChunk 批处理**。不能将它当作 v0.2 合批能力的实现证据。
- [jev_client.cpp](https://github.com/colliber/duckdb-jev/blob/fad67eb4a1def91fcf1efbc96bdf5e6e11208644/src/jev_client.cpp#L17)：静态进程级缓存，达到 100000 条后整体清空；key 使用 endpoint 与完整请求体，未包含凭据身份。不能直接照搬为跨连接缓存策略；还需考虑配置隔离、内存字节上限和敏感状态生命周期。numeric 响应只验类型，未严格校验有限性和值域，错误 body 和响应大小也缺少本项目要求的限制。
- [HTTP 测试](https://github.com/colliber/duckdb-jev/blob/fad67eb4a1def91fcf1efbc96bdf5e6e11208644/test/python/test_http.py) 与 [live 测试](https://github.com/colliber/duckdb-jev/blob/fad67eb4a1def91fcf1efbc96bdf5e6e11208644/test/live/jev.test)：可以借鉴分离可控契约测试和真实服务测试。其 `jev_on_error='null'` 行为不纳入 DuckJeu v0.1。

## 落入当前规格的结论

| 决定 | 对应位置 |
| --- | --- |
| 保留独立判断语义与 provider 适配边界，不复制 C++ 工程或硬编码服务协议 | 根 spec §9.1；FR-008、FR-012 |
| 保留行位置与独立请求身份，显式检测响应错配 | 根 spec §9.1、§10；FR-006 |
| 严格验证概率、标签、数量和响应大小；错误脱敏，不静默重试或转 NULL | 根 spec §3、§10；FR-004、FR-007、FR-009 |
| 真实与 mock 分别验收，不把参考项目的 live 测试存在视为本项目通过 | 根 spec §6、§10；SC-004 |
| 将合批、去重、并发、缓存及其故障测试放到 v0.2；缓存不得只信任 hash | 根 spec §9.2；roadmap v0.2 |
| 不引入参考项目的 ENUM、jev_ask、score API 和错误转空值策略 | 根 spec §2、§7；FR-001、FR-013 |

两个参考项目都存在自动重试；它们仅提供后续策略设计参考，当前“不静默重试”约束继续有效。未来是否重试以及缓存隔离范围，须在相应里程碑明确，不能隐式沿用参考默认值。

## 留给 plan 的验证事项

1. 锁定 DuckDB 与 Rust 扩展模板版本，实际验证加载、函数注册、连接设置、STRUCT/VARCHAR 读取及非确定函数注册能力。
2. 验证真实 JEV 协议、模型标识、二元概率与类别能力；明确受保护凭据来源、有限超时与响应大小限制。
3. 明确宿主层、判断语义层、provider 层、执行层的依赖方向及可独立运行的验证方式。
4. 将未来缓存正确性限定为完整判断上下文与编码版本一致；缓存容量、隔离、淘汰、并发预算和重试策略留待 v0.2 设计。
