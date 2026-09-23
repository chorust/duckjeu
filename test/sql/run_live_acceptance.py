#!/usr/bin/env python3
"""真实服务验收（SC-004）。

仅在同时设置 API 凭据（`DUCKJEU_API_KEY` 优先于 `TYPESAFE_API_KEY`）与
`DUCKJEU_LIVE_TEST=1` 时运行；否则输出
`BLOCKED` 并以退出码 0 结束（记录阻塞，不标记为完成）。

验收内容：三个函数各 ≥3 行**非敏感**输入，校验结果类型与行映射 100% 合法，
并保留 provider、实际 model、行数、请求数与耗时的可复核记录。

用法：
    export DUCKJEU_API_KEY=...  # 或 TYPESAFE_API_KEY
    DUCKJEU_LIVE_TEST=1 python3 test/sql/run_live_acceptance.py
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import platform
import subprocess
import sys
import time
import urllib.error
import urllib.request

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
DEFAULT_EXTENSION = REPO_ROOT / "build" / "release" / "duckjeu.duckdb_extension"
EVIDENCE_DIR = REPO_ROOT / "build" / "acceptance"

API_URL = os.environ.get("DUCKJEU_API_URL", "https://api.typesafe.ai/v1/systemone")
MODEL = os.environ.get("DUCKJEU_MODEL", "jev-latest")

# 非敏感合成输入。
TICKETS = [
    "I was charged twice for the same order and need a refund.",
    "How do I change my shipping address before the order ships?",
    "The package arrived damaged; please replace it.",
]
MARKET = [
    (101.5, 0.82, 12.0),
    (98.25, 0.41, 7.5),
    (110.0, 0.95, 21.0),
]
REGIME_QUESTION = "which market regime best describes this state?"
REGIMES = ["normal", "scarcity", "oversupply"]
REFUND_CRITERION = "the customer requests a refund"


def blocked(reason: str) -> int:
    print(f"BLOCKED: {reason}")
    print("真实服务验收未完成；请在具备凭据的环境中重跑。")
    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    (EVIDENCE_DIR / "live-status.json").write_text(
        json.dumps(
            {
                "status": "blocked",
                "reason": reason,
                "date": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
                "api_url": API_URL,
                "model_requested": MODEL,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n"
    )
    return 0


def probe_reported_model(token: str) -> str | None:
    """直接调用一次服务，记录响应中回报的实际 model。"""
    body = json.dumps(
        {
            "model": MODEL,
            "state": {"rows": ["hello"], "condition": "this is a greeting"},
            "questions": {"r0": {"type": "noul", "instructions": "probability the condition holds"}},
        }
    ).encode("utf-8")
    request = urllib.request.Request(
        API_URL,
        data=body,
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = json.loads(response.read().decode("utf-8"))
        return payload.get("model")
    except (urllib.error.URLError, json.JSONDecodeError, TimeoutError):
        return None


def duckdb_version() -> str:
    try:
        return subprocess.run(
            ["duckdb", "--version"], capture_output=True, text=True, check=True
        ).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "unknown"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--extension", type=pathlib.Path, default=DEFAULT_EXTENSION)
    args = parser.parse_args()
    extension = args.extension.resolve()

    token = None
    for key in ("DUCKJEU_API_KEY", "TYPESAFE_API_KEY"):
        candidate = os.environ.get(key)
        if candidate and candidate.strip():
            token = candidate
            break
    if not token:
        return blocked("DUCKJEU_API_KEY 与 TYPESAFE_API_KEY 均未设置")
    if os.environ.get("DUCKJEU_LIVE_TEST") != "1":
        return blocked("DUCKJEU_LIVE_TEST != 1（真实调用必须显式开启）")
    if not extension.exists():
        return blocked(f"扩展不存在：{extension}（先运行 make release）")

    import duckdb  # noqa: PLC0415

    con = duckdb.connect(config={"allow_unsigned_extensions": "true"})
    con.execute(f"LOAD '{extension}'")
    con.execute("SET duckjeu_provider='typesafe'")
    con.execute(f"SET duckjeu_api_url='{API_URL}'")
    con.execute(f"SET duckjeu_model='{MODEL}'")

    evidence: dict = {
        "status": "pending",
        "date": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "environment": {
            "os": f"{platform.system()} {platform.machine()}",
            "duckdb_cli": duckdb_version(),
            "duckdb_python": duckdb.__version__,
            "extension": str(extension),
        },
        "provider": "typesafe",
        "api_url": API_URL,
        "model_requested": MODEL,
        "model_reported": probe_reported_model(token),
        "functions": {},
        "notes": [
            "v0.1 逐行同步调用且无缓存，请求数按行数推导，未经服务端计费数据核对。",
            "输入为非敏感合成数据。",
        ],
    }

    failures: list[str] = []

    def record(function: str, rows: list, produced: list, elapsed: float) -> None:
        evidence["functions"][function] = {
            "rows": len(rows),
            "requests_expected": len(rows),
            "elapsed_seconds": round(elapsed, 3),
            "seconds_per_row": round(elapsed / max(len(rows), 1), 4),
            "results": produced,
        }

    con.execute("CREATE TABLE tickets(id INTEGER, message VARCHAR)")
    con.executemany("INSERT INTO tickets VALUES (?, ?)", list(enumerate(TICKETS)))
    con.execute("CREATE TABLE market(price DOUBLE, load DOUBLE, wind DOUBLE)")
    con.executemany("INSERT INTO market VALUES (?, ?, ?)", MARKET)

    started = time.monotonic()
    probabilities = con.execute(
        f"SELECT id, jev_prob(message, '{REFUND_CRITERION}') FROM tickets ORDER BY id"
    ).fetchall()
    record("jev_prob", TICKETS, probabilities, time.monotonic() - started)
    for _, value in probabilities:
        if value is None or not (0.0 <= float(value) <= 1.0):
            failures.append(f"jev_prob 返回越界或 NULL：{value}")

    started = time.monotonic()
    booleans = con.execute(
        f"SELECT id, jev_bool(message, '{REFUND_CRITERION}') FROM tickets ORDER BY id"
    ).fetchall()
    record("jev_bool", TICKETS, booleans, time.monotonic() - started)
    for (_, probability), (_, flag) in zip(probabilities, booleans):
        if probability is None or flag is None:
            continue
        if bool(flag) != (float(probability) >= 0.5):
            # 两次独立调用可能得到不同模型输出；记录为观察项而非失败。
            evidence.setdefault("observations", []).append(
                f"jev_bool 与独立 jev_prob 结果不一致（允许，非确定性）：{probability} vs {flag}"
            )

    choices = "'" + "','".join(REGIMES) + "'"
    started = time.monotonic()
    regimes = con.execute(
        "SELECT price, jev_choice(struct_pack(price := price, load := load, wind := wind),"
        f" '{REGIME_QUESTION}', [{choices}]) FROM market ORDER BY price"
    ).fetchall()
    record("jev_choice", MARKET, regimes, time.monotonic() - started)
    for _, label in regimes:
        if label not in REGIMES:
            failures.append(f"jev_choice 返回候选集外标签：{label}")

    evidence["status"] = "failed" if failures else "passed"
    evidence["failures"] = failures
    EVIDENCE_DIR.mkdir(parents=True, exist_ok=True)
    path = EVIDENCE_DIR / f"live-{time.strftime('%Y%m%dT%H%M%S')}.json"
    path.write_text(json.dumps(evidence, ensure_ascii=False, indent=2) + "\n")

    print(json.dumps(evidence, ensure_ascii=False, indent=2))
    print(f"\nevidence written to {path}")
    if failures:
        print("LIVE ACCEPTANCE FAILED")
        return 1
    print("LIVE ACCEPTANCE PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
