# v0.1 Baseline Evidence

**Date:** 2026-09-23  
**Platform:** macOS 26.6.2, arm64  
**DuckDB CLI/Python:** v1.5.5 (`duckdb==1.5.5` in `requirements-dev.txt`)  
**Rust/Cargo:** rustc 1.92.0, cargo 1.92.0  
**Rust DuckDB binding:** `duckdb` v1.10505.0 (`Cargo.lock`)

## Required commands

| Command | Result | Evidence |
| --- | --- | --- |
| `make release` | PASS | Built and packaged `build/release/duckjeu.duckdb_extension` for `osx_arm64`, DuckDB `v1.5.5`, ABI `C_STRUCT_UNSTABLE`. |
| `make demo` | PASS | Loaded the release extension and completed the offline sample SQL for probability, boolean, and choice outputs, including the NULL row. |
| `make contract-test` | PASS | `53/53 checks passed` against the local HTTP stub. Two expected client disconnect tracebacks were printed while timeout/oversized-response cases were exercised; the command exited 0. |

The live TypeSafe acceptance was not part of this baseline command set and was not run here.

## Existing SQL contract confirmed

- `jev_prob(state, criterion)` returns a numeric probability in `[0,1]`; boundary values 0 and 1 are accepted, while non-numeric, negative, and greater-than-one values fail.
- `jev_bool(state, criterion)` maps probabilities at or above `0.5` to `true`, and values below `0.5` to `false`.
- `jev_choice(state, question, choices)` returns one supplied candidate; empty/duplicate/NULL candidates, blank questions, and unknown labels fail.
- Top-level NULL arguments return NULL without sending an HTTP request. Internal STRUCT NULL values remain NULL, while absent fields remain absent.
- Empty input sends zero requests. Multi-row output remains aligned across DataChunks.
- Missing or extra answers, HTTP 401, timeout, and oversized responses fail the query. Error messages do not expose credentials, remote response bodies, or raw input state. Requests are not automatically retried.
- Provider defaults to `mock`; live TypeSafe calls require explicit provider configuration and credentials.

## Commands and environment

```text
macOS: sw_vers -productVersion -> 26.6.2
architecture: uname -m -> arm64
Rust: rustc --version -> rustc 1.92.0 (ded5c06cf 2025-12-08)
Cargo: cargo --version -> cargo 1.92.0 (344c4567c 2025-10-21)
DuckDB: duckdb --version -> v1.5.5 (Variegata) d8cdaa33fd
```
