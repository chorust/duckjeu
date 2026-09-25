# v0.3 Provider Abstraction Acceptance

Date: 2026-09-24

## Offline implementation

- `crates/judgment-core` owns judgment types, canonical serialization, result validation, provider contracts, cache rules, execution interfaces, and profile metrics without a DuckDB dependency. The root extension injects the DuckDB connection runtime through an adapter.
- TypeSafe uses shared provider/result metadata and retains reported model and usage. Multi-state batching is verified for reported effective model `jev-1.13.0`; other models remain `unknown` until separately tested.
- `selfhosted` adapts the pinned local-jev `POST /v1/systemone` protocol. It has an independent optional `DUCKJEU_LOCAL_JEV_API_KEY`, rejects truncated state and choices beyond 255 before request, reports cross-state batching as unsupported, and disables cross-query caching while weights are unpinned.
- Source inspection at commit `64a0b31ff343dca32142496cf9edca5a0174a18d` is recorded in [local-service.md](local-service.md). Source compatibility does not establish behavior of a running service or identify the downloaded weight revision.

## Verification and live status

- `cargo test --workspace`: 69 passed. `make contract-test`: 64/64; `make runtime-contract`: 33/33; `make profile-contract`: 14/14.
- TypeSafe's current three-function live acceptance passed on 2026-09-24; actual model reported was `jev-1.13.0`. The direct batch probe and DuckJeu extension SQL probe also passed; see [batch-live.json](batch-live.json), [batch-extension-live.json](batch-extension-live.json), and the current [live acceptance](../../../build/acceptance/live-20260924T144622.json).
- The fixed local-jev checkout passed its three-function live SQL acceptance on 2026-09-24; see [local-service.md](local-service.md) and [local-live.json](local-live.json). The server reported `nli-deberta-large`; the local cache snapshot and its evidence limits are recorded there.
- Local-jev's optional Bearer credential was not enabled during the live run. User-supplied `time -l` measurements associated with the run report 703.1 MiB maximum RSS, 1.51 GiB peak memory footprint, and zero swaps; provenance limits are recorded in [local-service.md](local-service.md). The service reports only the model alias, so the loaded weight revision is unknown and cross-query caching stays disabled. Its cross-state batch capability remains unsupported.

## Decision

The shared-core, adapter, and both providers' three-function live behavior passed. **v0.3 is accepted.** Resource measurements are recorded; the loaded local weight revision is explicitly unknown, so cross-query caching remains disabled. Do not compare local-jev probabilities directly with TypeSafe probabilities. See [local-service.md](local-service.md) and [batch-capability.md](batch-capability.md).
