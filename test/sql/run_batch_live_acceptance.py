#!/usr/bin/env python3
"""Explicitly gated TypeSafe composite-rows probe for cross-state batching."""

from __future__ import annotations

import json
import math
import os
import pathlib
import platform
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
EVIDENCE = REPO_ROOT / "specs/002-judgment-runtime-evolution/evidence/batch-live.json"
API_URL = os.environ.get("DUCKJEU_API_URL", "https://api.typesafe.ai/v1/systemone")
MODEL = os.environ.get("DUCKJEU_MODEL", "jev-latest")
MAX_RESPONSE_BYTES = 1_048_576

PROBES = {
    "noul": {
        "states": [
            "Synthetic payment record: the invoice was paid in full yesterday.",
            "Synthetic payment record: no payment was made and the invoice is overdue.",
        ],
        "unrelated": "Synthetic shipping record: the package arrived on Tuesday.",
        "criterion": "the invoice has not been paid",
    },
    "choice": {
        "states": [
            "Synthetic animal record: the animal is explicitly a bird.",
            "Synthetic animal record: the animal is explicitly a shark.",
        ],
        "unrelated": "Synthetic plant record: the plant is an oak tree.",
        "question": "Which animal is explicitly named in this row?",
        "choices": ["bird", "shark", "whale"],
        "expected": ["bird", "shark"],
    },
}


def safe_endpoint(url: str) -> str:
    parsed = urllib.parse.urlsplit(url)
    host = parsed.hostname or "unknown-host"
    if parsed.port:
        host = f"{host}:{parsed.port}"
    return f"{parsed.scheme}://{host}{parsed.path}"


def save_evidence(record: dict) -> None:
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
        "external_http_attempts": 0,
        "live_gate_enabled": os.environ.get("DUCKJEU_BATCH_LIVE_TEST") == "1",
    }
    save_evidence(record)
    print(f"BLOCKED: {reason}; 0 external HTTP attempts")
    print(f"evidence written to {EVIDENCE}")
    return 0


def question(kind: str, index: int, question_key: str, probe: dict) -> dict:
    if kind == "noul":
        return {
            "type": "noul",
            "instructions": (
                f"Evaluate whether state.condition holds for state.rows[{index}]. "
                "Return the probability that it holds for this row only."
            ),
        }
    criteria = {label: None for label in probe["choices"]}
    return {
        "type": "choice",
        "instructions": (
            f"Caller question: {probe['question']}\n"
            f"Select the single candidate label that best answers this question for "
            f"state.rows[{index}] only."
        ),
        "criteria": criteria,
    }


def body_for(
    kind: str,
    rows: list[str],
    question_rows: list[tuple[str, int]],
    probe: dict,
) -> dict:
    state: dict = {"rows": rows}
    if kind == "noul":
        state["condition"] = probe["criterion"]
    return {
        "model": MODEL,
        "state": state,
        "questions": {
            key: question(kind, index, key, probe) for key, index in question_rows
        },
    }


def parse_answers(kind: str, expected_keys: list[str], response: dict, probe: dict) -> dict:
    answers = response.get("answers")
    if not isinstance(answers, dict) or set(answers) != set(expected_keys):
        raise ValueError("response answer keys do not exactly match the probe keys")
    values: dict[str, float | str] = {}
    for key in expected_keys:
        answer = answers[key]
        if not isinstance(answer, dict):
            raise ValueError("answer is not an object")
        if kind == "noul":
            value = answer.get("noul")
            if answer.get("type") != "noul" or isinstance(value, bool) or not isinstance(value, (int, float)):
                raise ValueError("answer is not a numeric noul value")
            value = float(value)
            if not math.isfinite(value) or not 0.0 <= value <= 1.0:
                raise ValueError("noul answer is outside [0,1]")
            values[key] = value
        else:
            value = answer.get("choice")
            if answer.get("type") != "choice" or value not in probe["choices"]:
                raise ValueError("choice answer is not an exact candidate")
            values[key] = value
    return values


def main() -> int:
    if os.environ.get("DUCKJEU_BATCH_LIVE_TEST") != "1":
        return blocked("DUCKJEU_BATCH_LIVE_TEST != 1 (explicit live opt-in required)")

    token = next(
        (
            os.environ[name]
            for name in ("DUCKJEU_API_KEY", "TYPESAFE_API_KEY")
            if os.environ.get(name, "").strip()
        ),
        None,
    )
    if token is None:
        return blocked("DUCKJEU_API_KEY and TYPESAFE_API_KEY are both unset")

    record = {
        "status": "pending",
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "environment": {"os": f"{platform.system()} {platform.machine()}", "python": platform.python_version()},
        "provider": "typesafe",
        "endpoint": safe_endpoint(API_URL),
        "requested_model": MODEL,
        "live_gate_enabled": True,
        "credential_value_recorded": False,
        "external_http_attempts": 0,
        "probes": {},
        "failures": [],
    }
    failures: list[str] = record["failures"]

    def post(label: str, request_body: dict) -> tuple[dict, float]:
        record["external_http_attempts"] += 1
        request = urllib.request.Request(
            API_URL,
            data=json.dumps(request_body).encode("utf-8"),
            headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
            method="POST",
        )
        started = time.monotonic()
        try:
            with urllib.request.urlopen(request, timeout=45) as response:
                status = response.status
                raw = response.read(MAX_RESPONSE_BYTES + 1)
        except urllib.error.HTTPError as error:
            raise RuntimeError(f"{label}: HTTP {error.code}") from None
        except (urllib.error.URLError, TimeoutError, OSError):
            raise RuntimeError(f"{label}: request failed or timed out") from None
        if status < 200 or status >= 300:
            raise RuntimeError(f"{label}: HTTP {status}")
        if len(raw) > MAX_RESPONSE_BYTES:
            raise RuntimeError(f"{label}: response exceeded {MAX_RESPONSE_BYTES} bytes")
        try:
            payload = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            raise RuntimeError(f"{label}: response was not valid JSON") from None
        return payload, time.monotonic() - started

    for kind, probe in PROBES.items():
        item = {"single": {}, "composite": {}, "with_unrelated_row": {}}
        record["probes"][kind] = item
        expected_values: list[float | str] = []
        for index, state in enumerate(probe["states"]):
            body = body_for(kind, [state], [("r0", 0)], probe)
            try:
                response, elapsed = post(f"{kind} single row {index}", body)
                parsed = parse_answers(kind, ["r0"], response, probe)
                expected_values.append(parsed["r0"])
                item["single"][str(index)] = {
                    "result": parsed["r0"],
                    "model_reported": response.get("model"),
                    "elapsed_seconds": round(elapsed, 3),
                }
            except RuntimeError as error:
                failures.append(str(error))
                break
            except ValueError as error:
                failures.append(f"{kind} single row {index}: invalid response ({error})")
                break
        if len(expected_values) != 2:
            continue

        rows = probe["states"]
        # Insert questions in reverse order to ensure result association uses keys, not order.
        pair_questions = [("r1", 1), ("r0", 0)]
        try:
            response, elapsed = post(
                f"{kind} two-state composite", body_for(kind, rows, pair_questions, probe)
            )
            parsed = parse_answers(kind, ["r0", "r1"], response, probe)
            item["composite"] = {
                "results_by_key": parsed,
                "response_key_order": list(response["answers"].keys()),
                "model_reported": response.get("model"),
                "elapsed_seconds": round(elapsed, 3),
            }
        except RuntimeError as error:
            failures.append(str(error))
            continue
        except ValueError as error:
            failures.append(f"{kind} two-state composite: invalid response ({error})")
            continue

        rows_with_unrelated = [probe["unrelated"], *rows]
        inserted_questions = [("r2", 2), ("r1", 1)]
        try:
            response, elapsed = post(
                f"{kind} unrelated-row composite",
                body_for(kind, rows_with_unrelated, inserted_questions, probe),
            )
            parsed = parse_answers(kind, ["r1", "r2"], response, probe)
            item["with_unrelated_row"] = {
                "results_by_key": parsed,
                "response_key_order": list(response["answers"].keys()),
                "model_reported": response.get("model"),
                "elapsed_seconds": round(elapsed, 3),
            }
        except RuntimeError as error:
            failures.append(str(error))
            continue
        except ValueError as error:
            failures.append(f"{kind} unrelated-row composite: invalid response ({error})")
            continue

        if kind == "noul":
            pair = item["composite"]["results_by_key"]
            inserted = item["with_unrelated_row"]["results_by_key"]
            item["row0_vs_row1_delta"] = round(float(pair["r1"]) - float(pair["r0"]), 6)
            item["row_insertion_deltas"] = {
                "row0": round(float(inserted["r1"]) - float(pair["r0"]), 6),
                "row1": round(float(inserted["r2"]) - float(pair["r1"]), 6),
            }
            if float(pair["r1"]) <= float(pair["r0"]):
                failures.append("noul composite did not rank the unpaid row above the paid row")
            if float(inserted["r2"]) <= float(inserted["r1"]):
                failures.append("noul mapping changed after inserting an unrelated first row")
        else:
            pair = item["composite"]["results_by_key"]
            inserted = item["with_unrelated_row"]["results_by_key"]
            item["expected_labels"] = probe["expected"]
            if [pair["r0"], pair["r1"]] != probe["expected"]:
                failures.append("choice composite labels do not match the two referenced rows")
            if [inserted["r1"], inserted["r2"]] != probe["expected"]:
                failures.append("choice labels changed association after unrelated-row insertion")

    record["status"] = "failed" if failures else "passed"
    save_evidence(record)
    print(json.dumps(record, ensure_ascii=False, indent=2))
    print(f"\nevidence written to {EVIDENCE}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
