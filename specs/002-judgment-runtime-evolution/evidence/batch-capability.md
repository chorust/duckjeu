# TypeSafe multi-state batch capability

Date: 2026-09-24

## Decision

**Status: verified for the currently observed effective model `jev-1.13.0`.** DuckJeu may send cross-state composite requests only when batch mode is explicitly enabled and the requested model is `jev-latest` or `jev-1.13.0`. Every successful multi-judgment response must itself report `model: jev-1.13.0`; a missing or different model fails closed. Other requested model identifiers remain `Unknown` and are rejected before HTTP.

The service does not expose a native array of independent states. DuckJeu constructs one composite `state.rows` plus a distinct question key for each row. Real probes confirmed Noul and Choice answer-key association, reversed response-key order, row-order sensitivity, and unchanged answers after inserting an unrelated leading row. Each question can see the entire composite state, so this is verified for the synthetic probes, not a guarantee that composite context is semantically identical for every possible input. Batch stays opt-in.

## Live evidence

- Direct protocol probe: [batch-live.json](batch-live.json), 8 external requests across single-row baselines and two composite variants. For both Noul and Choice, the composite request returned the same row-associated answers as the corresponding single-row calls; response keys were reversed and still mapped correctly. Inserting an unrelated leading row changed neither target answer. The Noul unpaid-vs-paid probability delta was 0.97. All responses reported `jev-1.13.0`.
- DuckJeu extension end-to-end: [batch-extension-live.json](batch-extension-live.json), one SQL query, 4 unique judgments, exactly 2 external requests, both carrying 2 judgments. The query returned Noul probabilities `[0.02, 0.98]`, booleans `[false, true]`, and choices `[bird, shark]`. Profile reported `jev-1.13.0`, zero failed requests, and batch-size histogram `{"2": 2}`.
- No credential value, raw state, question, or provider response body is included in saved evidence.

An earlier attempt failed at Python TLS certificate verification before receiving responses. Installing `certifi` and using its CA bundle resolved that transport setup issue; the successful records above replace that inconclusive attempt. It was not evidence of unsupported service semantics.

## Offline and fail-closed evidence

- `tests/provider_protocol.rs` covers composite construction, unique keys, compatible contexts, exact answer coverage, reversed-key mapping, usage/model metadata, byte estimation, and rejection if the reported effective model changes.
- The runtime SQL contract proves an unverified requested model fails before HTTP; with the verified model, three SQL functions preserve row mapping and four distinct judgments produce two composite requests.
- `make bench` exercises 1K/10K/100K synthetic rows through the same adapter using a local stub. It checks result equality and profile-vs-observed request counts; this is offline implementation evidence, not a live model-semantic test.

## Scope and revalidation

The live capability applies to the TypeSafe service at the recorded endpoint and effective model `jev-1.13.0`. Re-run the gated probes before enabling a different effective model, changing the composite mapping, or asserting behavior for materially different question/state formats. If the service reports another model, DuckJeu rejects that batch result instead of silently extending this verification.
