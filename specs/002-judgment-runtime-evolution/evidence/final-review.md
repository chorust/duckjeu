# Spec 002 Implementation Review

Date: 2026-09-25

## Completed and verified

- v0.2 implementation: full-identity deduplication and row mapping, bounded connection cache, shared concurrency permits, TypeSafe composite batching for the observed effective model, batch splitting, and deterministic local HTTP contracts.
- v0.3 implementation: host-independent Rust judgment core, TypeSafe metadata adapter, and local-jev adapter with independent credentials and explicit capability limits.
- v0.4 implementation: query-scoped redacted profile, available timing/usage/cost aggregation, fixed-seed 1K/10K/100K offline comparison including TypeSafe-shaped batch requests, and real local-jev comparison at all three sizes.
- Live TypeSafe probes: direct Noul/Choice batch protocol passed; one DuckJeu SQL query produced 4 unique judgments in 2 external composite requests, with valid results and profile-reported effective model `jev-1.13.0`.
- Offline runtime SQL: 33/33; baseline SQL: 64/64; profile SQL: 14/14; Rust workspace: 69 passed. `make check` and workspace Clippy with `-D warnings` pass. Fixed benchmark correctness and observed/profile request counts match at 1K/10K/100K.

## Evidence limits and remaining unknowns

- Local-jev three-function SQL acceptance passed on the pinned source and `nli-deberta-large`; see [local-live.json](local-live.json). The optional live credential was not configured, and the service reports only the model alias. Resource measurements are recorded in [local-service.md](local-service.md); the loaded weight revision remains unknown, so cross-query cache is disabled. v0.3 is accepted.
- The real local-jev 1K/10K/100K comparison passed all correctness and preflight gates; see [benchmark-local-live.json](benchmark-local-live.json). At 100K it made 95,080 row, 9,999 optimized, and 47,595 prefilter HTTP attempts. Batch is explicitly unsupported and rejected before HTTP; cache made no cross-query hits because the loaded weight revision is unknown. Local pricing is unavailable, and remote TypeSafe multi-scale performance was not measured. v0.4 is accepted for the documented environment and evidence scope.
- TypeSafe batch verification is scoped to effective model `jev-1.13.0`; any other or missing response model fails closed.

## Reproducible verification commands

```text
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make check
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 cargo test --workspace
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make contract-test
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make runtime-contract
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make profile-contract
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make bench
DUCKJEU_LOCAL_JEV_BENCH=1 HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 .venv/bin/python test/bench/run_local_compare.py --rows 1000,10000,100000 --resume
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 cargo clippy --workspace --all-targets -- -D warnings
```

No credentials or judgment inputs are included in the checked-in evidence. No code was committed.
