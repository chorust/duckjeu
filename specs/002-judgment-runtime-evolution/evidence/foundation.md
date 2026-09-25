# Foundation Evidence

**Date:** 2026-09-23  
**Platform:** macOS 26.6.2 arm64; Rust 1.92.0; DuckDB 1.5.5 / duckdb-rs 1.10505.0.  
**C++ headers:** exact DuckDB `v1.5.5` source. The lifecycle bridge is enabled with `DUCKDB_SOURCE_DIR` pointing to that source checkout.

## Implemented boundary

- One Rust `Arc<Mutex<ConnectionRuntime>>` is owned by each DuckDB `ClientContextState`; scalar-function initialization receives cloned handles from that connection state. The extension registration connection does not create a user runtime.
- First-query observation begins when the state is first attached; later `QueryBegin`/`QueryEnd` callbacks update the same runtime. End status uses the C API error type to distinguish success, failure, and interruption.
- Session settings are read for each scalar execution. Prepared statements therefore see settings changed after preparation.
- `duckjeu_execution_mode` defaults to `row`; `duckjeu_batch_enabled` defaults to false. Batch count (16), request bytes (262144), and in-flight (4) limits must be positive. Enabling batch in `row` mode fails before a provider request.
- Providers now declare binary, choice, and multi-state batch capabilities. At this 2026-09-23 foundational snapshot, TypeSafe multi-state batching was still `unknown`; the later live verification is recorded in [batch-capability.md](batch-capability.md). Mock supports multi-item batches for offline contract work. TypeSafe responses retain requested/reported model and available token usage; inference time remains unknown.

## Validation

| Command | Result |
| --- | --- |
| `DUCKDB_SOURCE_DIR=… DUCKDB_SOURCE_VERSION=v1.5.5 cargo test` | PASS — 29 tests (6 unit, 16 core contract, 7 provider protocol). |
| `DUCKDB_SOURCE_DIR=… DUCKDB_SOURCE_VERSION=v1.5.5 make contract-test` | PASS — 64/64 SQL checks. |

The SQL checks cover the original three-function/NULL/error contracts, positive and invalid optimization settings, row/batch conflict rejection before HTTP, prepared-statement settings re-read, and settings isolation between separate connections. Stub-server disconnect traces during timeout/oversized-response scenarios are expected; both commands exited successfully.

No optimization, cross-query cache, or real batch capability is claimed complete by this foundational evidence.
