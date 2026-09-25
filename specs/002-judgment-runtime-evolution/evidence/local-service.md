# local-jev Source and Runtime Evidence

Date: 2026-09-24

## Fixed source inspection

- Repository: [amithgc/local-jev](https://github.com/amithgc/local-jev/tree/64a0b31ff343dca32142496cf9edca5a0174a18d)
- Checked out and inspected source commit: `64a0b31ff343dca32142496cf9edca5a0174a18d`.
- `src/local_jev/api.py` exposes `POST /v1/systemone`; its request contains one `state`, model, and a map of named questions. The response schema contains a reported model, keyed answers, and input/output token usage.
- The server may return `x-local-jev-truncated: true` when it shortens long state. The adapter rejects that response before parsing or returning any answer.
- `LOCAL_JEV_API_KEY` is optional in the service. When configured, its middleware checks Bearer auth on `/v1/*`. DuckJeu reads the separate `DUCKJEU_LOCAL_JEV_API_KEY` environment variable and omits Authorization when unset.
- Source validates at most 255 choice labels. local-jev uses the requested/default model alias and does not expose a pinned weight revision in the inspected model card; DuckJeu therefore disables cross-query cache identity for this provider.
- The `nli-deberta-large` model card names `MoritzLaurer/deberta-v3-large-zeroshot-v2.0` and lists about 0.87 GB of weights.

## Runtime validation

**Status: live SQL passed; resource measurement recorded.** The run at `2026-09-24T16:04:17+0800` passed [local-live.json](local-live.json) against server version `0.2.0`, using the pinned checkout at commit `64a0b31ff343dca32142496cf9edca5a0174a18d`. The run used Darwin arm64/Python 3.11.6, requested and reported `nli-deberta-large`, and made 11 provider HTTP attempts. All three functions returned three valid rows each; the 256-choice preflight failed before HTTP, and two identical cache-enabled queries each made one request with zero cache hits.

- The server was not configured with `LOCAL_JEV_API_KEY` for this run; the evidence records `local_jev_credential_configured: false`. Live Bearer authentication with a configured key was therefore not exercised.
- The Hugging Face cache's `refs/main` pointed to snapshot `cf44676c28ba7312e5c5f8f8d2c22b3e0c9cdae2`, whose files were present before the live run. The service reported only the alias `nli-deberta-large`, so the cache snapshot is evidence of the local revision available at run time, not a revision attested by the service response.
- Per-function elapsed times are in `local-live.json`. The accompanying macOS `time -l` output reported maximum resident set size `737263616` bytes (703.1 MiB), peak memory footprint `1617560920` bytes (1.51 GiB), and `0` swaps. The pasted timing output did not include its command line or timestamp, so these are recorded as user-supplied process measurements associated with this local-service validation, not as model-weight-only memory.

The explicit acceptance script is [run_local_acceptance.py](../../../test/sql/run_local_acceptance.py); it requires `DUCKJEU_LOCAL_JEV_TEST=1` and writes sanitized output to [local-live.json](local-live.json). Repeat it with `nli-deberta-large` only if refreshing the live evidence or measuring resource use.

The functional local-service gate and T040 resource characterization are complete. The loaded weight revision remains explicitly unknown because the service reports only the model alias; cross-query caching is disabled for that reason. Cross-state batching remains `unsupported` for local-jev.
