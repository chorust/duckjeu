#!/usr/bin/env python3
"""Explicitly gated DuckJeu acceptance against a running local-jev instance."""

from __future__ import annotations

import json
import os
import pathlib
import platform
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
EVIDENCE = REPO_ROOT / "specs/002-judgment-runtime-evolution/evidence/local-live.json"
EXTENSION = REPO_ROOT / "build/release/duckjeu.duckdb_extension"
API_URL = os.environ.get(
    "DUCKJEU_LOCAL_JEV_API_URL", "http://127.0.0.1:8765/v1/systemone"
)
MODEL = os.environ.get("DUCKJEU_LOCAL_JEV_MODEL", "nli-deberta-large")


def latest_profile(connection) -> dict:
    raw = connection.execute("SELECT duckjeu_last_profile()").fetchone()[0]
    if raw is None:
        raise RuntimeError("profile snapshot is missing")
    return json.loads(raw)


def safe_endpoint(url: str) -> str:
    parsed = urllib.parse.urlsplit(url)
    host = parsed.hostname or "unknown-host"
    if parsed.port:
        host = f"{host}:{parsed.port}"
    return f"{parsed.scheme}://{host}{parsed.path}"


def save(record: dict) -> None:
    EVIDENCE.parent.mkdir(parents=True, exist_ok=True)
    EVIDENCE.write_text(json.dumps(record, ensure_ascii=False, indent=2) + "\n")


def blocked(reason: str, attempts: int = 0) -> int:
    record = {
        "status": "blocked",
        "reason": reason,
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "provider": "selfhosted",
        "endpoint": safe_endpoint(API_URL),
        "requested_model": MODEL,
        "live_gate_enabled": os.environ.get("DUCKJEU_LOCAL_JEV_TEST") == "1",
        "external_http_attempts": attempts,
        "credential_value_recorded": False,
    }
    save(record)
    print(f"BLOCKED: {reason}; {attempts} loopback HTTP attempts")
    print(f"evidence written to {EVIDENCE}")
    return 0


def main() -> int:
    if os.environ.get("DUCKJEU_LOCAL_JEV_TEST") != "1":
        return blocked("DUCKJEU_LOCAL_JEV_TEST != 1 (explicit local-service opt-in required)")
    if not EXTENSION.exists():
        return blocked("release extension not found; run make release first")

    health_url = API_URL.removesuffix("/v1/systemone") + "/healthz"
    request = urllib.request.Request(health_url, method="GET")
    try:
        with urllib.request.urlopen(request, timeout=3) as response:
            if response.status != 200:
                return blocked(f"local-jev health returned HTTP {response.status}", 1)
            health = json.loads(response.read(65_536).decode("utf-8"))
    except (urllib.error.URLError, TimeoutError, OSError, ValueError):
        return blocked("local-jev health request failed", 1)

    record = {
        "status": "running",
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "environment": {
            "os": f"{platform.system()} {platform.machine()}",
            "python": platform.python_version(),
        },
        "provider": "selfhosted",
        "endpoint": safe_endpoint(API_URL),
        "requested_model": MODEL,
        "local_jev_credential_configured": bool(os.environ.get("DUCKJEU_LOCAL_JEV_API_KEY")),
        "server_version": health.get("version"),
        "server_default_model": health.get("default_model"),
        "credential_value_recorded": False,
        "health_http_attempts": 1,
        "duckjeu_external_http_attempts": 0,
        "checks": {},
        "failures": [],
    }
    try:
        import duckdb

        con = duckdb.connect(config={"allow_unsigned_extensions": "true"})
        con.execute(f"LOAD '{EXTENSION.resolve()}'")
        con.execute("SET duckjeu_provider='selfhosted'")
        con.execute(f"SET duckjeu_api_url='{API_URL}'")
        con.execute(f"SET duckjeu_local_jev_api_url='{API_URL}'")
        con.execute(f"SET duckjeu_model='{MODEL}'")
        con.execute("SET duckjeu_timeout_ms=120000")
        con.execute("SET duckjeu_max_response_bytes=1048576")
        con.execute("SET duckjeu_profile_enabled=true")

        cases = {
            "jev_prob": (
                "SELECT jev_prob(state, 'The customer asks for a refund') "
                "FROM (VALUES ('Synthetic support ticket: please refund my order.'), "
                "('Synthetic support ticket: the package arrived.'), "
                "('Synthetic support ticket: I was charged twice.')) t(state)"
            ),
            "jev_bool": (
                "SELECT jev_bool(state, 'The invoice is overdue and unpaid') "
                "FROM (VALUES ('Synthetic invoice: payment is overdue.'), "
                "('Synthetic invoice: payment was received.'), "
                "('Synthetic invoice: the due date is next week.')) t(state)"
            ),
            "jev_choice": (
                "SELECT jev_choice(state, 'Which topic does the ticket concern?', "
                "['billing','shipping','account']) "
                "FROM (VALUES ('Synthetic ticket: I was charged twice.'), "
                "('Synthetic ticket: my package is late.'), "
                "('Synthetic ticket: I cannot sign in.')) t(state)"
            ),
        }
        for function, sql in cases.items():
            started = time.monotonic()
            rows = con.execute(sql).fetchall()
            elapsed = time.monotonic() - started
            query_profile = latest_profile(con)
            if len(rows) != 3 or any(row[0] is None for row in rows):
                raise RuntimeError(f"{function}: expected three non-NULL results")
            if query_profile["counts"]["external_requests"] != 3:
                raise RuntimeError(f"{function}: expected three observed service attempts")
            if query_profile["status"] != "completed":
                raise RuntimeError(f"{function}: query profile did not complete")
            if function == "jev_prob" and any(not 0.0 <= row[0] <= 1.0 for row in rows):
                raise RuntimeError("jev_prob returned a value outside [0,1]")
            if function == "jev_choice" and any(
                row[0] not in {"billing", "shipping", "account"} for row in rows
            ):
                raise RuntimeError("jev_choice returned a label outside the candidate set")
            record["checks"][function] = {
                "rows": len(rows),
                "results": [row[0] for row in rows],
                "elapsed_seconds": round(elapsed, 3),
                "external_http_attempts": query_profile["counts"]["external_requests"],
                "model_reported": query_profile["reported_models"],
            }
            record["duckjeu_external_http_attempts"] += query_profile["counts"]["external_requests"]

        # The pinned adapter must reject the service's 255-choice limit before sending a request.
        con.execute("SET duckjeu_execution_mode='row'")
        choices = "[" + ",".join(f"'choice_{index}'" for index in range(256)) + "]"
        choice_limit_rejected = False
        try:
            con.execute(
                "SELECT jev_choice('Synthetic choice-limit state', 'choose', "
                f"{choices})"
            ).fetchall()
        except Exception:
            choice_limit_rejected = True
        limit_profile = latest_profile(con)
        limit_ok = (
            choice_limit_rejected
            and limit_profile["status"] == "failed"
            and limit_profile["counts"]["external_requests"] == 0
        )
        record["checks"]["choice_limit_preflight"] = {
            "passed": limit_ok,
            "external_http_attempts": limit_profile["counts"]["external_requests"],
        }
        if not limit_ok:
            raise RuntimeError("256-choice request was not rejected before HTTP")

        # Even with DuckJeu's cache enabled, unknown local weight revisions cannot be reused.
        con.execute("SET duckjeu_execution_mode='optimized'")
        con.execute("SET duckjeu_cache_enabled=true")
        cache_sql = (
            "SELECT jev_prob('Synthetic local cache isolation state', "
            "'is this a cache isolation probe?')"
        )
        con.execute(cache_sql).fetchone()
        first_profile = latest_profile(con)
        con.execute(cache_sql).fetchone()
        second_profile = latest_profile(con)
        cache_ok = all(
            item["counts"]["external_requests"] == 1
            and item["counts"]["cache_hit_judgments"] == 0
            for item in (first_profile, second_profile)
        )
        record["checks"]["unknown_weight_cache_isolation"] = {
            "passed": cache_ok,
            "first_query_http_attempts": first_profile["counts"]["external_requests"],
            "second_query_http_attempts": second_profile["counts"]["external_requests"],
            "cache_hits": first_profile["counts"]["cache_hit_judgments"]
            + second_profile["counts"]["cache_hit_judgments"],
        }
        if not cache_ok:
            raise RuntimeError("local-jev result was reused across queries with unknown weights")
        record["duckjeu_external_http_attempts"] += (
            first_profile["counts"]["external_requests"]
            + second_profile["counts"]["external_requests"]
        )

        con.close()
        record["status"] = "passed"
    except Exception as error:  # evidence gets a fixed message, never request data or credentials
        record["status"] = "failed"
        record["failures"].append(type(error).__name__)
    save(record)
    print(json.dumps(record, ensure_ascii=False, indent=2))
    print(f"evidence written to {EVIDENCE}")
    return 0 if record["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
