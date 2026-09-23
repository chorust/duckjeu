-- DuckJeu v0.1 离线演示（默认 mock provider，无网络、无凭据）。
-- 用法（推荐）：
--   make demo
-- 手动运行：
--   make release
--   make venv
--   .venv/bin/python examples/make_sample_data.py
--   mkdir -p build
--   sed "s|@@EXTENSION@@|$PWD/build/release/duckjeu.duckdb_extension|" examples/demo.sql > build/demo.sql
--   duckdb -unsigned -noheader -list -c ".read build/demo.sql"
--
-- 说明：macOS 的 hardened runtime 不允许相对路径 LOAD，因此 `make demo`
-- 会把 @@EXTENSION@@ 替换为绝对路径后写到 build/demo.sql 再执行。

LOAD '@@EXTENSION@@';

-- 1) 逐行概率：模型判断不是已验证事实，请按业务门槛自行比较。
SELECT id,
       jev_prob(message, 'the customer requests a refund') AS refund_probability
FROM read_parquet('examples/data/tickets.parquet')
ORDER BY id;

-- 2) 布尔判断：v0.1 固定 p >= 0.5 为 true。
SELECT count(*) AS refund_like_tickets
FROM read_parquet('examples/data/tickets.parquet')
WHERE jev_bool(message, 'the customer requests a refund');

-- 3) 结构化状态 + 限定类别：返回候选集内的单个标签。
SELECT jev_choice(
           struct_pack(price := price, load := load, wind := wind),
           'which market regime best describes this state?',
           ['normal', 'scarcity', 'oversupply']
       ) AS regime,
       price
FROM read_parquet('examples/data/market.parquet');

-- 4) 顶层 NULL 短路：结果为 NULL 且不调用 provider。
SELECT jev_prob(NULL, 'anything') AS null_state,
       jev_choice('state', 'question', NULL) AS null_choices;
