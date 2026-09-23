# DuckJeu — v0.1 技术规格

**状态（2026-09-23）**：v0.1 / Spec 001 已完成验收。`make demo`、`cargo test` 和 mock/stub 契约验收（53/53）通过；TypeSafe live acceptance 的 `jev_prob`、`jev_bool`、`jev_choice` 均以 3 行非敏感样例通过。完整结果见 [live acceptance evidence](build/acceptance/live-20260923T100542.json) 和 [roadmap.md](roadmap.md)。记录请求模型为 `jev-latest`；服务未提供可记录的实际模型名，预期请求数按逐行调用推导，未与计费记录核对。

本文件是仓库当前范围与技术约束的主规格。可验收需求见 [001-judgment-foundation/spec.md](specs/001-judgment-foundation/spec.md)，后续修改须同步；参考源码核查见 [reference-review.md](specs/001-judgment-foundation/reference-review.md)。

## 1. 目标

在 DuckDB 中对 Parquet/表数据调用 JEV judgment：输入行状态 `state` 和自然语言标准 `criterion`，输出可用于普通 SQL 分析的概率、布尔值或类别。先实现一个可加载的 Rust 扩展，验证真实 provider 的端到端调用，不在 v0.1 建设 JDL、PG、worker 或复杂缓存。

```sql
-- 编译并加载本地扩展后执行；实际产物路径由构建脚本给出。
LOAD 'path/to/duckjeu.duckdb_extension';

SELECT
    time_run,
    jev_prob(
        struct_pack(
            price := price,
            load := load,
            wind := wind,
            forecast_error := forecast_error
        ),
        'the market is experiencing scarcity'
    ) AS p_scarcity
FROM read_parquet('market.parquet');
```

成功标准：锁定版本的 DuckDB 可加载扩展；以上查询在真实 JEV provider 上返回与输入行对齐的有效概率；三个 SQL 函数的 mock 契约测试、失败路径测试通过。不能将 mock 测试通过当成真实 JEV 集成完成。

## 2. 公开 SQL API

| 函数 | 参数 | 返回值 | 契约 |
| --- | --- | --- | --- |
| `jev_prob(state, criterion)` | STRUCT 或 VARCHAR 状态；非空 VARCHAR criterion | DOUBLE / NULL | provider 对二元 criterion 成立的概率，必须在 [0,1] 内 |
| `jev_bool(state, criterion)` | 同上 | BOOLEAN / NULL | 复用二元概率，v0.1 固定 `p >= 0.5` 为 true；业务门槛请用 `jev_prob` 明确比较 |
| `jev_choice(state, question, choices)` | 状态、非空 VARCHAR 问题、至少两个不重复的 VARCHAR 选项 | VARCHAR / NULL | 返回必须属于 `choices` 的单个 label |

参数顺序统一为 `(state, criterion)`，不采用 `(criterion, state)`。VARCHAR state 视为原始文本；STRUCT state 按下述规则编码。目标是支持 DuckDB 的任意同构 STRUCT；若当前 Rust 扩展绑定不支持泛型参数，应使用经过测试的签名/重载或桥接，不得让文档示例静默失效。

```sql
SELECT jev_bool(message, 'the customer requests a refund') FROM tickets;

SELECT jev_choice(
    struct_pack(price := price, load := load, wind := wind),
    'which market regime best describes this state?',
    ['normal', 'scarcity', 'oversupply']
) AS regime
FROM read_parquet('market.parquet');
```

## 3. 数据与正确性契约

**规范化状态**：STRUCT 转为稳定的类型化表示，字段按名称排序，区分数字/文本、SQL NULL/缺失；VARCHAR 不修改其原文。v0.1 至少支持文本、有限数值、布尔和 NULL 字段。对 DATE/TIMESTAMP、嵌套 LIST/STRUCT 等，先验证当前扩展的读取与稳定序列化能力；不支持时明确报错，不能靠调试格式或不可靠的字符串强制转换。

**行映射**：对 DataChunk 的第 i 个输入行，必须把对应 judgment 写到输出向量的第 i 个元素。预留 request_id，防止未来批量和并发请求错配。顶层 state、criterion、question 或 choices 为 SQL NULL 时，结果为 SQL NULL，不调用 provider；STRUCT 内部 NULL 字段按 NULL 保留。空 question/criterion、重复或空 choices、非有限/越界概率、未知 label、返回数量不一致均显式报错。

**二元 judgment**：`jev_prob` 返回 `[0,1]` 的概率；`jev_bool` 仅将同一概率按 0.5 阈值转换为布尔值，不宣称满足业务校准。**多选 judgment**：`jev_choice` 根据调用方提供的 question 判断，并返回给定的一个标签；不把标签顺序或未经验证的分数当作置信度。不同 provider 的概率和选择语义必须分别验证。

**错误处理**：请求超时、鉴权失败、无效响应或 provider 不支持所需问题类型时，v0.1 默认使 SQL 查询报错；不把失败转成 0、false、空标签或静默重试。错误中不能暴露凭据或原始敏感 state。调用需有有限的超时和响应大小限制。

**执行语义**：外部模型调用有成本且可能非确定；注册函数时不能标为永久确定性。DuckDB 可能优化或重复执行表达式，不能承诺“SQL 结果行数等于实际请求数”。可以用普通 SQL 的廉价过滤减少候选，但必须通过目标版本的 EXPLAIN 和实际调用计数验证。

## 4. 组件结构

```text
read_parquet / DuckDB table
           |
    DuckDB scalar function
           |
       DataChunk
           |
  typed input + canonical serializer
           |
       JudgmentRequest
           |
     Provider adapter
       /         \
   mock          real JEV
       \         /
       typed response validation
           |
   positional DuckDB output vector
```

```text
src/
  lib.rs           # 扩展入口及 SQL 函数注册
  functions/       # prob、bool、choice，输入/输出向量桥接
  serialize.rs     # CanonicalState 与类型校验
  judgment.rs      # 请求/响应契约、request_id 与校验
  provider/        # trait、mock、真实 JEV adapter
  config.rs        # provider 配置与安全的凭据读取
examples/          # 最小数据生成及 SQL demo
test/              # SQL smoke、契约与错误路径测试
```

v0.1 可以先逐行请求，但内部必须保留 chunk 行索引与独立的 provider 接口。v0.2 再实现真正的 DataChunk 批量发送、去重、并发与缓存。扩展模板、DuckDB Rust API、编译 feature 和发行方式需要先在本机验证并锁定版本。

## 5. Provider

v0.1 提供确定性 mock 和一个经真实请求验证的 JEV adapter。实际 endpoint、请求格式、模型标识及响应语义在实现时按服务协议确定，不预设所有 provider 的能力一致。

provider、model、endpoint 和超时应采用连接级配置。真实 provider 需要显式开启；凭据从受保护的配置来源读取，避免出现在 SQL 和日志中。README 要说明真实调用的数据外发和费用。

## 6. 验收

验证从干净环境构建、加载，以及三个 SQL 函数的 mock 测试。覆盖 STRUCT/VARCHAR、NULL、混合多行、概率边界、布尔阈值、合法 choice 与行序对齐。

对空问题、重复选项、越界概率、未知 label、超时和 provider 错误逐项检查。使用非敏感 Parquet 与真实 JEV 服务执行最小 SQL，记录实际 provider/model、结果类型和行数；不得以 mock 的结果代替真实集成验收。

README 应给出构建与加载命令、mock 和 live 演示、验证过的 DuckDB/系统版本、请求数与总耗时，并说明可能的数据外发。详细 benchmark 属于后续 v0.2–v0.4。

## 7. 非目标

不实现 PostgreSQL、JDL、shared cache、background worker、window/group/compare、judgment index、planner 修改或自动策略执行。具体发展顺序见 [roadmap.md](roadmap.md)。

## 8. References

- [recodelabs/duckdb-jev](https://github.com/recodelabs/duckdb-jev)：参考 DataChunk 批处理、缓存及并发请求架构。
- [colliber/duckdb-jev](https://github.com/colliber/duckdb-jev)：参考 judgment 结果的 SQL 类型映射与函数 API。

以上为参考实现；具体支持能力、兼容版本和许可证应在实际采用代码前核验。

## 9. 面向 roadmap 的架构约束

新项目从 v0.1 建立可独立验证的职责边界；先保持单一扩展和简单执行路径，不提前交付 roadmap 的运行时功能。第 4 节目录为初始建议，最终模块与绑定选择在 plan 中验证。

### 9.1 当前必须建立的边界

- **DuckDB 宿主层**：仅负责函数注册、连接配置接入、输入类型读取、NULL 处理、chunk 行映射与输出写回。DuckDB 向量、连接、内存生命周期和错误表示不得泄漏为 provider 或判断语义层的必需依赖。
- **判断语义层**：拥有类型化状态、稳定编码、问题类型、请求身份、响应校验、概率与标签规则。可在没有 DuckDB 进程或真实网络的条件下独立验证；`jev_bool` 使用相同二元概率处理规则，但不承诺与另一次独立 `jev_prob` 调用共享网络请求或随机模型输出。
- **Provider 层**：只承担实际服务协议、认证接入、能力声明和响应转换。mock 与真实 adapter 经过同一契约验证；不支持的能力明确失败，不能通过 SQL 层猜测结果语义。provider 不负责写回 DuckDB 向量。
- **执行层**：拥有请求与输入行的关联和执行策略。v0.1 允许逐行同步调用；保留独立 request_id 与行位置，未来合批或乱序完成不得改变结果所属行。协议不提供身份时，由 adapter 在可验证的一对一调用上下文中关联，禁止对有歧义的批量响应按顺序猜测。
- **配置与凭据**：连接级配置以一次执行中一致的上下文传入；不得使用可被其他连接覆盖的全局可变 provider/model 配置。凭据不属于状态内容或可输出的请求身份。
- **契约验证**：宿主映射、独立判断语义与 provider 协议分别可验证；跨层只共享明确的数据与错误契约，不把真实网络设为所有测试的前置依赖。

### 9.2 演进约束与延期范围

| Roadmap | v0.1 设计必须支持的演进边界 | 后续才交付的能力 |
| --- | --- | --- |
| v0.2 批处理与缓存 | 输入行位置与请求身份独立；同一状态不同问题类型、标准/问题、候选、provider/model 或编码版本不可误视为同一判断。去重后的结果可回填多个原始位置；缓存命中不能只依赖 hash，须校验完整规范化内容与判断上下文 | 合批、去重、并发限制、有界缓存及 1K/10K/100K 基准 |
| v0.3 Provider 抽象 | 判断语义不依赖 DuckDB 或单一服务协议；显式保留服务、模型及能力上下文，校验不同服务的概率与类别语义 | 抽取共享 Rust core，增加经过验证的本地/自托管服务 |
| v0.4 Profiling | 序列化、调用、校验和写回的职责可区分；可取得实际请求计数和总耗时，观测不输出敏感 state 或凭据 | 分阶段耗时、费用估计及系统性性能分析 |
| v0.5 PostgreSQL | 新宿主可复用状态、判断与 provider 契约；连接、事务、向量和宿主错误映射留在各宿主边界 | pgrx 接入、自管理 PG 验证、权限、缓存与 worker 设计 |
| v0.6 集合 judgment | 现有逐行请求语义保持明确；整组判断与逐行结果聚合不能混同，新增问题类型不得修改基础函数含义 | window/group/compare 场景与算子 |
| v1.0 JDL | 问题类型、标准、输出、provider/model 与阈值语义可识别；当前内部类型不宣称为稳定 JDL 格式 | 从两个宿主实际需求形成版本化定义和评测信息 |
| 后续研究 | 保存当前契约与明确的演进边界，不为尚未验证场景预建通用框架 | Store、物化、索引、streaming、校准/漂移、蒸馏和 Python/agent 接口各自立项 |

plan 必须逐项说明这些约束落在哪些职责上，并验证 DuckDB Rust 绑定与目标版本可行；未验证前不能承诺泛型 STRUCT、扩展发行或性能。保持 v0.1 可运行路径，不为了未来引入插件平台、调度服务或多宿主工程。

## 10. 补充验收与边界解释

- 空问题或空选项包含仅空白字符串；有效文本与标签不自动修剪，重复选项按原始标签精确判断。选项中的 NULL 非法，顶层 choices 为 NULL 则返回 NULL。空字符串 state 是有效原始文本，与 SQL NULL 区分。
- 空数据集不发请求。混合有效行、顶层 NULL 和重复状态的用例须跨越至少一个 DataChunk 边界，验证结果对齐。
- 缺失、重复、未知请求身份或数量不匹配必须被识别；可正确关联的乱序响应可以接收，有歧义的响应不能输出。
- 查询失败不撤销已经发生的外部请求与费用；错误信息保持脱敏。
- 真实服务验收覆盖概率、布尔及类别三种形式，每种至少三行非敏感输入；保留实际 provider/model、结果类型、行数、请求数及耗时。服务不支持所需能力或缺少凭据时记录阻塞，不能标为完成。
- 规划评审须覆盖第 9 节七类演进方向；实现验收须覆盖特性规格的全部验收场景。完成规格质量检查仅代表需求可进入规划，不代表实现或真实集成完成。
