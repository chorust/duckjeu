#!/usr/bin/env python3
"""Run the fixed 1K/10K/100K comparison against a real local-jev server."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import platform
import sys
import time
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from typing import Any

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "examples"))

import duckdb  # noqa: E402

from make_sample_data import RUNTIME_SEED, generate_runtime_rows  # noqa: E402

DEFAULT_EXTENSION = REPO_ROOT / "build/release/duckjeu.duckdb_extension"
DEFAULT_OUTPUT = (
    REPO_ROOT
    / "specs/002-judgment-runtime-evolution/evidence/benchmark-local-live.json"
)
DEFAULT_API_URL = "http://127.0.0.1:8765/v1/systemone"


def sql_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def safe_endpoint(url: str) -> str:
    parsed = urllib.parse.urlsplit(url)
    host = parsed.hostname or "unknown-host"
    if parsed.port:
        host = f"{host}:{parsed.port}"
    return f"{parsed.scheme}://{host}{parsed.path}"


def profile_for(con: duckdb.DuckDBPyConnection) -> dict[str, Any]:
    raw = con.execute("SELECT duckjeu_last_profile()").fetchone()[0]
    if raw is None:
        raise RuntimeError("profile_missing")
    return json.loads(raw)


def run_query(
    con: duckdb.DuckDBPyConnection, query: str
) -> tuple[list[tuple], float, dict[str, Any]]:
    started = time.perf_counter()
    rows = con.execute(query).fetchall()
    elapsed_ms = (time.perf_counter() - started) * 1000
    return rows, elapsed_ms, profile_for(con)


def result_rows_match(left: list[tuple], right: list[tuple]) -> bool:
    if len(left) != len(right):
        return False
    for left_row, right_row in zip(left, right):
        if left_row[0] != right_row[0]:
            return False
        left_value, right_value = left_row[1], right_row[1]
        if isinstance(left_value, (int, float)) and isinstance(right_value, (int, float)):
            if abs(float(left_value) - float(right_value)) > 1e-6:
                return False
        elif left_value != right_value:
            return False
    return True


def summarize_query(profile: dict[str, Any], elapsed_ms: float) -> dict[str, Any]:
    return {
        "status": profile.get("status"),
        "external_http_attempts": profile.get("counts", {}).get("external_requests"),
        "counts": profile.get("counts", {}),
        "reported_models": profile.get("reported_models", []),
        "usage": profile.get("usage", {}),
        "elapsed_ms": round(elapsed_ms, 3),
        "query_wall_ms": profile.get("timing_ms", {}).get("query_wall"),
    }


def set_common_settings(con: duckdb.DuckDBPyConnection, api_url: str, model: str) -> None:
    for setting in (
        "SET duckjeu_provider='selfhosted'",
        f"SET duckjeu_api_url={sql_quote(api_url)}",
        f"SET duckjeu_local_jev_api_url={sql_quote(api_url)}",
        f"SET duckjeu_model={sql_quote(model)}",
        "SET duckjeu_timeout_ms=120000",
        "SET duckjeu_max_response_bytes=1048576",
        "SET duckjeu_max_inflight=4",
        "SET duckjeu_profile_enabled=true",
        "SET duckjeu_execution_mode='row'",
        "SET duckjeu_batch_enabled=false",
        "SET duckjeu_cache_enabled=false",
    ):
        con.execute(setting)


def run_size(
    extension: pathlib.Path,
    api_url: str,
    model: str,
    size: int,
    seed: int,
) -> dict[str, Any]:
    data, summary = generate_runtime_rows(size, seed)
    con = duckdb.connect(config={"allow_unsigned_extensions": "true"})
    try:
        con.execute(f"LOAD '{extension}'")
        set_common_settings(con, api_url, model)
        con.execute(
            "CREATE TEMP TABLE benchmark_rows(id BIGINT, state VARCHAR, criterion VARCHAR)"
        )
        con.executemany("INSERT INTO benchmark_rows VALUES (?, ?, ?)", data)
        query = "SELECT id, jev_prob(state, criterion) FROM benchmark_rows ORDER BY id"
        modes: dict[str, Any] = {}

        print(f"  {size}: row mode ({summary['valid_rows']} live requests expected)", flush=True)
        row_results, elapsed, profile = run_query(con, query)
        modes["row"] = summarize_query(profile, elapsed)
        modes["row"]["output_rows"] = len(row_results)
        modes["row"]["nonnull_output_rows"] = sum(value is not None for _, value in row_results)
        modes["row"]["correct"] = (
            len(row_results) == size
            and modes["row"]["nonnull_output_rows"] == summary["valid_rows"]
            and modes["row"]["external_http_attempts"] == summary["valid_rows"]
            and profile.get("status") == "completed"
        )

        print(f"  {size}: optimized dedup mode", flush=True)
        con.execute("SET duckjeu_execution_mode='optimized'")
        optimized_results, elapsed, profile = run_query(con, query)
        modes["optimized_dedup"] = summarize_query(profile, elapsed)
        modes["optimized_dedup"]["correct"] = result_rows_match(
            row_results, optimized_results
        )

        print(f"  {size}: SQL prefilter mode", flush=True)
        con.execute("SET duckjeu_execution_mode='row'")
        prefilter_query = (
            "SELECT id, jev_prob(state, criterion) FROM benchmark_rows "
            "WHERE id % 2 = 0 ORDER BY id"
        )
        prefiltered_results, elapsed, profile = run_query(con, prefilter_query)
        expected_prefilter = [row for row in row_results if row[0] % 2 == 0]
        modes["prefilter"] = summarize_query(profile, elapsed)
        modes["prefilter"].update(
            {
                "predicate": "id % 2 = 0",
                "valid_output_rows": len(prefiltered_results),
                "correct": result_rows_match(expected_prefilter, prefiltered_results),
            }
        )

        print(f"  {size}: batch capability preflight", flush=True)
        con.execute("SET duckjeu_execution_mode='optimized'")
        con.execute("SET duckjeu_batch_enabled=true")
        started = time.perf_counter()
        batch_error: str | None = None
        try:
            con.execute(query).fetchall()
        except Exception as error:  # expected: this provider has no verified multi-state batch
            batch_error = type(error).__name__
        elapsed = (time.perf_counter() - started) * 1000
        batch_profile = profile_for(con)
        batch_summary = summarize_query(batch_profile, elapsed)
        batch_summary.update(
            {
                "capability": "unsupported",
                "reason": "local-jev cross-state batch capability is not verified",
                "error_type": batch_error,
                "no_http_before_preflight": (
                    batch_error is not None
                    and batch_profile.get("status") == "failed"
                    and batch_summary["external_http_attempts"] == 0
                ),
            }
        )
        modes["batch"] = batch_summary

        print(f"  {size}: cross-query cache mode (two identical queries)", flush=True)
        con.execute("SET duckjeu_batch_enabled=false")
        con.execute("SET duckjeu_cache_enabled=true")
        con.execute("SET duckjeu_cache_max_entries=20000")
        con.execute("SET duckjeu_cache_max_bytes=134217728")
        con.execute("SET duckjeu_cache_ttl_ms=3600000")
        con.execute("SELECT duckjeu_cache_clear()")
        cache_first, first_ms, first_profile = run_query(con, query)
        cache_second, second_ms, second_profile = run_query(con, query)
        first = summarize_query(first_profile, first_ms)
        second = summarize_query(second_profile, second_ms)
        modes["cache"] = {
            "status": "measured",
            "correct": result_rows_match(row_results, cache_first)
            and result_rows_match(row_results, cache_second),
            "first_query": first,
            "second_query": second,
            "cross_query_cache_hits": first["counts"].get("cache_hit_judgments", 0)
            + second["counts"].get("cache_hit_judgments", 0),
            "unknown_weight_revision_disables_cache": (
                first["external_http_attempts"] == summary["unique_judgments"]
                and second["external_http_attempts"] == summary["unique_judgments"]
                and first["counts"].get("cache_hit_judgments", 0) == 0
                and second["counts"].get("cache_hit_judgments", 0) == 0
            ),
        }

        correctness = {
            "row_completed_with_one_request_per_valid_row": modes["row"]["correct"],
            "optimized_equals_row": modes["optimized_dedup"]["correct"],
            "prefilter_equals_row_subset": modes["prefilter"]["correct"],
            "batch_rejected_before_http": modes["batch"]["no_http_before_preflight"],
            "cache_results_equal_row": modes["cache"]["correct"],
            "cache_disabled_for_unknown_weight_revision": modes["cache"][
                "unknown_weight_revision_disables_cache"
            ],
        }
        return {
            "data": summary,
            "provider": "selfhosted",
            "endpoint_kind": "real_local_jev_service",
            "requested_model": model,
            "modes": modes,
            "correctness": correctness,
            "cost_estimate": None,
            "cost_unknown_reason": "local service has no billing or pricing metadata",
        }
    finally:
        con.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--extension", type=pathlib.Path, default=DEFAULT_EXTENSION)
    parser.add_argument("--api-url", default=os.environ.get("DUCKJEU_LOCAL_JEV_API_URL", DEFAULT_API_URL))
    parser.add_argument("--model", default=os.environ.get("DUCKJEU_LOCAL_JEV_MODEL", "nli-deberta-large"))
    parser.add_argument("--rows", default="1000,10000,100000", help="comma-separated sizes")
    parser.add_argument("--seed", type=int, default=RUNTIME_SEED)
    parser.add_argument("--output", type=pathlib.Path, default=DEFAULT_OUTPUT)
    parser.add_argument(
        "--resume",
        action="store_true",
        help="continue an existing report and skip sizes already recorded",
    )
    args = parser.parse_args()

    if os.environ.get("DUCKJEU_LOCAL_JEV_BENCH") != "1":
        print("BLOCKED: set DUCKJEU_LOCAL_JEV_BENCH=1 to run live model requests")
        return 2
    extension = args.extension.resolve()
    if not extension.exists():
        print(f"extension not found: {extension}; run `make release` first")
        return 2
    try:
        sizes = [int(value.strip()) for value in args.rows.split(",") if value.strip()]
        if not sizes or any(size <= 0 for size in sizes):
            raise ValueError
    except ValueError:
        print("--rows must contain positive comma-separated integers")
        return 2

    health_url = args.api_url.removesuffix("/v1/systemone") + "/healthz"
    try:
        with urllib.request.urlopen(health_url, timeout=5) as response:
            health = json.loads(response.read(65536).decode("utf-8"))
            if response.status != 200:
                raise RuntimeError("health_check_failed")
    except Exception as error:
        print(f"BLOCKED: local-jev health check failed ({type(error).__name__})")
        return 2

    extension_label = (
        str(extension.relative_to(REPO_ROOT))
        if extension.is_relative_to(REPO_ROOT)
        else "custom_extension_path"
    )
    report: dict[str, Any]
    if args.resume:
        if not args.output.exists():
            print(f"cannot resume: report not found: {args.output}")
            return 2
        try:
            report = json.loads(args.output.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            print(f"cannot resume: invalid report: {args.output}")
            return 2
        expected = {
            "seed": args.seed,
            "provider": "selfhosted",
            "endpoint": safe_endpoint(args.api_url),
            "requested_model": args.model,
            "extension": extension_label,
        }
        if any(report.get(key) != value for key, value in expected.items()):
            print("cannot resume: seed, provider, endpoint, model, or extension differs")
            return 2
        report["status"] = "running"
        report.setdefault("failures", [])
        report.setdefault("results", [])
        report.setdefault("resume_timestamps_utc", []).append(
            datetime.now(timezone.utc).isoformat()
        )
    else:
        report = {
            "schema_version": 1,
            "status": "running",
            "timestamp_utc": datetime.now(timezone.utc).isoformat(),
            "seed": args.seed,
            "duckdb_python_version": duckdb.__version__,
            "python_version": platform.python_version(),
            "platform": platform.platform(),
            "extension": extension_label,
            "provider": "selfhosted",
            "endpoint": safe_endpoint(args.api_url),
            "service_version": health.get("version"),
            "service_default_model": health.get("default_model"),
            "requested_model": args.model,
            "reported_weight_revision": "unknown; service reports model alias only",
            "cross_query_cache_policy": "disabled because the loaded weight revision is unverified",
            "live_gate_enabled": True,
            "credential_configured": bool(os.environ.get("DUCKJEU_LOCAL_JEV_API_KEY")),
            "settings": {"timeout_ms": 120000, "max_inflight": 4},
            "results": [],
            "failures": [],
        }

    def save() -> None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")

    save()
    try:
        recorded_sizes = {
            result.get("data", {}).get("rows") for result in report["results"]
        }
        for size in sizes:
            if size in recorded_sizes:
                print(f"skipping {size}: already present in resumed evidence", flush=True)
                continue
            print(f"measuring {size} rows against real local-jev ({health.get('version')})", flush=True)
            result = run_size(extension, args.api_url, args.model, size, args.seed)
            report["results"].append(result)
            if not all(result["correctness"].values()):
                report["failures"].append(f"correctness_gate_failed_{size}")
            save()
            modes = result["modes"]
            print(
                f"  {size}: row={modes['row']['external_http_attempts']} HTTP, "
                f"optimized={modes['optimized_dedup']['external_http_attempts']} HTTP, "
                f"prefilter={modes['prefilter']['external_http_attempts']} HTTP, "
                f"batch=unsupported (0 HTTP), "
                f"cache queries={modes['cache']['first_query']['external_http_attempts']}+"
                f"{modes['cache']['second_query']['external_http_attempts']} HTTP",
                flush=True,
            )
    except Exception as error:
        report["failures"].append(f"{type(error).__name__}")
        report["status"] = "failed"
        save()
        print(f"FAILED: {type(error).__name__}; partial evidence saved to {args.output}")
        return 1

    report["status"] = "passed" if not report["failures"] else "failed"
    save()
    print(f"{report['status'].upper()}: evidence written to {args.output}")
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
