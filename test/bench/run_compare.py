#!/usr/bin/env python3
"""Compare DuckJeu execution modes against the local scripted HTTP service only."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import platform
import sys
import time
from datetime import datetime, timezone
from typing import Any

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "examples"))
sys.path.insert(0, str(REPO_ROOT / "test"))

import duckdb  # noqa: E402

from make_sample_data import RUNTIME_SEED, generate_runtime_rows  # noqa: E402
from stub_server import StubServer  # noqa: E402

DEFAULT_EXTENSION = REPO_ROOT / "build" / "release" / "duckjeu.duckdb_extension"


def measure_query(con: duckdb.DuckDBPyConnection, query: str) -> tuple[list[tuple], float]:
    started = time.perf_counter()
    rows = con.execute(query).fetchall()
    return rows, (time.perf_counter() - started) * 1000


def run_size(extension: pathlib.Path, stub: StubServer, size: int, seed: int) -> dict[str, Any]:
    data, summary = generate_runtime_rows(size, seed)
    con = duckdb.connect(config={"allow_unsigned_extensions": "true"})
    try:
        con.execute(f"LOAD '{extension}'")
        con.execute("SET duckjeu_provider='typesafe'")
        con.execute(f"SET duckjeu_api_url='{stub.url('ok')}'")
        con.execute("SET duckjeu_model='jev-1.13.0'")
        con.execute("SET duckjeu_timeout_ms=5000")
        con.execute("SET duckjeu_max_response_bytes=1048576")
        con.execute("SET duckjeu_max_inflight=4")
        con.execute("CREATE TEMP TABLE benchmark_rows(id BIGINT, state VARCHAR, criterion VARCHAR)")
        con.executemany("INSERT INTO benchmark_rows VALUES (?, ?, ?)", data)
        query = "SELECT id, jev_prob(state, criterion) FROM benchmark_rows ORDER BY id"

        # Baseline: one provider call per valid input row.
        con.execute("SET duckjeu_execution_mode='row'")
        con.execute("SET duckjeu_batch_enabled=false")
        con.execute("SET duckjeu_cache_enabled=false")
        stub.reset()
        row_results, row_ms = measure_query(con, query)
        row_requests = stub.request_count

        # Optimized mode groups complete identities and preserves row positions.
        con.execute("SET duckjeu_execution_mode='optimized'")
        stub.reset()
        optimized_results, optimized_ms = measure_query(con, query)
        optimized_requests = stub.request_count

        # SQL prefiltering is measured before the judgment function runs.
        con.execute("SET duckjeu_execution_mode='row'")
        stub.reset()
        prefiltered_results, prefiltered_ms = measure_query(
            con,
            "SELECT id, jev_prob(state, criterion) FROM benchmark_rows "
            "WHERE id % 2 = 0 ORDER BY id",
        )
        prefiltered_requests = stub.request_count
        expected_prefilter = [row for row in row_results if row[0] % 2 == 0]

        # Exercise the exact composite adapter with the locally scripted TypeSafe protocol.
        con.execute("SET duckjeu_execution_mode='optimized'")
        con.execute("SET duckjeu_batch_enabled=true")
        con.execute("SET duckjeu_profile_enabled=true")
        stub.reset()
        batch_results, batch_ms = measure_query(con, query)
        batch_request_records = list(stub.requests)
        batch_requests = len(batch_request_records)
        batch_sizes: dict[str, int] = {}
        for request in batch_request_records:
            size = len(request["body"].get("questions", {}))
            if size > 1:
                batch_sizes[str(size)] = batch_sizes.get(str(size), 0) + 1
        batch_profile_raw = con.execute("SELECT duckjeu_last_profile()").fetchone()[0]
        batch_profile = json.loads(batch_profile_raw) if batch_profile_raw else {}
        con.execute("SET duckjeu_batch_enabled=false")
        con.execute("SET duckjeu_profile_enabled=false")

        # A second identical query measures cross-query cache hits. The model is pinned and
        # the stub reports the same model identifier, satisfying the cache identity contract.
        con.execute("SET duckjeu_cache_enabled=true")
        con.execute("SET duckjeu_cache_max_entries=20000")
        con.execute("SET duckjeu_cache_max_bytes=134217728")
        con.execute("SET duckjeu_cache_ttl_ms=3600000")
        con.execute("SELECT duckjeu_cache_clear()")
        stub.reset()
        cached_first, cache_first_ms = measure_query(con, query)
        cache_first_requests = stub.request_count
        cached_second, cache_second_ms = measure_query(con, query)
        cache_requests_after_second = stub.request_count - cache_first_requests
        cache_hits = max(0, summary["unique_judgments"] - cache_requests_after_second)

        optimized_ok = optimized_results == row_results
        batch_ok = batch_results == row_results
        cache_ok = cached_first == row_results and cached_second == row_results
        prefilter_ok = prefiltered_results == expected_prefilter
        batch_reduced = (
            batch_requests < summary["unique_judgments"]
            if summary["unique_judgments"] > 1
            else batch_requests == summary["unique_judgments"]
        )
        batch_metrics_ok = (
            batch_profile.get("counts", {}).get("external_requests") == batch_requests
            and batch_profile.get("counts", {}).get("unique_judgments")
            == summary["unique_judgments"]
            and batch_profile.get("counts", {}).get("batched_requests")
            == sum(batch_sizes.values())
        )
        return {
            "data": summary,
            "provider": "typesafe",
            "endpoint_kind": "local_scripted_stub",
            "requested_model": "jev-1.13.0",
            "modes": {
                "row": {
                    "status": "measured",
                    "correct": True,
                    "http_attempts": row_requests,
                    "elapsed_ms": round(row_ms, 3),
                },
                "optimized_dedup": {
                    "status": "measured",
                    "correct": optimized_ok,
                    "http_attempts": optimized_requests,
                    "elapsed_ms": round(optimized_ms, 3),
                },
                "prefilter": {
                    "status": "measured",
                    "correct": prefilter_ok,
                    "predicate": "id % 2 = 0",
                    "valid_output_rows": len(prefiltered_results),
                    "http_attempts": prefiltered_requests,
                    "elapsed_ms": round(prefiltered_ms, 3),
                },
                "batch": {
                    "status": "measured",
                    "capability": "verified_for_jev-1.13.0",
                    "correct": batch_ok,
                    "request_count_reduced": batch_reduced,
                    "profile_matches_observed_requests": batch_metrics_ok,
                    "http_attempts": batch_requests,
                    "batched_requests": sum(batch_sizes.values()),
                    "batch_size_histogram": batch_sizes,
                    "profile_counts": batch_profile.get("counts", {}),
                    "elapsed_ms": round(batch_ms, 3),
                },
                "cache": {
                    "status": "measured",
                    "correct": cache_ok,
                    "first_query_http_attempts": cache_first_requests,
                    "second_query_http_attempts": cache_requests_after_second,
                    "cache_hits_estimated_from_observed_requests": cache_hits,
                    "cache_hit_rate": round(cache_hits / max(1, summary["unique_judgments"]), 6),
                    "first_query_elapsed_ms": round(cache_first_ms, 3),
                    "second_query_elapsed_ms": round(cache_second_ms, 3),
                },
            },
            "correctness": {
                "optimized_equals_row": optimized_ok,
                "batch_equals_row": batch_ok,
                "batch_requests_reduced": batch_reduced,
                "batch_profile_matches_observed_requests": batch_metrics_ok,
                "prefilter_equals_row_subset": prefilter_ok,
                "cache_equals_row": cache_ok,
            },
            "cost_estimate": None,
            "cost_unknown_reason": "local_stub_is_not_a_billable_service_and_has_no_price_data",
        }
    finally:
        con.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--extension", type=pathlib.Path, default=DEFAULT_EXTENSION)
    parser.add_argument("--rows", default="1000,10000,100000", help="comma-separated sizes")
    parser.add_argument("--seed", type=int, default=RUNTIME_SEED)
    parser.add_argument("--output", type=pathlib.Path, default=REPO_ROOT / "build/bench/runtime.json")
    args = parser.parse_args()
    extension = args.extension.resolve()
    if not extension.exists():
        print(f"extension not found: {extension}\nrun `make release` first")
        return 2
    try:
        sizes = [int(value.strip()) for value in args.rows.split(",") if value.strip()]
        if not sizes or any(size <= 0 for size in sizes):
            raise ValueError
    except ValueError:
        print("--rows must contain positive comma-separated integers")
        return 2

    os.environ.setdefault("TYPESAFE_API_KEY", "local-stub-benchmark-token")
    with StubServer() as stub:
        results = []
        for size in sizes:
            print(f"measuring {size} rows against the local stub", flush=True)
            results.append(run_size(extension, stub, size, args.seed))
    report = {
        "schema_version": 1,
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "seed": args.seed,
        "duckdb_python_version": duckdb.__version__,
        "python_version": platform.python_version(),
        "platform": platform.platform(),
        "extension": str(extension.relative_to(REPO_ROOT))
        if extension.is_relative_to(REPO_ROOT)
        else "custom_extension_path",
        "service": "local scripted HTTP stub; no external service calls",
        "settings": {
            "requested_model": "jev-1.13.0",
            "timeout_ms": 5000,
            "max_inflight": 4,
            "cache_max_entries": 20000,
            "cache_max_bytes": 134217728,
            "cache_ttl_ms": 3600000,
        },
        "sql": {
            "row_optimized_batch_cache": "SELECT id, jev_prob(state, criterion) FROM benchmark_rows ORDER BY id",
            "prefilter": "SELECT id, jev_prob(state, criterion) FROM benchmark_rows WHERE id % 2 = 0 ORDER BY id",
        },
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    for result in results:
        modes = result["modes"]
        print(
            f"{result['data']['rows']:>6} rows: "
            f"row={modes['row']['http_attempts']} HTTP, "
            f"optimized={modes['optimized_dedup']['http_attempts']} HTTP, "
            f"cache_hits={modes['cache']['cache_hits_estimated_from_observed_requests']}, "
            f"batch={modes['batch']['http_attempts']} HTTP / "
            f"{modes['batch']['batched_requests']} composite"
        )
    print(f"wrote {args.output}")
    return 0 if all(
        result["correctness"]["optimized_equals_row"]
        and result["correctness"]["batch_equals_row"]
        and result["correctness"]["batch_requests_reduced"]
        and result["correctness"]["batch_profile_matches_observed_requests"]
        and result["correctness"]["prefilter_equals_row_subset"]
        and result["correctness"]["cache_equals_row"]
        for result in results
    ) else 1


if __name__ == "__main__":
    sys.exit(main())
