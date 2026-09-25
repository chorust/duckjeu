# US1 Evidence: Full-Identity Deduplication and Row Mapping

Date: 2026-09-23

## Implemented behavior

- `JudgmentIdentity` compares canonical version, state kind and complete canonical bytes; question kind and source text; ordered choices; provider namespace, endpoint, requested/effective model; and opaque connection/query scopes. Credentials are not stored in identities.
- Canonical states expose a self-describing byte envelope (`DJST`, version, state-kind tag, payload length, full payload). STRUCT fields remain name-sorted; text stays raw for provider serialization.
- The optimized path collects and validates each complete DataChunk before issuing requests or writing results. It groups by full identity, maps each group back to its function invocation/chunk/row targets, and shares only successful results until QueryEnd. QueryEnd clears those temporary results.
- Row mode remains the default. At the time of this 2026-09-23 US1 checkpoint, explicit TypeSafe batch mode failed closed pending live capability verification; see the later [batch capability evidence](batch-capability.md). Cross-query caching is not part of this checkpoint.
- The local stub exposes an actual HTTP request counter and scripted success, delay and error scenarios. Client disconnects caused by timeout scenarios are suppressed in its test log.

## Verification

Commands run:

```text
DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5-headers DUCKDB_SOURCE_VERSION=v1.5.5 cargo test
.venv/bin/python test/sql/run_runtime_contract_tests.py
.venv/bin/python test/sql/run_contract_tests.py
```

Results:

- Rust: 35 tests passed (7 unit, 16 core contract, 5 identity, 7 provider protocol); 0 failed. The identity suite also checks that the executor rejects wrong result types and out-of-domain answers before reuse or writeback.
- US1 SQL runtime: 7/7 passed. Across 2,500 ordered rows and a DataChunk boundary, `jev_prob`, `jev_bool`, and `jev_choice` optimized outputs matched row-mode outputs exactly. The local server observed 7,500 HTTP attempts in row mode and 4 in optimized mode: two distinct noul identities shared by probability/bool, and two choice identities.
- Empty input and top-level NULL sent zero requests. Invalid input failed before any provider request.
- Existing SQL contract: 64/64 passed, including the 5,000-row row-mode mapping and request-count checks.
- All SQL requests went to the local stub. No external live service was called for this evidence.

## Checkpoint

US1 passes its offline identity, row-alignment, request-count, NULL, and invalid-input checks. This does not establish cross-state batching, connection-level concurrency limits, or cross-query cache behavior; those remain for US2/US3 and the v0.2 acceptance gate.
