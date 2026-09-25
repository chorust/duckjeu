# Profile Contract Evidence

Date: 2026-09-24

## Implemented report

`duckjeu_last_profile()` returns the current connection's latest completed judgment query as versioned, redacted JSON. `duckjeu_profile_enabled` is connection scoped and defaults to `false`; it controls serialization and external round-trip detail while retaining basic query counters. Reading the profile does not replace the saved query. A query with no judgment rows returns `NULL`.

The report aggregates input/null/valid rows, unique and deduplicated judgments, cache hits, provider invocations, actual external attempts, batch sizes, failures, reported model identifiers, available usage, and query wall time. Serialization and client-observed HTTP round-trip are separate sums. Server inference is emitted only with provider-reported timing; pure network time remains `null` without direct evidence. Concurrent request sums are not clamped to query wall time.

No state, criterion/question, choices, service endpoint, credential, or provider error body is serialized. TypeSafe estimated cost uses reported input tokens and the publicly documented rate/source/date embedded in the JSON. Missing usage or any possibly billable failed request makes the amount unknown; other providers do not inherit TypeSafe pricing. Estimates are not invoices.

## Verification

```text
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5-headers DUCKDB_SOURCE_VERSION=v1.5.5 cargo test --workspace
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make profile-contract
```

- Rust workspace: 69 passed, including core metrics and provider-specific default model selection.
- SQL profile contract: 14/14 passed. It covers empty and isolated connections, aggregation across all three functions, full-identity deduplication, model metadata, input/credential redaction, unknown usage/cost/timing, read-without-overwrite, detail-off counters, failed queries, interruption status, local-jev's 255-choice preflight with zero HTTP requests, explicit self-hosted model forwarding, 50KB row-mode input, and profile refresh after optimized input validation failure.
- The lifecycle contract uses DuckDB's observed `QueryEnd` success value (`error_type=0`); invalid input is failure and `DUCKDB_ERROR_INTERRUPT` is cancellation.
- All provider traffic in this contract went to the local scripted HTTP stub. No live provider request was made by the profile test.

## Acceptance

Offline profile API and metrics contracts pass. No claim is made about live provider inference timing or a reconciled TypeSafe bill.
