#!/usr/bin/env python3
"""Offline profile API contract tests; all provider traffic goes to the local stub."""

from __future__ import annotations

import json
import os
import pathlib
import sys
import threading
import time

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "test"))

import duckdb  # noqa: E402

from stub_server import StubServer  # noqa: E402

DEFAULT_EXTENSION = REPO_ROOT / "build/release/duckjeu.duckdb_extension"
SENSITIVE = "private customer state do not include in profile"


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


def profile(connection) -> dict | None:
    raw = connection.execute("SELECT duckjeu_last_profile()").fetchone()[0]
    return json.loads(raw) if raw is not None else None


def main() -> int:
    parser_extension = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else DEFAULT_EXTENSION.resolve()
    if not parser_extension.exists():
        print(f"extension not found: {parser_extension}\nrun `make release` first")
        return 2
    os.environ.setdefault("TYPESAFE_API_KEY", "stub-token-for-profile-tests")
    results = Results()

    with StubServer() as stub:
        con = duckdb.connect(config={"allow_unsigned_extensions": "true"})
        con.execute(f"LOAD '{parser_extension}'")
        con.execute("SET duckjeu_provider='typesafe'")
        con.execute(f"SET duckjeu_api_url='{stub.url('ok')}'")
        con.execute("SET duckjeu_model='jev-latest'")
        con.execute("SET duckjeu_timeout_ms=5000")
        con.execute("SET duckjeu_max_response_bytes=1048576")
        con.execute("SET duckjeu_execution_mode='optimized'")
        con.execute("SET duckjeu_profile_enabled=true")

        absent = duckdb.connect(config={"allow_unsigned_extensions": "true"})
        absent.execute(f"LOAD '{parser_extension}'")
        results.check("未执行判断的独立连接返回 NULL", absent.execute("SELECT duckjeu_last_profile()").fetchone()[0] is None)

        stub.reset()
        con.execute(
            "SELECT jev_prob(?, 'private criterion'), "
            "jev_bool(?, 'private criterion'), "
            "jev_choice(?, 'private question', ['private-label','public-label'])",
            [SENSITIVE, SENSITIVE, SENSITIVE],
        ).fetchone()
        aggregate = profile(con)
        results.check(
            "最近判断查询按连接聚合三函数计数",
            aggregate is not None
            and aggregate["counts"]["input_rows"] == 3
            and aggregate["counts"]["valid_rows"] == 3
            and aggregate["counts"]["external_requests"] == stub.request_count == 2,
            json.dumps(aggregate, ensure_ascii=False),
        )
        results.check(
            "同查询不同函数共享完整身份并保留模型元数据",
            aggregate is not None
            and aggregate["counts"]["unique_judgments"] == 2
            and aggregate["counts"]["deduplicated_rows"] == 1
            and aggregate["reported_models"] == ["jev-latest"],
            json.dumps(aggregate, ensure_ascii=False),
        )
        results.check(
            "profile 不泄漏状态、问题、候选或凭据",
            aggregate is not None
            and all(value not in json.dumps(aggregate) for value in [SENSITIVE, "private criterion", "private-label", "stub-token"]),
        )
        results.check(
            "缺失用量/价格时费用与推理拆分为未知",
            aggregate is not None
            and aggregate["cost_estimate"]["amount"] is None
            and aggregate["timing_ms"]["server_inference_sum"] is None
            and aggregate["timing_ms"]["pure_network_sum"] is None,
        )
        results.check(
            "读取 profile 不覆盖最近判断查询",
            profile(con) == aggregate,
        )
        results.check(
            "另一连接看不到本连接的 profile",
            absent.execute("SELECT duckjeu_last_profile()").fetchone()[0] is None,
        )
        absent.close()

        # Reproduce the row-size regression, then verify an optimized validation error
        # replaces that successful profile instead of leaving it stale.
        validation = duckdb.connect(config={"allow_unsigned_extensions": "true"})
        validation.execute(f"LOAD '{parser_extension}'")
        validation.execute("SET duckjeu_provider='mock'")
        validation.execute("SET duckjeu_profile_enabled=true")
        validation.execute("SET duckjeu_execution_mode='row'")
        validation.execute("SELECT jev_prob(repeat('x', 50000), 'q')").fetchone()
        large_row_profile = profile(validation)
        results.check(
            "row 模式接受 50KB 输入并记录成功 profile",
            large_row_profile is not None
            and large_row_profile["status"] == "completed"
            and large_row_profile["counts"]["input_rows"] == 1
            and large_row_profile["counts"]["valid_rows"] == 1,
            json.dumps(large_row_profile, ensure_ascii=False),
        )

        validation.execute("SET duckjeu_execution_mode='optimized'")
        invalid_error = None
        try:
            validation.execute("SELECT jev_prob('invalid state', '   ')").fetchall()
        except duckdb.Error as error:
            invalid_error = str(error)
        invalid_profile = profile(validation)
        results.check(
            "optimized 输入校验失败刷新 profile 并记录尝试行",
            invalid_error is not None
            and invalid_profile is not None
            and invalid_profile["status"] == "failed"
            and invalid_profile["counts"]["input_rows"] == 1
            and invalid_profile["counts"]["valid_rows"] == 0
            and invalid_profile["counts"]["provider_invocations"] == 0,
            json.dumps(
                {"error": invalid_error, "profile": invalid_profile},
                ensure_ascii=False,
            ),
        )
        validation.close()

        # Detailed timing can be disabled without suppressing basic counters.
        con.execute("SET duckjeu_profile_enabled=false")
        stub.reset()
        con.execute("SELECT jev_prob('ordinary state', 'ordinary criterion')").fetchone()
        no_detail = profile(con)
        results.check(
            "关闭详细计时仍保留基础计数",
            no_detail is not None
            and no_detail["counts"]["external_requests"] == 1
            and no_detail["timing_ms"]["serialization_sum"] is None
            and no_detail["timing_ms"]["external_roundtrip_sum"] is None,
            json.dumps(no_detail, ensure_ascii=False),
        )

        con.execute("SET duckjeu_profile_enabled=true")
        con.execute(f"SET duckjeu_api_url='{stub.url('missing')}'")
        try:
            con.execute("SELECT jev_prob('0.7', 'failure criterion')").fetchall()
        except duckdb.Error:
            pass
        failed = profile(con)
        results.check(
            "失败查询保留已发生请求和未知费用",
            failed is not None
            and failed["status"] == "failed"
            and failed["counts"]["external_requests"] == 1
            and failed["counts"]["failed_requests"] == 1
            and failed["cost_estimate"]["amount"] is None,
            json.dumps(failed, ensure_ascii=False),
        )

        # Use a fresh connection so the earlier explicit TypeSafe model does not leak into
        # the local provider's default-model check.
        local = duckdb.connect(config={"allow_unsigned_extensions": "true"})
        local.execute(f"LOAD '{parser_extension}'")
        local.execute("SET duckjeu_provider='selfhosted'")
        local.execute(f"SET duckjeu_api_url='{stub.url('ok')}'")
        local.execute(f"SET duckjeu_local_jev_api_url='{stub.url('ok')}'")
        local.execute("SET duckjeu_execution_mode='row'")
        local.execute("SET duckjeu_profile_enabled=true")
        stub.reset()
        too_many_choices = [f"choice-{index}" for index in range(256)]
        try:
            local.execute(
                "SELECT jev_choice('synthetic state', 'choose', ?)",
                [too_many_choices],
            ).fetchall()
            cap_rejected = False
        except duckdb.Error:
            cap_rejected = True
        cap_profile = profile(local)
        results.check(
            "local-jev 超过 255 个候选时在网络前失败",
            cap_rejected
            and cap_profile is not None
            and cap_profile["status"] == "failed"
            and cap_profile["requested_model"] == "nli-deberta-large"
            and cap_profile["counts"]["external_requests"] == 0
            and stub.request_count == 0,
            json.dumps(cap_profile, ensure_ascii=False),
        )

        # Explicit self-hosted model names must reach both the request and profile unchanged.
        local.execute("SET duckjeu_model='jev-latest'")
        stub.reset()
        local.execute("SELECT jev_prob('explicit model state', 'criterion')").fetchone()
        explicit_model_profile = profile(local)
        results.check(
            "self-hosted 保留显式模型名并原样发送",
            explicit_model_profile is not None
            and explicit_model_profile["requested_model"] == "jev-latest"
            and stub.request_count == 1
            and stub.requests[0]["body"]["model"] == "jev-latest",
            json.dumps(
                {
                    "profile": explicit_model_profile,
                    "request_model": stub.requests[0]["body"].get("model")
                    if stub.requests
                    else None,
                },
                ensure_ascii=False,
            ),
        )
        local.close()

        # Interruption occurs while a local stub response is pending; no credentials leave the process.
        cancel = duckdb.connect(config={"allow_unsigned_extensions": "true"})
        cancel.execute(f"LOAD '{parser_extension}'")
        for setting in (
            "SET duckjeu_provider='typesafe'",
            f"SET duckjeu_api_url='{stub.url('slow')}'",
            "SET duckjeu_model='jev-latest'",
            "SET duckjeu_timeout_ms=5000",
            "SET duckjeu_execution_mode='row'",
            "SET duckjeu_profile_enabled=true",
        ):
            cancel.execute(setting)
        query_errors: list[Exception] = []

        def run_slow_query() -> None:
            try:
                cancel.execute("SELECT jev_prob('synthetic cancellation state', 'q')").fetchall()
            except Exception as error:  # intentionally interrupted query
                query_errors.append(error)

        worker = threading.Thread(target=run_slow_query)
        worker.start()
        time.sleep(0.15)
        cancel.interrupt()
        worker.join(timeout=10)
        cancel_result = profile(cancel)
        results.check(
            "取消查询保留取消状态",
            not worker.is_alive()
            and bool(query_errors)
            and cancel_result is not None
            and cancel_result["status"] == "cancelled",
            json.dumps(cancel_result, ensure_ascii=False),
        )
        cancel.close()
        con.close()

    total = results.passed + len(results.failed)
    print(f"\n{results.passed}/{total} checks passed")
    if results.failed:
        print("failed checks:")
        for name in results.failed:
            print(f"  - {name}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
