# US2 Evidence: Bounded batching and connection concurrency

Date: 2026-09-24

## Offline implementation

- The executor groups distinct identities only when provider capability is `Verified`, and splits requests by configured judgment count, exact TypeSafe serialized byte size, and provider limits.
- Each batch gets unique request keys. The executor verifies exact key coverage and result domains before returning any values; a failed batch is not retried.
- Connection-owned permits bound concurrent provider calls. Waits observe query cancellation; RAII releases permits after success or failure. Duplicate in-flight identities share one result.
- TypeSafe batching is verified only for `jev-1.13.0` effective responses. `jev-latest` is allowed as a requested model because the live service resolved it to that version; response-model drift fails closed. Other requested models remain unknown.

## Verification

Commands and evidence:

```text
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make runtime-contract
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5 DUCKDB_SOURCE_VERSION=v1.5.5 make bench
DUCKJEU_BATCH_LIVE_TEST=1 make batch-live
DUCKJEU_BATCH_EXTENSION_LIVE_TEST=1 make batch-extension-live
```

- Runtime SQL: 33/33 passed. It verifies unknown models fail before HTTP, optimized three-function row mapping, and four unique TypeSafe judgments using two composite HTTP requests. Row-mode `max_inflight=1` remains capped at one concurrent provider request.
- Direct real-service probe: 8 requests passed Noul and Choice single/composite/unrelated-row checks; all responses reported `jev-1.13.0`, answer keys arrived reversed, and row association remained correct. See [batch-live.json](batch-live.json).
- DuckJeu extension live SQL: exactly 2 external requests for 4 unique judgments; both were 2-judgment batches. Probabilities, boolean threshold results, choice labels, completion state, and reported model all passed. See [batch-extension-live.json](batch-extension-live.json).
- Rust workspace includes controlled HTTP tests for one-roundtrip batches, splitting, reversed correlation, missing answers, timeout/no retry, byte estimation, and the model-drift fail-closed check. Connection tests verify peak concurrency 2, cancellation, in-flight sharing, and permit release.
- The initial live probe's TLS setup failed before responses. The user installed `certifi`; the successful runs above used its CA bundle. Evidence contains no credential value.

## Acceptance status

US2 passes its live semantic, request-reduction, row-mapping, and concurrency gates for the observed TypeSafe effective model. Generic offline batching remains bounded and fail-closed for unverified models and local-jev. Local-jev's three-function live SQL, resource record, and 1K/10K/100K benchmark are complete; its loaded weight revision remains unknown and cross-query cache is disabled. Its cross-state batch remains unsupported and is rejected before HTTP. See [batch-capability.md](batch-capability.md) and [benchmark-local-live.json](benchmark-local-live.json).
