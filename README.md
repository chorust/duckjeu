# duckjeu

English | [简体中文](README_CN.md)

`duckjeu` is a DuckDB extension for running JEV (Judgment-Enabled Vector) judgments on table or Parquet data, one row at a time, and using the typed results in regular SQL.

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
| `duckjeu_provider` | VARCHAR | `mock` | `mock` or `typesafe`; setting `typesafe` explicitly enables real requests |
| `duckjeu_api_url` | VARCHAR | `https://api.typesafe.ai/v1/systemone` | Service endpoint; may point to a local stub for tests |
| `duckjeu_model` | VARCHAR | `jev-latest` | Model identifier |
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
DuckDB host layer     src/lib.rs, src/functions/, src/config.rs
  ↓ Registration, NULL handling, chunk row mapping, output writing, connection configuration
Judgment semantics    src/serialize.rs, src/judgment.rs
  ↓ Typed state, stable encoding, request identity, response validation, threshold rules
  (independent of DuckDB and network access)
Provider layer        src/provider/{mod,mock,typesafe}.rs
  ↓ Service protocol, authentication, capabilities, response conversion
  (does not write to DuckDB vectors)
```

The judgment semantics layer can be verified without a database process or network (`cargo test`). Future roadmap stages—batching and caching, multiple providers, profiling, a PostgreSQL host, judgments over row sets, and JDL—build on these boundaries. See [section 9 of spec.md](spec.md#9-面向-roadmap-的架构约束) for details.

## Platforms and limitations

- Verified only on macOS arm64 with DuckDB v1.5.5; Linux and Windows have not been verified.
- v0.1 makes synchronous requests one row at a time, with no cache, batching, or concurrency control. Measure performance in your environment.
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
make contract-test   # SQL contracts and failure paths using a local HTTP stub (53 checks)
```

The SQL suite installs `duckdb==1.5.5` in the project virtual environment. It covers text and STRUCT state, NULL behavior, empty datasets, probability boundaries, boolean thresholds, choice membership, row alignment across 5,000 rows and DataChunk boundaries, connection scoped settings, prepared statement reruns, and provider failure paths.

Live service acceptance is opt-in and makes real requests:

```bash
export DUCKJEU_API_KEY=...          # or TYPESAFE_API_KEY
DUCKJEU_LIVE_TEST=1 make live-test
```
