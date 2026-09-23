#!/usr/bin/env python3
"""生成非敏感样例 Parquet：tickets.parquet 与 market.parquet。

用法：
    python3 examples/make_sample_data.py [输出目录]

样例数据为合成内容，不含任何真实客户信息。
"""

from __future__ import annotations

import pathlib
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


def main() -> int:
    out_dir = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path(__file__).parent / "data"
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
    return 0


if __name__ == "__main__":
    sys.exit(main())
