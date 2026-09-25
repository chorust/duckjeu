#!/usr/bin/env python3
"""Explicitly gated, low-volume TypeSafe batching test through the DuckJeu extension."""

from __future__ import annotations

import json
import os
import pathlib
import platform
import sys
import time
import urllib.parse

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
EXTENSION = REPO_ROOT / "build/release/duckjeu.duckdb_extension"
EVIDENCE = (
    REPO_ROOT
    / "specs/002-judgment-runtime-evolution/evidence/batch-extension-live.json"
)
API_URL = os.environ.get("DUCKJEU_API_URL", "https://api.typesafe.ai/v1/systemone")
MODEL = os.environ.get("DUCKJEU_MODEL", "jev-latest")


def sql_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def safe_endpoint(url: str) -> str:
    parsed = urllib.parse.urlsplit(url)
    host = parsed.hostname or "unknown-host"
    if parsed.port:
        host = f"{host}:{parsed.port}"
    return f"{parsed.scheme}://{host}{parsed.path}"


def save(record: dict) -> None:
    EVIDENCE.parent.mkdir(parents=True, exist_ok=True)
    EVIDENCE.write_text(json.dumps(record, ensure_ascii=False, indent=2) + "\n")


def blocked(reason: str) -> int:
    record = {
        "status": "blocked",
        "reason": reason,
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "provider": "typesafe",
        "endpoint": safe_endpoint(API_URL),
        "requested_model": MODEL,
        "live_gate_enabled": os.environ.get("DUCKJEU_BATCH_EXTENSION_LIVE_TEST") == "1",
        "credential_value_recorded": False,
        "duckjeu_external_http_attempts": 0,
    }
    save(record)
    print(f"BLOCKED: {reason}; 0 DuckJeu HTTP attempts")
    print(f"evidence written to {EVIDENCE}")
    return 0


def main() -> int:
    if os.environ.get("DUCKJEU_BATCH_EXTENSION_LIVE_TEST") != "1":
        return blocked("DUCKJEU_BATCH_EXTENSION_LIVE_TEST != 1 (explicit live opt-in required)")
    if not any(
        os.environ.get(name, "").strip()
        for name in ("DUCKJEU_API_KEY", "TYPESAFE_API_KEY")
    ):
        return blocked("DUCKJEU_API_KEY and TYPESAFE_API_KEY are both unset")
    if not EXTENSION.exists():
        return blocked("release extension not found; run make release first")

    record = {
        "status": "running",
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "environment": {
            "os": f"{platform.system()} {platform.machine()}",
            "python": platform.python_version(),
        },
        "provider": "typesafe",
        "endpoint": safe_endpoint(API_URL),
        "requested_model": MODEL,
        "live_gate_enabled": True,
        "credential_value_recorded": False,
        "expected_external_http_attempts": 2,
        "duckjeu_external_http_attempts": 0,
        "checks": {},
        "failures": [],
    }

    connection = None
    try:
        import duckdb

        connection = duckdb.connect(config={"allow_unsigned_extensions": "true"})
        connection.execute(f"LOAD '{EXTENSION.resolve()}'")
        for setting in (
            "SET duckjeu_provider='typesafe'",
            f"SET duckjeu_api_url={sql_quote(API_URL)}",
            f"SET duckjeu_model={sql_quote(MODEL)}",
            "SET duckjeu_execution_mode='optimized'",
            "SET duckjeu_batch_enabled=true",
            "SET duckjeu_cache_enabled=false",
            "SET duckjeu_profile_enabled=true",
            "SET duckjeu_timeout_ms=45000",
        ):
            connection.execute(setting)

        connection.execute(
            "CREATE TEMP TABLE batch_live_rows AS SELECT * FROM (VALUES "
            "(0, 'Synthetic payment record: the invoice was paid in full yesterday.'), "
            "(1, 'Synthetic payment record: no payment was made and the invoice is overdue.')"
            ") AS t(id, payment_state)"
        )
        query = """
            WITH payments AS (
                SELECT id,
                       jev_prob(payment_state, 'the invoice has not been paid') AS probability,
                       jev_bool(payment_state, 'the invoice has not been paid') AS unpaid,
                       NULL::VARCHAR AS animal
                FROM batch_live_rows
            ), animals AS (
                SELECT id + 2 AS id,
                       NULL::DOUBLE AS probability,
                       NULL::BOOLEAN AS unpaid,
                       jev_choice(
                           CASE id
                             WHEN 0 THEN 'Synthetic animal record: the animal is explicitly a bird.'
                             ELSE 'Synthetic animal record: the animal is explicitly a shark.'
                           END,
                           'Which animal is explicitly named in this row?',
                           ['bird', 'shark', 'whale']
                       ) AS animal
                FROM batch_live_rows
            )
            SELECT * FROM payments
            UNION ALL
            SELECT * FROM animals
            ORDER BY id
        """
        rows = connection.execute(query).fetchall()
        raw_profile = connection.execute("SELECT duckjeu_last_profile()").fetchone()[0]
        profile = json.loads(raw_profile) if raw_profile else None
        if profile is None:
            raise RuntimeError("profile missing")

        counts = profile.get("counts", {})
        record["duckjeu_external_http_attempts"] = counts.get("external_requests", 0)
        probabilities = [rows[0][1], rows[1][1]]
        booleans = [rows[0][2], rows[1][2]]
        animals = [rows[2][3], rows[3][3]]
        checks = {
            "four_unique_judgments": counts.get("unique_judgments") == 4,
            "exactly_two_external_requests": counts.get("external_requests") == 2,
            "both_requests_are_composite": counts.get("batched_requests") == 2
            and counts.get("batch_size_histogram") == {"2": 2},
            "reported_model_is_pinned": profile.get("reported_models") == ["jev-1.13.0"],
            "probability_rows_are_associated": all(
                isinstance(value, (int, float)) and 0.0 <= value <= 1.0
                for value in probabilities
            )
            and probabilities[1] > probabilities[0],
            "boolean_rows_are_associated": booleans == [False, True],
            "choice_rows_are_associated": animals == ["bird", "shark"],
            "query_completed": profile.get("status") == "completed",
        }
        record["checks"] = checks
        record["results"] = {
            "probabilities": probabilities,
            "booleans": booleans,
            "choices": animals,
        }
        record["profile"] = {
            "status": profile.get("status"),
            "reported_models": profile.get("reported_models"),
            "counts": counts,
        }
        if not all(checks.values()):
            record["failures"].append("one or more DuckJeu live batch assertions failed")
        record["status"] = "passed" if not record["failures"] else "failed"
    except Exception as error:
        record["status"] = "failed"
        record["failures"].append(type(error).__name__)
        if connection is not None:
            try:
                raw_profile = connection.execute("SELECT duckjeu_last_profile()").fetchone()[0]
                profile = json.loads(raw_profile) if raw_profile else None
                if profile is not None:
                    counts = profile.get("counts", {})
                    record["duckjeu_external_http_attempts"] = counts.get("external_requests", 0)
                    record["profile"] = {
                        "status": profile.get("status"),
                        "reported_models": profile.get("reported_models"),
                        "counts": counts,
                    }
            except Exception:
                pass
    finally:
        if connection is not None:
            connection.close()

    save(record)
    print(json.dumps(record, ensure_ascii=False, indent=2))
    print(f"\nevidence written to {EVIDENCE}")
    return 0 if record["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
