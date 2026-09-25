# DuckDB Connection Lifecycle Probe

**Date:** 2026-09-23  
**Runtime:** DuckDB CLI and Python 1.5.5, macOS arm64  
**Headers:** DuckDB source tag `v1.5.5`, `src/include`  
**Build:** `DUCKDB_SOURCE_DIR=/tmp/duckdb-v1.5.5-headers make release` — PASS; extension loaded into Python DuckDB 1.5.5.

## Observed lifecycle

The probe registered a `ClientContextState` through the exact-version C++ `RegisteredStateManager` and executed the loaded extension on one Python connection.

| Scenario | Observed callback/output |
| --- | --- |
| First valid `jev_prob` query | `QueryBegin`, then `QueryEnd error_type=0` |
| Invalid criterion | `QueryBegin`, then `QueryEnd error_type=32` (`DUCKDB_ERROR_INVALID_INPUT`) |
| `Connection.interrupt()` during a long query | `QueryBegin`, then `QueryEnd error_type=29` (`DUCKDB_ERROR_INTERRUPT`) |
| Close connection after use | `state-destroyed` once |

Representative trace:

```text
[duckjeu-probe] QueryBegin
[duckjeu-probe] QueryEnd error=0 error_type=0 interrupted=0
[duckjeu-probe] QueryBegin
[duckjeu-probe] QueryEnd error=1 error_type=32 interrupted=1
[duckjeu-probe] QueryBegin
[duckjeu-probe] QueryEnd error=1 error_type=29 interrupted=1
[duckjeu-probe] state-destroyed
```

The first raw attempt showed that DuckDB calls `QueryBegin` before scalar-function initialization registers the state; a newly registered state would receive that query's `QueryEnd` but miss its `QueryBegin`. The probe was adjusted to start the first observation window when it creates the state. Later queries receive DuckDB's native begin/end callbacks. The state was not created during extension initialization; it was created from a scalar function's real `ClientContext` and destroyed with that connection.

`ClientContext::interrupted` was true for both the ordinary input error and cancellation, so it cannot classify cancellation. Calling C++ `ErrorData::Type()` directly failed to link because DuckDB does not export that internal symbol to the loadable extension. The successful probe copies the callback's `ErrorData` into DuckDB's C API error wrapper and calls `duckdb_error_data_error_type` through the initialized extension API table; this distinguished invalid input (32) from interruption (29) without an unresolved C++ symbol.

## Gate result and implementation constraint

**T002: PASS with the first-query adjustment above.** Connection-owned state, deterministic destruction, successful/error/cancelled query completion, and cancellation classification are observable. Cross-query cache/profile work may proceed only through this connection-bound state. The bridge uses DuckDB's internal C++ headers and C API wrapper layout, so it must be compiled against the exact DuckDB v1.5.5 headers and must not be used with another ABI version.
