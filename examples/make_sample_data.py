#!/usr/bin/env python3
"""生成非敏感样例 Parquet：tickets.parquet 与 market.parquet。

用法：
    python3 examples/make_sample_data.py [输出目录]

样例数据为合成内容，不含任何真实客户信息。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import random
import sys

import duckdb

TICKETS = [
    (1, "I was charged twice for the same order and need a refund.", "billing", 42.0),
    (2, "How do I change my shipping address before the order ships?", "account", 18.5),
    (3, "The package arrived damaged; please replace it.", "shipping", 75.25),
    (4, "Can you cancel my subscription effective next month?", "billing", 12.0),
    (5, "The app crashes whenever I open the settings page.", "bug", 0.0),
    (6, "Thanks, everything works now — no action needed.", "other", 0.0),
]

MARKET = [
    (101.5, 0.82, 12.0),
    (98.25, 0.41, 7.5),
    (110.0, 0.95, 21.0),
    (95.75, 0.30, 3.25),
]

RUNTIME_CRITERIA = (
    "the value is at least 0.25",
    "the value is at least 0.50",
    "the value is at least 0.75",
    "the value is positive",
)
RUNTIME_SEED = 20260923


def generate_runtime_rows(size: int, seed: int = RUNTIME_SEED) -> tuple[list[tuple[int, str | None, str | None]], dict]:
    """Create reproducible, non-sensitive rows with 90% planned state repetition."""
    if size <= 0:
        raise ValueError("size must be positive")
    rng = random.Random(seed + size)
    state_pool_size = max(1, size // 10)
    rows: list[tuple[int, str | None, str | None]] = []
    cross_chunk_duplicate_rows: list[int] = []
    for row_id in range(size):
        state_index = rng.randrange(state_pool_size)
        state = f"{state_index / max(1, state_pool_size - 1):.6f}"
        criterion = RUNTIME_CRITERIA[state_index % len(RUNTIME_CRITERIA)]
        if rng.random() < 0.03:
            state = None
        if rng.random() < 0.02:
            criterion = None
        rows.append((row_id, state, criterion))

    # Force one identical non-NULL judgment across DuckDB's default 2,048-row chunk edge.
    if size > 2048:
        boundary = (size // 2) // 2048 * 2048
        if 0 < boundary < size:
            pair = ("0.500000", RUNTIME_CRITERIA[0])
            rows[boundary - 1] = (boundary - 1, *pair)
            rows[boundary] = (boundary, *pair)
            cross_chunk_duplicate_rows = [boundary - 1, boundary]

    valid_identities = {(state, criterion) for _, state, criterion in rows if state is not None and criterion is not None}
    digest_input = json.dumps(rows, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    summary = {
        "seed": seed,
        "rows": size,
        "state_pool_size": state_pool_size,
        "planned_unique_state_fraction": 0.1,
        "null_state_rows": sum(state is None for _, state, _ in rows),
        "null_criterion_rows": sum(criterion is None for _, _, criterion in rows),
        "valid_rows": sum(state is not None and criterion is not None for _, state, criterion in rows),
        "unique_judgments": len(valid_identities),
        "deduplicated_rows": sum(state is not None and criterion is not None for _, state, criterion in rows) - len(valid_identities),
        "cross_chunk_duplicate_row_ids": cross_chunk_duplicate_rows,
        "sha256": hashlib.sha256(digest_input).hexdigest(),
    }
    return rows, summary


def write_runtime_datasets(out_dir: pathlib.Path, seed: int) -> list[dict]:
    runtime_dir = out_dir / "runtime-benchmarks"
    runtime_dir.mkdir(parents=True, exist_ok=True)
    con = duckdb.connect()
    summaries = []
    try:
        con.execute("CREATE TABLE runtime_rows(id BIGINT, state VARCHAR, criterion VARCHAR)")
        for size in (1_000, 10_000, 100_000):
            rows, summary = generate_runtime_rows(size, seed)
            con.execute("DELETE FROM runtime_rows")
            con.executemany("INSERT INTO runtime_rows VALUES (?, ?, ?)", rows)
            target = runtime_dir / f"runtime-{size}.parquet"
            con.execute(f"COPY runtime_rows TO '{target}' (FORMAT PARQUET)")
            summary["file"] = str(target)
            summaries.append(summary)
    finally:
        con.close()
    summary_path = runtime_dir / "summary.json"
    summary_path.write_text(json.dumps(summaries, indent=2) + "\n", encoding="utf-8")
    return summaries


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out_dir", nargs="?", type=pathlib.Path, default=pathlib.Path(__file__).parent / "data")
    parser.add_argument("--runtime-benchmarks", action="store_true", help="also write fixed-seed 1K/10K/100K runtime Parquet data")
    parser.add_argument("--seed", type=int, default=RUNTIME_SEED)
    args = parser.parse_args()
    out_dir = args.out_dir
    out_dir.mkdir(parents=True, exist_ok=True)
    con = duckdb.connect()

    con.execute("CREATE TABLE tickets(id INTEGER, message VARCHAR, topic VARCHAR, amount DOUBLE)")
    con.executemany("INSERT INTO tickets VALUES (?, ?, ?, ?)", TICKETS)
    con.execute(f"COPY tickets TO '{out_dir / 'tickets.parquet'}' (FORMAT PARQUET)")

    con.execute("CREATE TABLE market(price DOUBLE, load DOUBLE, wind DOUBLE)")
    con.executemany("INSERT INTO market VALUES (?, ?, ?)", MARKET)
    con.execute(f"COPY market TO '{out_dir / 'market.parquet'}' (FORMAT PARQUET)")

    print(f"wrote {out_dir / 'tickets.parquet'}")
    print(f"wrote {out_dir / 'market.parquet'}")
    if args.runtime_benchmarks:
        for summary in write_runtime_datasets(out_dir, args.seed):
            print(
                f"wrote runtime {summary['rows']} rows; valid={summary['valid_rows']}; "
                f"unique={summary['unique_judgments']}; sha256={summary['sha256']}"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())
