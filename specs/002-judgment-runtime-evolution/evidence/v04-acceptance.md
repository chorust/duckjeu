# v0.4 Profiling and Comparison Acceptance

Date: 2026-09-25

## Offline profile and four-mode comparison

- `make profile-contract`: SQL profile checks pass; see [profile.md](profile.md).
- `make bench`: fixed seed `20260923`, DuckDB Python 1.5.5, macOS arm64; the 1K/10K/100K local-stub report is [benchmark-offline.json](benchmark-offline.json).
- Row, optimized dedup, SQL prefilter, TypeSafe batch, and cache outputs matched. Row/optimized/batch HTTP attempts were 956/100/8, 9,522/1,001/71, and 95,080/9,999/710. Every batch profile reconciled with actual stub requests; the profile recorded 8/68/682 composite calls at those scales. Repeated cache queries made zero additional requests at each scale.
- Stub traffic is local and non-billable. Its response usage and timing do not represent a real provider bill or service performance.

## Real local-jev multi-scale comparison

- Environment: fixed seed `20260923`; DuckDB 1.5.5; macOS 26.6.2 arm64; local-jev 0.2.0 at pinned source commit `64a0b31ff343dca32142496cf9edca5a0174a18d`; requested/reported model alias `nli-deberta-large`; loaded weight revision unknown. Service endpoint was loopback `127.0.0.1:8765`; Hugging Face and Transformers offline mode were enabled.
- Command: `DUCKJEU_LOCAL_JEV_BENCH=1 HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 .venv/bin/python test/bench/run_local_compare.py --rows 1000,10000,100000 --resume`.
- All three scales passed row count/result mapping, optimized-vs-row equality, prefilter-vs-row-subset equality, zero-HTTP batch preflight, and cache-result equality. Times below are end-to-end query wall times in seconds; cache shows its two repeated queries.

| Rows | Valid / unique | Row HTTP / s | Optimized HTTP / s | Prefilter HTTP / s | Batch preflight HTTP / s | Cache HTTP / s (query 1 + 2) | Cross-query hits |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1,000 | 956 / 100 | 956 / 100.649 | 100 / 10.374 | 471 / 48.849 | 0 / 0.001 | 100 + 100 / 10.332 + 10.379 | 0 |
| 10,000 | 9,522 / 1,001 | 9,522 / 1,005.970 | 1,001 / 108.531 | 4,755 / 519.137 | 0 / 0.002 | 1,001 + 1,001 / 107.569 + 111.888 | 0 |
| 100,000 | 95,080 / 9,999 | 95,080 / 10,073.829 | 9,999 / 1,171.452 | 47,595 / 5,626.500 | 0 / 0.007 | 9,999 + 9,999 / 1,048.064 + 1,041.932 | 0 |

- local-jev does not have verified cross-state batch capability. The optimized batch path fails explicitly at capability preflight with zero HTTP requests at all sizes; it is recorded as unsupported, not as a successful batch measurement.
- Cross-query caching is disabled because the loaded weight revision is unknown. Each repeated cache query therefore performs the same number of external requests as optimized dedup; both result sets match the row baseline. This confirms fail-closed cache behavior, not a cache speedup.
- The service has no pricing metadata, so cost is unknown. No external billing API was called. The profile leaves unsupported timing/usage splits unknown; it does not infer server inference time from end-to-end latency.

## Live evidence and decision

- Direct TypeSafe semantic probe: 8 requests passed single-vs-composite Noul/Choice, reverse key order, and unrelated-row insertion checks; see [batch-live.json](batch-live.json).
- DuckJeu extension live SQL: 2 requests for 4 unique judgments passed; see [batch-extension-live.json](batch-extension-live.json).
- Real local-jev comparison completed for fixed-seed 1K/10K/100K inputs using `nli-deberta-large`; detailed measurements are above and in [benchmark-local-live.json](benchmark-local-live.json). The run targeted only `127.0.0.1` and incurred no remote API billing.
- local-jev cross-state batch remains explicitly unsupported and is rejected before HTTP. The cache made zero cross-query hits because the service reports only a model alias and the loaded weight revision is unverified; repeated-query outputs still matched the row baseline. These observations are the expected fail-closed behavior, not evidence of batch or cache speedups.
- The local service reports model alias `nli-deberta-large`, but not the loaded weight revision; cross-query cache is therefore intentionally disabled. User-supplied macOS `time -l` output records 703.1 MiB maximum RSS, 1.51 GiB peak memory footprint, and zero swaps; provenance is documented in [local-service.md](local-service.md). Inference usage may be reported by the server; local pricing is unavailable.
- Profile checks ensure state/question/choice/credential redaction; unsupported inference timing, network split, usage, and costs remain unknown.

**v0.4 is accepted** for the documented DuckDB v1.5.5 / macOS arm64 environment. The report includes offline four-mode and real local-jev multi-scale results, explicitly records unsupported batch, disabled cross-query cache, and unknown local pricing. TypeSafe multi-scale performance and actual provider billing were not tested; no claims are made about either.
