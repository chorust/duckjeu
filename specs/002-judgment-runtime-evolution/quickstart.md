# Quickstart: v0.2–v0.4 Validation Guide

本文件记录 Spec 002 的验收顺序和当前实测进度。US1–US3 离线实现、通用批次/并发边界、共享 judgment core、local-jev adapter、profile API 与 stub 对照脚本均已实现。2026-09-24 TypeSafe 直接 Noul/Choice 复合探针通过；DuckJeu 扩展 SQL 又以两个 composite HTTP 请求完成四个唯一判断，profile 与结果映射均通过。能力只对报告实际模型 `jev-1.13.0` 验证。v0.2、v0.3、v0.4 的原验收记录已形成；当前 v0.2 取消传播和 v0.4 计价来源待修复、复验；local-jev 峰值 RSS 和 memory footprint 已记录，实际加载权重 revision 未知、跨查询缓存关闭。真实 local-jev 1K/10K/100K 对照通过；TypeSafe 多规模性能和任何真实账单金额未测试。具体契约见 [SQL runtime](contracts/sql-runtime.md)、[provider batch](contracts/provider-batch.md)、[profile report](contracts/profile-report.md)。

## 1. 现有基线

在仓库根目录、macOS arm64、DuckDB v1.5.5、Rust ≥1.85、Python 3 下：

```sh
git submodule update --init
make release venv
make demo
make contract-test
```

预期：v0.1 三函数仍可加载运行；顶层 NULL 不调用服务、概率/布尔/类别及错误规则不变。任何新功能导致此基线失败时，后续验收停止。

v0.2 起的连接状态桥接使用 DuckDB 内部 C++ 生命周期接口，必须与运行时 ABI 完全匹配。为启用连接级执行状态、缓存和概况，另准备准确的 v1.5.5 源码并导出路径：

```sh
git clone --depth 1 --branch v1.5.5 https://github.com/duckdb/duckdb.git /tmp/duckdb-v1.5.5
export DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5
```

设置该路径可构建连接级优化功能；不设置该路径仍可构建和使用 `row` 模式，但优化、batch、cache clear 等连接级功能不可用。`build.rs` 会拒绝非 v1.5.5 源码。

## 2. v0.2 离线执行契约

离线 runtime 契约入口使用本地脚本化 HTTP 服务，不调用真实收费服务：

```sh
.venv/bin/python test/sql/run_runtime_contract_tests.py
```

至少覆盖：跨 DataChunk 重复状态、不同问题与候选顺序、哈希碰撞、乱序/缺失/重复响应、超时、连接内并发上限、两连接缓存隔离、凭据及配置切换、TTL/条目数/字节淘汰、缓存清空、默认 `row` 模式。期望行映射和既有结果契约 100% 正确，错误命中为零，连接峰值请求不超过设置值。脚本须报告实际服务请求数，不能以结果行数代替。取消查询的最终状态由 profile SQL contract 覆盖；执行期间中断后是否停止后续请求，仍待修复和专项验证。

离线 runtime SQL、基础 SQL 契约和 Rust workspace 契约分别覆盖连接/缓存、原有 SQL 行为和核心关联。executor 的受控 HTTP harness 验证真实本地往返、拆批、乱序响应、缺失响应、超时计数及失败不重试；provider/core 契约另覆盖错误 key 和结果类型。TypeSafe 未验证模型的合批仍在 HTTP 前失败。详见 [US2 evidence](evidence/us2.md) 与 [US3 evidence](evidence/us3.md)。

直接协议探针会运行单行对照、Noul/Choice 复合请求、逆序 key 和无关行插入检查（需要真实凭据并显式开启）：

```sh
DUCKJEU_BATCH_LIVE_TEST=1 .venv/bin/python test/sql/run_batch_live_acceptance.py
```

DuckJeu 扩展端到端探针将四个合成判断放进一条 SQL 查询，预期 profile 为 4 个唯一判断、2 次外部请求、2 个二元 batch，且 Noul/Boolean/Choice 行映射正确：

```sh
DUCKJEU_BATCH_EXTENSION_LIVE_TEST=1 make batch-extension-live
```

两项 live 验收均已通过。直接协议探针共 8 次外部 HTTP；DuckJeu 扩展验收共 2 次。遇到 CA 验证错误时，可在项目 venv 安装并指定 certifi 根证书后重试（不要关闭 TLS 验证）：

```sh
.venv/bin/python -m pip install certifi
export SSL_CERT_FILE="$(.venv/bin/python -c 'import certifi; print(certifi.where())')"
DUCKJEU_BATCH_EXTENSION_LIVE_TEST=1 make batch-extension-live
```

结果见 [batch-live evidence](evidence/batch-live.json)、[extension live evidence](evidence/batch-extension-live.json) 和 [capability decision](evidence/batch-capability.md)。

## 3. v0.3 本地服务验收（adapter 与三函数 live SQL 已通过）

在**另一个终端**准备固定源码版本的 [local-jev](https://github.com/amithgc/local-jev/tree/64a0b31ff343dca32142496cf9edca5a0174a18d)，需要 `uv` 和 Python 3.12。以下示例用项目默认的小模型完成协议验收；首次运行会下载其权重。

```sh
git clone https://github.com/amithgc/local-jev.git /tmp/duckjeu-local-jev
git -C /tmp/duckjeu-local-jev checkout 64a0b31ff343dca32142496cf9edca5a0174a18d
cd /tmp/duckjeu-local-jev
uv venv --python 3.12 .venv
uv pip install --python .venv/bin/python -e .
.venv/bin/local-jev serve --model nli-deberta-large
```

本地服务应在 `http://127.0.0.1:8765/v1/systemone` 应答。记录下载的权重 revision；若服务不能固定或确认 revision，保持跨查询缓存关闭，并在报告中将实际权重版本标为未知。若为其设置 `LOCAL_JEV_API_KEY`，在 DuckJeu 进程环境中用 `DUCKJEU_LOCAL_JEV_API_KEY` 提供相同值，不写入 SQL 或验收报告。项目的 `scripts/test-api.sh` 可单独确认服务自身健康，但不能代替 DuckJeu live 验收。

`selfhosted` provider、独立可选凭据、255 个候选上限、截断响应拒绝及显式 live 验收脚本已经实现。部署 local-jev 服务后，在 DuckJeu 仓库根目录运行：

```sh
DUCKJEU_LOCAL_JEV_TEST=1 .venv/bin/python test/sql/run_local_acceptance.py
```

预期：`duckjeu_provider='selfhosted'` 在同一个现有 SQL 契约下对三函数各至少三行非敏感输入成功；记录请求模型、服务报告的实际模型、行映射、类型和值域；不支持能力/超过 255 个选项/截断状态明确失败。此项已有通过记录 [local-live.json](evidence/local-live.json)，无需为本次修复重复运行。若重新运行，模型用 `nli-deberta-large`；本机服务不支持 `jev-latest`。TypeSafe 与 local-jev 的概率不按数值相等验收。

## 4. v0.4 概况与四模式对照（API 已实现；离线/真实验收分开记录）

在连接中设置 `duckjeu_profile_enabled = true`，完成判断查询后另行执行：

```sql
SELECT duckjeu_last_profile();
```

预期：返回 [profile-report.md](contracts/profile-report.md) 定义的脱敏 JSON，含真实外部请求数、缓存命中、耗时、请求/实际模型及已知用量。TypeSafe 未报告服务推理时间时该字段为 null；费用依据不足时金额为 null。

当前实现尚未核查自定义 `duckjeu_api_url` 或模型的计价来源；即使端点是本地 stub，只要选择 `typesafe` 且返回完整用量，profile 也可能给出 USD 金额。修复前需自行核实该金额的计价依据。现有取消 SQL 检查只验证最终状态为 `cancelled`，未验证中断后停止发送请求。

profile SQL contract 使用本地 stub，不会调用真实服务：

```sh
make profile-contract
```

固定数据离线对照入口也只访问本地 stub，现已包含经过 live 验证的 TypeSafe composite adapter：

```sh
.venv/bin/python test/bench/run_compare.py --rows 1000,10000,100000 --output build/bench/runtime.json
```

离线证据覆盖 1K/10K/100K 的逐行、优化去重、预过滤、TypeSafe batch 和缓存模式，结果与实际请求数见 [benchmark-offline.json](evidence/benchmark-offline.json)。三种规模的 batch 分别观察到 8、71、710 次 stub HTTP，与逐行模式的 956、9,522、95,080 次相比均减少，且 profile 请求计数与 stub 一致。stub 时延/请求不能代表真实服务费用或性能；TypeSafe 真实服务多规模测试尚未执行，local-jev 实测见下文。

local-jev 真实服务对照需先启动固定源码服务，再显式打开本地 live gate。它不会访问外部计费 API；100K row 模式按当前固定种子会触发 95,080 次本地推理请求。该服务的跨状态 batch 未验证，因此脚本记录为 unsupported 并验证请求前失败；权重 revision 未知时跨查询缓存应继续无命中。已完成报告记录各模式的 HTTP 请求数、耗时、结果校验和未知费用原因。

```sh
# 另一个终端：local-jev 服务应已加载 nli-deberta-large
DUCKJEU_LOCAL_JEV_BENCH=1 .venv/bin/python test/bench/run_local_compare.py \
  --rows 1000,10000,100000 --resume
```

本次实际运行另启用离线权重环境变量以避免 Hub 网络访问：

```sh
DUCKJEU_LOCAL_JEV_BENCH=1 HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 \
  .venv/bin/python test/bench/run_local_compare.py \
  --rows 1000,10000,100000 --resume
```

实测结果与时间见 [benchmark-local-live.json](evidence/benchmark-local-live.json)。100K row、optimized dedup、prefilter 分别实测约 10,073.8 秒、1,171.5 秒和 5,626.5 秒；缓存对照两次查询均未命中并分别发出 9,999 次请求，因为权重 revision 未知时缓存保持关闭。完整三档需较长本地运行时间。

`--resume` 会跳过报告里已经完整记录的规模；从头复跑时请指定新的 `--output` 路径，以保留本次验收证据。

## 5. 完成判定

- v0.2：离线契约全部通过，且至少一个真实服务通过跨状态合批及请求数验收；TypeSafe 复合状态未通过时须另找经验证服务。
- v0.3：现有远端和真实 local-jev 均完成三函数 live 验收，核心在无 DuckDB 环境可验证；记录模型及能力差异。local-jev 资源已记录，实际加载权重 revision 未知并关闭跨查询缓存。
- v0.4：1K/10K/100K 四模式证据齐全，报告不泄漏敏感输入、不臆造推理耗时或账单金额。local-jev 合批明确 unsupported，未知权重 revision 下跨查询缓存为零命中；TypeSafe 多规模性能未测。

每一版验收后再更新 [roadmap.md](../../roadmap.md) 的实现状态。
