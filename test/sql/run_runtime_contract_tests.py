#!/usr/bin/env python3
"""离线运行时契约验收；真实请求只发往本机 stub。"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import sys
import tempfile
import time

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "test"))

import duckdb  # noqa: E402

from stub_server import StubServer  # noqa: E402

DEFAULT_EXTENSION = REPO_ROOT / "build" / "release" / "duckjeu.duckdb_extension"


class Results:
    def __init__(self) -> None:
        self.passed = 0
        self.failed: list[str] = []

    def check(self, name: str, condition: bool, detail: str = "") -> None:
        if condition:
            self.passed += 1
            print(f"  ok   {name}")
        else:
            self.failed.append(name)
            print(f"  FAIL {name}{(' — ' + detail) if detail else ''}")

    def report(self) -> int:
        total = self.passed + len(self.failed)
        print(f"\n{self.passed}/{total} checks passed")
        if self.failed:
            print("failed checks:")
            for name in self.failed:
                print(f"  - {name}")
            return 1
        return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--extension", type=pathlib.Path, default=DEFAULT_EXTENSION)
    args = parser.parse_args()
    extension = args.extension.resolve()
    if not extension.exists():
        print(f"extension not found: {extension}\nrun `make release` first")
        return 2

    os.environ.setdefault("TYPESAFE_API_KEY", "stub-token-for-runtime-tests")
    results = Results()
    with StubServer() as stub:
        con = duckdb.connect(config={"allow_unsigned_extensions": "true"})
        con.execute(f"LOAD '{extension}'")
        con.execute("SET duckjeu_provider='typesafe'")
        con.execute(f"SET duckjeu_api_url='{stub.url('echo-value')}'")
        con.execute("SET duckjeu_model='jev-latest'")
        con.execute("SET duckjeu_timeout_ms=5000")
        con.execute("SET duckjeu_max_response_bytes=1048576")
        con.execute(
            "CREATE TEMP TABLE identity_rows AS "
            "SELECT i::INTEGER AS id, "
            "CASE WHEN i % 2 = 0 THEN '0.2' ELSE '0.8' END AS state "
            "FROM range(2500) t(i)"
        )

        query = (
            "SELECT jev_prob(state, 'is the value high?'), "
            "jev_bool(state, 'is the value high?'), "
            "jev_choice(state, 'which value?', ['0.2','0.8']) "
            "FROM identity_rows ORDER BY id"
        )
        con.execute("SET duckjeu_execution_mode='row'")
        stub.reset()
        row_results = con.execute(query).fetchall()
        row_requests = stub.request_count

        con.execute("SET duckjeu_execution_mode='optimized'")
        stub.reset()
        optimized_results = con.execute(query).fetchall()
        optimized_requests = stub.request_count
        expected = [
            (0.2 if i % 2 == 0 else 0.8, i % 2 == 1, "0.2" if i % 2 == 0 else "0.8")
            for i in range(2500)
        ]
        results.check(
            "jev_prob/jev_bool/jev_choice 优化结果与逐行结果逐行一致",
            optimized_results == row_results == expected,
            "result mismatch",
        )
        results.check(
            "重复状态跨 DataChunk 且跨函数去重到四个真实 HTTP 请求",
            row_requests == 7500 and optimized_requests == 4,
            f"row={row_requests}, optimized={optimized_requests}",
        )

        # A single optimized request must obey the byte cap even when batching is off.
        con.execute("SET duckjeu_batch_enabled=false")
        con.execute("SET duckjeu_max_request_bytes=1")
        stub.reset()
        byte_limit_error = None
        try:
            con.execute("SELECT jev_prob('0.2', 'single request byte limit')").fetchall()
        except duckdb.Error as exc:
            byte_limit_error = str(exc)
        results.check(
            "非合批优化请求遵守字节上限且外发前失败",
            byte_limit_error is not None
            and "duckjeu_max_request_bytes" in byte_limit_error
            and stub.request_count == 0,
            f"error={byte_limit_error}, requests={stub.request_count}",
        )
        con.execute("SET duckjeu_max_request_bytes=262144")

        # 空输入及任一顶层 NULL 不生成判断身份，也不触达 provider。
        stub.reset()
        empty = con.execute(
            "SELECT jev_prob(state, 'q') FROM identity_rows WHERE false"
        ).fetchall()
        results.check("空输入无结果且零请求", empty == [] and stub.request_count == 0)

        nulls = con.execute(
            "SELECT jev_prob(NULL, 'q'), jev_prob('s', NULL), "
            "jev_choice('s', 'q', NULL)"
        ).fetchone()
        results.check("顶层 NULL 保持 NULL", nulls == (None, None, None), str(nulls))
        results.check("顶层 NULL 不发请求", stub.request_count == 0, str(stub.request_count))

        error = None
        try:
            con.execute("SELECT jev_prob('s', '   ')").fetchall()
        except duckdb.Error as exc:
            error = str(exc)
        results.check("非法输入报错", error is not None, str(error))
        results.check("非法输入在 provider 请求前失败", stub.request_count == 0, str(stub.request_count))

        # An unverified model must fail closed before HTTP; the live-verified model batches end-to-end.
        con.execute("SET duckjeu_batch_enabled=true")
        con.execute("SET duckjeu_execution_mode='optimized'")
        con.execute("SET duckjeu_profile_enabled=true")
        con.execute(f"SET duckjeu_api_url='{stub.url('echo-batch')}'")
        con.execute("SET duckjeu_model='unverified-model'")
        stub.reset()
        batch_error = None
        try:
            con.execute(
                "SELECT jev_prob(state, 'is the value high?') "
                "FROM (VALUES ('0.2'), ('0.8')) AS t(state)"
            ).fetchall()
        except duckdb.Error as exc:
            batch_error = str(exc)
        results.check(
            "未验证模型的 TypeSafe 跨状态合批显式失败",
            batch_error is not None and "batch" in batch_error.lower(),
            str(batch_error),
        )
        results.check(
            "未验证模型的合批在 HTTP 请求前失败",
            stub.request_count == 0,
            str(stub.request_count),
        )

        con.execute("SET duckjeu_model='jev-1.13.0'")
        stub.reset()
        batch_results = con.execute(
            "WITH sample(id, payment, animal) AS ("
            "VALUES (0, '0.2', 'bird'), (1, '0.8', 'shark')) "
            "SELECT id, jev_prob(payment, 'is the value high?'), "
            "jev_bool(payment, 'is the value high?'), "
            "jev_choice(animal, 'which animal?', ['bird','shark']) "
            "FROM sample ORDER BY id"
        ).fetchall()
        batch_profile = json.loads(
            con.execute("SELECT duckjeu_last_profile()").fetchone()[0]
        )
        results.check(
            "TypeSafe 优化 SQL 跨状态批次保持三函数结果与行映射",
            batch_results
            == [(0, 0.2, False, "bird"), (1, 0.8, True, "shark")],
            str(batch_results),
        )
        results.check(
            "TypeSafe SQL 对四个唯一判断只发两个 batch HTTP 请求",
            stub.request_count == 2
            and batch_profile["counts"]["unique_judgments"] == 4
            and batch_profile["counts"]["external_requests"] == 2
            and batch_profile["counts"]["batched_requests"] == 2
            and batch_profile["counts"]["batch_size_histogram"] == {"2": 2},
            f"requests={stub.request_count}, profile={batch_profile['counts']}",
        )
        batch_bodies = [request["body"] for request in stub.requests]
        results.check(
            "TypeSafe HTTP composite body carries both indexed states and questions",
            len(batch_bodies) == 2
            and all(len(body["state"]["rows"]) == 2 for body in batch_bodies)
            and all(len(body["questions"]) == 2 for body in batch_bodies)
            and any(body["state"].get("condition") == "is the value high?" for body in batch_bodies)
            and any(
                all(
                    "state.rows[0]" in question["instructions"]
                    or "state.rows[1]" in question["instructions"]
                    for question in body["questions"].values()
                )
                for body in batch_bodies
            ),
            str(batch_bodies),
        )

        con.execute("SET duckjeu_profile_enabled=false")
        con.execute("SET duckjeu_batch_enabled=false")
        con.execute("SET duckjeu_model='jev-latest'")
        con.execute(f"SET duckjeu_api_url='{stub.url('echo-value')}'")
        stub.reset()

        # The offline mock verifies executor batching and cross-function row association.
        con.execute("SET duckjeu_provider='mock'")
        con.execute("SET duckjeu_batch_enabled=false")
        row_batch_query = (
            "SELECT jev_prob('low', 'same criterion'), "
            "jev_bool('high', 'same criterion'), "
            "jev_choice('animal', 'which animal?', ['bird','shark'])"
        )
        row_mock = con.execute(row_batch_query).fetchone()
        con.execute("SET duckjeu_batch_enabled=true")
        batch_mock = con.execute(row_batch_query).fetchone()
        results.check(
            "mock 多状态合批与逐行模式结果一致",
            batch_mock == row_mock,
            f"row={row_mock}, batch={batch_mock}",
        )
        results.check("mock 合批场景没有外部 HTTP 请求", stub.request_count == 0)

        # Force parallel row-mode provider work; verify the connection permit caps it.
        con.execute("SET duckjeu_provider='typesafe'")
        con.execute(f"SET duckjeu_api_url='{stub.url('slow-short')}'")
        con.execute("SET duckjeu_model='jev-1.13.0'")
        con.execute("SET duckjeu_execution_mode='row'")
        con.execute("SET duckjeu_batch_enabled=false")
        con.execute("SET duckjeu_cache_enabled=false")
        con.execute("SET duckjeu_max_inflight=16")
        con.execute("SET threads=8")
        with tempfile.TemporaryDirectory(prefix="duckjeu-runtime-") as tempdir:
            parquet = pathlib.Path(tempdir) / "parallel_rows.parquet"
            con.execute(
                "COPY (SELECT i::VARCHAR AS state FROM range(4096) AS t(i)) "
                f"TO '{parquet}' (FORMAT PARQUET, ROW_GROUP_SIZE 2048)"
            )
            inflight_query = (
                "SELECT jev_prob(state, 'row inflight contract') "
                f"FROM read_parquet('{parquet}')"
            )
            stub.reset()
            parallel_results = con.execute(inflight_query).fetchall()
            parallel_requests = stub.request_count
            parallel_peak = stub.peak_concurrent_requests

            con.execute("SET duckjeu_max_inflight=1")
            stub.reset()
            limited_results = con.execute(inflight_query).fetchall()
            limited_requests = stub.request_count
            limited_peak = stub.peak_concurrent_requests
        results.check(
            "row 并发验收夹具确实产生并发请求",
            len(parallel_results) == 4096
            and parallel_requests == 4096
            and parallel_peak > 1,
            f"rows={len(parallel_results)}, requests={parallel_requests}, peak={parallel_peak}",
        )
        results.check(
            "row 模式 max_inflight=1 将 provider 并发限制为 1",
            len(limited_results) == 4096
            and limited_requests == 4096
            and limited_peak == 1,
            f"rows={len(limited_results)}, requests={limited_requests}, peak={limited_peak}",
        )

        con.close()
        run_cache_contracts(results, extension, stub)
    return results.report()


def run_cache_contracts(results: Results, extension: pathlib.Path, stub: StubServer) -> None:
    print("\nUS3 — 连接级缓存、凭据隔离与清空")
    con = duckdb.connect(config={"allow_unsigned_extensions": "true"})
    con.execute(f"LOAD '{extension}'")
    con.execute("SET duckjeu_provider='typesafe'")
    con.execute(f"SET duckjeu_api_url='{stub.url('echo-value')}'")
    con.execute("SET duckjeu_model='jev-1.13.0'")
    con.execute("SET duckjeu_timeout_ms=5000")
    con.execute("SET duckjeu_max_response_bytes=1048576")
    con.execute("SET duckjeu_execution_mode='optimized'")
    con.execute("SET duckjeu_cache_enabled=true")
    con.execute("SET duckjeu_cache_max_entries=16")
    con.execute("SET duckjeu_cache_max_bytes=1048576")
    con.execute("SET duckjeu_cache_ttl_ms=60000")

    query = "SELECT jev_prob('0.8', 'cache criterion')"
    stub.reset()
    first = con.execute(query).fetchone()[0]
    second = con.execute(query).fetchone()[0]
    results.check(
        "固定模型的重复查询跨查询命中连接缓存",
        first == second and stub.request_count == 1,
        f"requests={stub.request_count}",
    )
    con.execute("SET duckjeu_model='jev-1.13.1'")
    con.execute(query).fetchone()
    model_switch_requests = stub.request_count
    con.execute(f"SET duckjeu_api_url='{stub.url('echo-model')}'")
    con.execute(query).fetchone()
    results.check(
        "模型和服务端点切换均不误命中",
        model_switch_requests == 2 and stub.request_count == 3,
        f"after_model={model_switch_requests}, after_endpoint={stub.request_count}",
    )
    con.execute(f"SET duckjeu_api_url='{stub.url('echo-value')}'")
    con.execute("SET duckjeu_model='jev-1.13.0'")
    con.execute("SELECT duckjeu_cache_clear()").fetchone()
    con.execute(query).fetchone()
    cleared = con.execute("SELECT duckjeu_cache_clear()").fetchone()[0]
    results.check("cache_clear 返回当前连接已清条目数", cleared == 1)
    con.execute(query).fetchone()
    results.check("清空后同一判断重新请求", stub.request_count == 5, str(stub.request_count))

    con.execute("SELECT duckjeu_cache_clear()").fetchone()
    con.execute("SET duckjeu_cache_ttl_ms=20")
    stub.reset()
    con.execute(query).fetchone()
    time.sleep(0.05)
    con.execute(query).fetchone()
    results.check("TTL 到期后重新请求", stub.request_count == 2, str(stub.request_count))
    con.execute("SET duckjeu_cache_ttl_ms=60000")

    # Mutable aliases and unpinned model names are never reused between queries.
    con.execute("SET duckjeu_model='jev-latest'")
    stub.reset()
    con.execute(query).fetchone()
    con.execute(query).fetchone()
    results.check("jev-latest 不跨查询缓存", stub.request_count == 2, str(stub.request_count))

    # Invalid responses are neither returned from nor inserted into the cache.
    con.execute(f"SET duckjeu_api_url='{stub.url('missing')}'")
    stub.reset()
    failures = 0
    for _ in range(2):
        try:
            con.execute(query).fetchall()
        except duckdb.Error:
            failures += 1
    results.check(
        "失败响应不缓存且每次显式失败",
        failures == 2 and stub.request_count == 2,
        f"failures={failures}, requests={stub.request_count}",
    )

    # Re-establish a pinned model, then prove the cache belongs to each connection.
    con.execute(f"SET duckjeu_api_url='{stub.url('echo-value')}'")
    con.execute("SET duckjeu_model='jev-1.13.0'")
    other = duckdb.connect(config={"allow_unsigned_extensions": "true"})
    other.execute(f"LOAD '{extension}'")
    for setting in (
        "SET duckjeu_provider='typesafe'",
        f"SET duckjeu_api_url='{stub.url('echo-value')}'",
        "SET duckjeu_model='jev-1.13.0'",
        "SET duckjeu_execution_mode='optimized'",
        "SET duckjeu_cache_enabled=true",
    ):
        other.execute(setting)
    con.execute("SELECT duckjeu_cache_clear()").fetchone()
    stub.reset()
    con.execute(query).fetchone()
    other.execute(query).fetchone()
    con.execute(query).fetchone()
    other.execute(query).fetchone()
    results.check(
        "相同判断在两个连接分别请求、各自在连接内命中",
        stub.request_count == 2,
        str(stub.request_count),
    )
    cleared = con.execute("SELECT duckjeu_cache_clear()").fetchone()[0]
    other.execute(query).fetchone()
    results.check(
        "清空一个连接不影响另一个连接的缓存",
        cleared == 1 and stub.request_count == 2,
        f"cleared={cleared}, requests={stub.request_count}",
    )
    con.execute(query).fetchone()
    results.check("被清空的连接重新请求", stub.request_count == 3, str(stub.request_count))

    # Credential changes rotate the opaque scope and remove old entries without exposing tokens.
    old_key = os.environ.get("DUCKJEU_API_KEY")
    try:
        os.environ["DUCKJEU_API_KEY"] = "runtime-cache-key-a"
        stub.reset()
        con.execute(query).fetchone()
        con.execute(query).fetchone()
        os.environ["DUCKJEU_API_KEY"] = "runtime-cache-key-b"
        con.execute(query).fetchone()
        results.check("凭据轮换清除旧缓存且不误命中", stub.request_count == 2, str(stub.request_count))
    finally:
        if old_key is None:
            os.environ.pop("DUCKJEU_API_KEY", None)
        else:
            os.environ["DUCKJEU_API_KEY"] = old_key

    con.execute("SET duckjeu_cache_enabled=false")
    stub.reset()
    con.execute(query).fetchone()
    con.execute(query).fetchone()
    results.check("关闭缓存后重复查询均请求 provider", stub.request_count == 2, str(stub.request_count))

    con.execute("SET duckjeu_execution_mode='row'")
    con.execute("SET duckjeu_cache_enabled=true")
    conflict = None
    try:
        con.execute(query).fetchall()
    except duckdb.Error as exc:
        conflict = str(exc)
    results.check("row 模式开启缓存时显式报配置冲突", conflict is not None and "cache" in conflict.lower())

    other.close()
    fresh = duckdb.connect(config={"allow_unsigned_extensions": "true"})
    fresh.execute(f"LOAD '{extension}'")
    for setting in (
        "SET duckjeu_provider='typesafe'",
        f"SET duckjeu_api_url='{stub.url('echo-value')}'",
        "SET duckjeu_model='jev-1.13.0'",
        "SET duckjeu_execution_mode='optimized'",
        "SET duckjeu_cache_enabled=true",
    ):
        fresh.execute(setting)
    stub.reset()
    fresh.execute(query).fetchone()
    results.check("关闭连接后新连接没有旧缓存", stub.request_count == 1, str(stub.request_count))
    fresh.close()

    # Lowering active limits must reconcile existing entries before lookup and clear.
    print("\nUS3 — 活跃缓存策略收紧")
    con.execute("SET duckjeu_execution_mode='optimized'")
    con.execute("SET duckjeu_cache_enabled=true")
    con.execute("SET duckjeu_cache_max_entries=2")
    con.execute("SET duckjeu_cache_max_bytes=1048576")
    con.execute("SET duckjeu_cache_ttl_ms=60000")
    entry_queries = (
        "SELECT jev_prob('0.2', 'entry policy reduction')",
        "SELECT jev_prob('0.8', 'entry policy reduction')",
    )
    con.execute("SELECT duckjeu_cache_clear()")
    stub.reset()
    for reduced_query in entry_queries:
        con.execute(reduced_query).fetchone()
    con.execute("SET duckjeu_cache_max_entries=1")
    for reduced_query in entry_queries:
        con.execute(reduced_query).fetchone()
    entry_requests_after_reduction = stub.request_count
    remaining_entries = con.execute("SELECT duckjeu_cache_clear()").fetchone()[0]
    results.check(
        "降低 entry 上限先淘汰旧项再查缓存",
        entry_requests_after_reduction == 4 and remaining_entries == 1,
        f"requests={entry_requests_after_reduction}, clear={remaining_entries}",
    )

    con.execute("SET duckjeu_cache_max_entries=8")
    con.execute("SET duckjeu_cache_max_bytes=1048576")
    con.execute("SELECT duckjeu_cache_clear()")
    stub.reset()
    for reduced_query in entry_queries:
        con.execute(reduced_query).fetchone()
    con.execute("SET duckjeu_cache_max_bytes=1")
    for reduced_query in entry_queries:
        con.execute(reduced_query).fetchone()
    byte_requests_after_reduction = stub.request_count
    remaining_entries = con.execute("SELECT duckjeu_cache_clear()").fetchone()[0]
    results.check(
        "降低 byte 上限清除超限项且不再缓存",
        byte_requests_after_reduction == 4 and remaining_entries == 0,
        f"requests={byte_requests_after_reduction}, clear={remaining_entries}",
    )

    con.close()


if __name__ == "__main__":
    sys.exit(main())
