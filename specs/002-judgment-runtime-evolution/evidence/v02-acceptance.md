# v0.2 Acceptance Summary

Date: 2026-09-24

| Area | Offline result | Live result | Status |
| --- | --- | --- | --- |
| US1 full-identity deduplication and row mapping | Rust identity suite and SQL runtime checks pass; full identities keep state, context, provider, model, and credential scopes distinct | Not required for this offline checkpoint | Passed |
| US2 keyed batches and request splitting | Controlled HTTP validates split, reordered correlation, missing-response/timeout failure without retry; runtime SQL 33/33 passes | Direct TypeSafe probes pass for Noul and Choice; DuckJeu SQL sends 2 composite requests for 4 unique judgments, with correct row mappings and reported model `jev-1.13.0` | Passed for the verified effective model |
| US2 connection concurrency and cancellation | Rust tests verify peak two, cancellation, in-flight coalescing, permit release; runtime SQL verifies row-mode `max_inflight=1` | Not needed to characterize the connection-local cap | Passed |
| US3 bounded cache | Rust cache and SQL runtime contracts pass; failures are not cached, TTL/LRU/byte limits and cache clearing are enforced | Local stub only | Passed |
| Fixed-size comparison | Fixed-seed 1K/10K/100K local-stub row, optimized, prefilter, batch, and cache runs all preserve results and reconcile profile with observed requests | No benchmark calls to a billable service | Passed offline; costs remain unknown |

## Decision

**v0.2 is accepted for the recorded DuckDB v1.5.5 / macOS arm64 environment.** Real TypeSafe multi-state batching passed the Noul/Choice semantic and HTTP-count probes, including an end-to-end DuckJeu extension SQL query. Batching remains explicitly opt-in and fail-closed for any unverified effective model. Evidence: [batch-capability.md](batch-capability.md), [batch-live.json](batch-live.json), [batch-extension-live.json](batch-extension-live.json).

This acceptance does not by itself imply v0.3 or v0.4 completion. Those milestones have since been separately accepted: local-jev live behavior and resource measurement passed; its 1K/10K/100K comparison is recorded in [benchmark-local-live.json](benchmark-local-live.json). Its loaded weight revision remains unknown and cross-query cache is disabled.

## Commands and results

- `DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make runtime-contract` — 33/33 passed.
- `DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make bench` — fixed-seed 1K/10K/100K local-stub comparison passed. Row/optimized/batch HTTP attempts were 956/100/8, 9,522/1,001/71, and 95,080/9,999/710 respectively. All result comparisons and profile-to-stub request counts matched; cache repeat calls made zero requests.
- `DUCKJEU_BATCH_LIVE_TEST=1 make batch-live` — direct real TypeSafe semantic probes passed, 8 external requests.
- `DUCKJEU_BATCH_EXTENSION_LIVE_TEST=1 make batch-extension-live` — DuckJeu extension SQL passed, 2 external requests for 4 unique judgments.
- `DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 cargo test --workspace` — 69 tests passed (root extension and judgment-core contracts).
- `DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make check` — format, Clippy `-D warnings`, and root Rust tests passed.
- `DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make contract-test` — 64/64 passed.
- `DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make profile-contract` — 11/11 passed.
- `DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 cargo clippy --workspace --all-targets -- -D warnings` — passed.
