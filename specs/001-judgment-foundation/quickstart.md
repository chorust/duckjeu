# Quickstart: 构建、加载与验收 DuckJeu v0.1

目标：在干净环境中无需修改源码即可完成安装加载、离线演示与真实调用验收。对应 SC-001 ~ SC-005。
全部命令以仓库根为工作目录。

## 前置条件

- macOS arm64（首个验证平台；Linux/Windows 未验证）
- Rust ≥ 1.85（`rustup`；本机验证 1.92.0）
- DuckDB CLI **v1.5.5**（版本锁定，`duckdb --version` 确认）
- python3（仅测试与示例需要；`make venv` 会安装 `duckdb==1.5.5`）

## 构建与加载

```bash
git submodule update --init      # extension-ci-tools（打包脚本，固定 a285602bf745）
make release                     # cargo build --release + 追加扩展元数据
```

产出 `build/release/duckjeu.duckdb_extension`。v0.1 未签名，加载时允许 unsigned 扩展：

```bash
duckdb -unsigned -cmd "LOAD '$PWD/build/release/duckjeu.duckdb_extension'"
```

macOS 的 hardened runtime **不允许相对路径 LOAD**，必须使用绝对路径；或把扩展复制到
`~/.duckdb/extensions/v1.5.5/osx_arm64/` 后直接 `LOAD duckjeu`。

## 离线演示（mock，默认 provider，SC-001）

```bash
make demo
```

该目标生成非敏感样例 Parquet、把 `examples/demo.sql` 的 `@@EXTENSION@@` 替换为绝对路径写入
`build/demo.sql` 并执行。等价的手动步骤：

```bash
python3 examples/make_sample_data.py
mkdir -p build
sed "s|@@EXTENSION@@|$PWD/build/release/duckjeu.duckdb_extension|" examples/demo.sql > build/demo.sql
duckdb -unsigned -noheader -list -c ".read build/demo.sql"
```

预期：三个函数均成功；概率 ∈ [0,1]、布尔按 `p >= 0.5`、类别属于候选集；顶层 NULL 返回 NULL。

## 契约与错误路径测试（SC-002、SC-003）

```bash
make test            # 纯 Rust 契约测试（无数据库、无网络）
make contract-test   # SQL 契约 + 本地 HTTP stub（53 项检查）
```

覆盖内容：文本/STRUCT 状态、顶层与内部 NULL、空数据集零请求、概率边界 0/1、布尔阈值含边界、
类别成员校验、5000 行跨 DataChunk 行对齐、连接级配置隔离、prepared statement 重跑取新配置、
凭据仅经 Authorization header、以及越界概率、负值、非数值、未知标签、缺失/多余 answer、401、
超时、超大响应、大错误正文等失败路径；错误输出零凭据泄漏。

## 真实调用验收（SC-004）

```bash
export TYPESAFE_API_KEY=...        # 仅环境变量，不要写入 SQL 或日志
DUCKJEU_LIVE_TEST=1 make live-test
```

内容：三个函数各 3 行非敏感输入；记录 provider、请求 model 与响应回报的实际 model、行数、
请求数（v0.1 逐行推导）与耗时，写入 `build/acceptance/live-*.json`。缺少凭据或未显式开启时输出
`BLOCKED` 并写入 `build/acceptance/live-status.json`，**不标记为完成**。

## 设计评审（SC-005）

`specs/001-judgment-foundation/design-review.md` 逐项确认根规格 §9.2 七类演进方向的职责归属、
兼容性约束与本期边界（7/7），并确认基础契约不随 provider 切换改变。

## 常见问题

- **`relative path not allowed in hardened program`**：macOS 需要绝对路径 LOAD（见上）。
- **`duckjeu_provider='typesafe' requires the TYPESAFE_API_KEY environment variable`**：导出凭据后再运行；凭据没有 SQL 设置途径。
- **切换端点**（本地 stub 或代理）：`SET duckjeu_api_url='http://127.0.0.1:8000/v1/systemone'`。
- **不支持的 state 类型报错**：DATE/TIMESTAMP、嵌套 LIST/STRUCT、BLOB 等不在 v0.1 支持范围，会明确报错而非静默转换。
- **调用次数与行数不一致**：DuckDB 可能优化或重复执行表达式，v0.1 不承诺二者相等。
