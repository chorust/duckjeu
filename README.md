# duckjeu

English | [简体中文](README_CN.md)

`duckjeu` is a DuckDB extension for running JEV (Judgment-Enabled Vector) judgments on table or Parquet data and using the typed results in regular SQL. It uses row mode by default and offers an opt-in optimized mode.

```sql
SELECT jev_bool(message, 'the customer requests a refund') FROM tickets;

SELECT jev_choice(
           struct_pack(price := price, load := load, wind := wind),
           'which market regime best describes this state?',
           ['normal', 'scarcity', 'oversupply']
       ) AS regime
FROM read_parquet('market.parquet');
```

> **Model judgments are not verified facts.** v0.1 does not guarantee business calibration. Treat results as model signals and choose thresholds for your use case; use `jev_prob` for an explicit threshold.
>
> **Data and cost:** With `duckjeu_provider='typesafe'`, the state, criterion or question, and choice labels are sent to the configured service and real requests may be billed. DuckDB may evaluate expressions more than once, and a failed query does not undo requests already made. Filter rows before calling the provider, and do not send sensitive data unless authorized.

## Public API (v0.1)

| Function | Arguments | Return | Contract |
| --- | --- | --- | --- |
| `jev_prob(state, criterion)` | STRUCT or VARCHAR state; non-empty VARCHAR criterion | DOUBLE / NULL | Probability that a binary criterion holds, constrained to [0, 1] |
| `jev_bool(state, criterion)` | Same as above | BOOLEAN / NULL | Applies `p >= 0.5` (inclusive); each invocation is independent and does not share a request with `jev_prob` |
| `jev_choice(state, question, choices)` | State, non-empty VARCHAR question, at least two distinct VARCHAR choices | VARCHAR / NULL | Returns one label from `choices` based on `question` |

- **STRUCT state** is normalized into a stable representation: fields are sorted by name, carry type labels, and distinguish SQL NULL from a missing field. Supported fields are BOOLEAN, integers, DECIMAL, HUGEINT, finite FLOAT/DOUBLE, VARCHAR, and NULL. Unsupported types such as DATE/TIMESTAMP, nested LIST/STRUCT, and BLOB produce an explicit error.
- **VARCHAR state** is sent as its original text, without wrapping.
- A top-level **NULL** (`state`, `criterion`, `question`, or `choices`) returns NULL and does not call the provider. NULL fields inside a STRUCT are preserved as NULL.
- An empty input dataset produces no requests.

## Install and configure

Requirements: macOS arm64 (the first verified platform), Rust >= 1.85, and DuckDB CLI **v1.5.5**. Python 3 is used by the demo and test targets.

Initialize the packaging helper and build the extension:

```bash
git submodule update --init          # extension-ci-tools packaging scripts
make release
```

The extension is created at `build/release/duckjeu.duckdb_extension`. v0.1 is unsigned, so allow unsigned extensions when loading it:

```bash
duckdb -unsigned -cmd "LOAD '$PWD/build/release/duckjeu.duckdb_extension'"
```

macOS hardened runtime requires an absolute path in `LOAD`. Alternatively, copy the extension to `~/.duckdb/extensions/v1.5.5/osx_arm64/` and run `LOAD duckjeu`.

`duckdb-rs` uses the unstable C API. The extension uses the `C_STRUCT_UNSTABLE` ABI and is **only guaranteed to work with DuckDB v1.5.5**.

Settings are connection scoped, are not shared between connections, and are read on each execution:

| Setting | Type | Default | Description |
| --- | --- | --- | --- |
| `duckjeu_provider` | VARCHAR | `mock` | `mock`, `typesafe`, or `selfhosted`; the latter two explicitly enable service requests |
| `duckjeu_api_url` | VARCHAR | `https://api.typesafe.ai/v1/systemone` | Service endpoint; may point to a local stub for tests |
| `duckjeu_local_jev_api_url` | VARCHAR | `http://127.0.0.1:8765/v1/systemone` | Default endpoint for `selfhosted`; `duckjeu_api_url` can override it |
| `duckjeu_model` | VARCHAR | empty; resolved by provider | Model identifier; explicit values are preserved for every provider |
| `duckjeu_timeout_ms` | BIGINT | `30000` | Total timeout for one request |
| `duckjeu_max_response_bytes` | BIGINT | `1048576` | Maximum response body size |

Credentials are read from the environment before DuckDB starts. There is no SQL setting for credentials, and they are not included in errors or logs. `DUCKJEU_API_KEY` takes precedence over `TYPESAFE_API_KEY`:

```bash
export DUCKJEU_API_KEY=...          # or export TYPESAFE_API_KEY=...
```

## Usage

After loading the extension, explicitly enable the real provider to make real requests:

```sql
SET duckjeu_provider = 'typesafe';
SELECT jev_prob(message, 'the customer requests a refund') FROM tickets;
```

### Live service CLI showcase

The [live demo SQL](examples/live_demo.sql) calls the real provider for three non-sensitive rows with each function, then shows top-level NULL behavior. It makes up to nine judgment requests; provider billing applies, and DuckDB may evaluate expressions more than once.

Run from the repository root after setting an API key in your shell:

```bash
export DUCKJEU_API_KEY=...          # or TYPESAFE_API_KEY
make release venv
.venv/bin/python examples/make_sample_data.py
mkdir -p build
sed "s|@@EXTENSION@@|$PWD/build/release/duckjeu.duckdb_extension|" examples/live_demo.sql > build/live_demo.sql
duckdb -unsigned -box -c ".read build/live_demo.sql"
```

One real service run produced this output. Probabilities and labels may differ on later runs:

```text
┌───────┬────────────────────┐
│  id   │ refund_probability │
│ int32 │       double       │
├───────┼────────────────────┤
│     1 │               0.98 │
│     2 │               0.01 │
│     3 │               0.04 │
└───────┴────────────────────┘
┌───────┬─────────────┐
│  id   │ refund_like │
│ int32 │   boolean   │
├───────┼─────────────┤
│     1 │ true        │
│     2 │ false       │
│     3 │ false       │
└───────┴─────────────┘
┌────────┬────────────┐
│ price  │   regime   │
│ double │  varchar   │
├────────┼────────────┤
│  95.75 │ oversupply │
│  98.25 │ oversupply │
│  101.5 │ normal     │
└────────┴────────────┘
┌────────────┬──────────────┐
│ null_state │ null_choices │
│   double   │   varchar    │
├────────────┼──────────────┤
│       NULL │ NULL         │
└────────────┴──────────────┘
```

Timeouts, authentication failures, HTTP errors, missing or extra responses, type errors, out of range probabilities, unknown labels, and unsupported provider capabilities make the query fail. v0.1 does not silently retry or turn failures into 0, false, or an empty label. HTTP failure messages expose only the status and a fixed hint; they do not include credentials, raw state, or the remote response body.

## Architecture

```
DuckDB host           src/lib.rs, src/functions/, src/host/, src/config.rs
  ↓ Registration, vectors, query lifecycle, connection configuration
Shared judgment core  crates/judgment-core/src/
  ↓ State encoding, identity, validation, execution, cache, profile aggregation
  (no DuckDB dependency)
Provider adapters     src/provider/{mock,typesafe,localjev}.rs
  ↓ Service protocol, authentication, capabilities, response conversion
```

The shared core can be verified without a database process or network (`cargo test`). Batching, caching, provider adapters, and profiling are implemented on these boundaries. PostgreSQL hosting, judgments over row sets, and JDL remain roadmap stages. See [section 9 of spec.md](spec.md#9-面向-roadmap-的架构约束) for details.

## v0.2 execution work

Connection-scoped optimized execution uses DuckDB's internal `ClientContext` bridge. Build it against the exact DuckDB v1.5.5 source tree:

```bash
export DUCKDB_SOURCE_DIR=/path/to/duckdb-v1.5.5
export DUCKDB_SOURCE_VERSION=v1.5.5
make release
```

`duckjeu_execution_mode` defaults to `row`. Set it to `optimized` to deduplicate identical judgments within a query and preserve row mapping. `duckjeu_max_inflight` defaults to 4. Explicit batch settings are `duckjeu_batch_enabled` (default `false`), `duckjeu_batch_max_judgments` (16), and `duckjeu_max_request_bytes` (262144); the request byte limit applies only in `optimized` mode. TypeSafe cross-state batching is verified for responses reporting effective model `jev-1.13.0`; `jev-latest` is accepted as the request alias only while it resolves to that verified model. Other or drifted model responses fail closed. Batching remains opt-in. Composite state exposes the other rows to each question, so outputs may differ from single-row context; see the [live capability evidence](specs/002-judgment-runtime-evolution/evidence/batch-capability.md).

**Known cancellation limit:** Interruption is currently propagated to request waiters at query cleanup. A row-mode query interrupted during a slow provider call may continue sending requests for the rest of its chunk before it stops. Requests already sent cannot be undone.

Cross-query caching is disabled by default. In optimized mode, set `duckjeu_cache_enabled=true`; defaults are 10,000 entries, 64 MiB estimated memory, and a 60,000 ms TTL. The cache is isolated by connection and credential scope. `duckjeu_cache_clear()` clears only the current connection and returns the number of entries removed. TypeSafe caching requires a versioned model identifier such as `jev-1.13.0` and a matching model in the response. Mutable aliases such as `jev-latest` are not reused across queries. Cache entries are memory-only and contain validated successes.

The `selfhosted` provider adapts local-jev's `/v1/systemone` protocol. When `duckjeu_model` is not explicitly set, its default model is `nli-deberta-large`; TypeSafe and mock use `jev-latest`. Any explicit model value, including `jev-latest`, is sent as configured. It reads optional credentials only from `DUCKJEU_LOCAL_JEV_API_KEY`, rejects truncated responses and more than 255 choices, disables cross-query caching while the model weight revision is unknown, and reports multi-state batching as unsupported until separately verified. Run the server yourself; DuckJeu does not download model weights or start a service.

`duckjeu_profile_enabled` defaults to `false`. Set it to `true` before a judgment query to include serialization and client round-trip timing, then read `SELECT duckjeu_last_profile()` for the connection's most recent ended judgment query, including failed or cancelled queries. The versioned JSON contains aggregate counts, timings, usage, and a cost estimate; it excludes state, questions, choices, endpoints, credentials, and provider error text. Missing token usage, failed requests that may be billable, and unsupported server timing remain `null` with a reason. The separate stub benchmark reports unknown cost.

**Known pricing limit:** The current profile applies the published TypeSafe input-token rate whenever the configured provider is `typesafe` and usage is complete. It does not verify the endpoint or model's pricing. If `duckjeu_api_url` points to another service or stub, a reported USD amount is not a supported cost estimate. Treat the amount as unverified until pricing provenance is checked; see the [profile contract](specs/002-judgment-runtime-evolution/contracts/profile-report.md).

Offline runtime contracts and the fixed-size benchmark use a local scripted HTTP service:

```bash
make runtime-contract
make profile-contract
make bench
```

The direct TypeSafe protocol probe and the low-volume DuckJeu extension SQL probe require separate explicit gates. The latter sends exactly two requests for four synthetic judgments:

```bash
DUCKJEU_BATCH_LIVE_TEST=1 make batch-live
DUCKJEU_BATCH_EXTENSION_LIVE_TEST=1 make batch-extension-live
```

Both probes passed on 2026-09-24. The direct protocol probe used 8 HTTP requests; the DuckJeu extension SQL probe used 2 and confirmed result mapping and profile counts. The first attempt had a Python CA trust issue, resolved by installing/configuring `certifi`; no judgment responses were received during that attempt. See the [probe evidence](specs/002-judgment-runtime-evolution/evidence/batch-capability.md).

For an independently started local-jev service, `make local-live` requires `DUCKJEU_LOCAL_JEV_TEST=1`. The endpoint defaults to loopback; `DUCKJEU_LOCAL_JEV_API_URL` can override it. The script checks service health before running SQL acceptance.

See the [current roadmap](roadmap.md) and [Spec 002 evidence](specs/002-judgment-runtime-evolution/evidence/) for milestone status. Earlier v0.2–v0.4 acceptance evidence covers the verified environment; v0.2 cancellation and v0.4 pricing need fixes and renewed review. Real local-jev 1K/10K/100K comparisons passed; its loaded weight revision is unknown, so cross-query caching stays disabled, and cross-state batching is unsupported. Local pricing and TypeSafe multi-scale performance remain unknown or unmeasured.

## Platforms and limitations

- Verified only on macOS arm64 with DuckDB v1.5.5; Linux and Windows have not been verified.
- TypeSafe multi-state batching is verified only for effective model `jev-1.13.0`; local-jev live SQL was verified with `nli-deberta-large`, while cross-state batching remains unsupported.
- The 0.5 threshold in `jev_bool` is a fixed contract and does not imply business calibration.
- See the [provider protocol](specs/001-judgment-foundation/contracts/provider-protocol.md) and [SQL contract](specs/001-judgment-foundation/contracts/sql-api.md) for details.

## Development & testing

The offline `make demo` target uses the mock provider and does not contact a service. It creates the sample Parquet files and the project virtual environment as needed:

```bash
make demo
```

Run the Rust and SQL contract suites with:

```bash
make test            # Rust contracts; no database or network required
make contract-test   # v0.1 SQL contracts and failure paths using a local HTTP stub (64 checks)
make runtime-contract # optimized runtime, batch preflight, cache and connection contracts (local stub)
make profile-contract # redacted query profile SQL contracts (local stub)
```

The SQL suite installs `duckdb==1.5.5` in the project virtual environment. It covers text and STRUCT state, NULL behavior, empty datasets, probability boundaries, boolean thresholds, choice membership, row alignment across 5,000 rows and DataChunk boundaries, connection scoped settings, prepared statement reruns, and provider failure paths. The runtime suite covers deduplication, explicit batch capability checks, bounded cache behavior, credential rotation, and connection isolation using only a local stub.

Live service acceptance is opt-in and makes real requests:

```bash
export DUCKJEU_API_KEY=...          # or TYPESAFE_API_KEY
DUCKJEU_LIVE_TEST=1 make live-test
```
