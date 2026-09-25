# US3 Evidence: Connection-scoped bounded cache

Date: 2026-09-23

## Implemented behavior

- Cache is disabled by default and requires optimized execution. Entries contain only fully validated successes and are isolated to the DuckDB `ClientContext` runtime.
- LRU eviction enforces both entry and conservative byte limits. Positive TTL expires entries; expiration and eviction counters are available. `duckjeu_cache_clear()` clears only the current connection and returns the number of removed entries.
- Cache identity includes complete judgment contents, provider, endpoint, model, and an opaque credential scope. Credential rotation clears the connection cache. TypeSafe cache reuse is limited to explicit numeric `jev-x.y...` model versions whose response reports the same model; mutable aliases such as `jev-latest` remain uncached.
- Query snapshots count row and optimized execution inputs, nulls, unique/deduplicated judgments, cache hits, provider invocations, external requests, batches, batch sizes, and failed external requests.

## Verification

Commands run:

```text
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5-headers DUCKDB_SOURCE_VERSION=v1.5.5 cargo test
.venv/bin/python test/sql/run_runtime_contract_tests.py
.venv/bin/python examples/make_sample_data.py /tmp/duckjeu-sample-smoke --runtime-benchmarks
.venv/bin/python test/bench/run_compare.py --output specs/002-judgment-runtime-evolution/evidence/benchmark-offline.json
```

Results:

- Rust workspace: 65 passed, including success-only caching, TTL boundary, LRU and byte limits, credential-scope rotation, query counters, core, and metrics contracts.
- Runtime SQL: 25/25 passed. It verified cache hits across queries, TTL expiry, endpoint/model changes, `jev-latest` bypass, uncached failures, credential rotation, disabled-cache behavior, cache clearing, two-connection isolation, and a fresh connection after close.
- `make bench` regenerated the fixed-seed 1K/10K/100K local-stub report at [benchmark-offline.json](benchmark-offline.json). Optimized dedup requests were 100 / 1,001 / 9,999 versus 956 / 9,522 / 95,080 row-mode requests; batch requests were 8 / 71 / 710. All row, optimized, prefilter, batch, and cache outputs matched; profile request counts matched stub observations. The second cache query used zero requests at all sizes (100% observed hit rate).
- All benchmark traffic went to the local scripted stub. Cost is recorded as unknown because the stub is non-billable and supplies no price data.

## Acceptance status

Offline US3 implementation and benchmark requirements pass. Together with the later US2 live evidence, this supports the v0.2 acceptance recorded in [v02-acceptance.md](v02-acceptance.md).
