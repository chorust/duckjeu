#!/usr/bin/env python3
"""DuckJeu v0.1 SQL 契约与错误路径测试。

覆盖 `specs/001-judgment-foundation/spec.md` 的场景 1–4：
  * 概率、布尔与类别的类型化结果与行对齐（含跨 DataChunk 边界）
  * 顶层/内部 NULL、空数据集零请求、非法输入报错
  * 连接级配置隔离、显式开启真实调用、凭据不入错误信息
  * HTTP 错误、超时、响应大小上限、无效响应全部显式失败

用法：
    python3 test/sql/run_contract_tests.py [--extension build/release/duckjeu.duckdb_extension]
"""

from __future__ import annotations

import argparse
import os
import pathlib
import sys

REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "test"))

import duckdb  # noqa: E402

from stub_server import FAKE_SECRET, StubServer  # noqa: E402

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


def connect(extension: pathlib.Path) -> duckdb.DuckDBPyConnection:
    con = duckdb.connect(config={"allow_unsigned_extensions": "true"})
    con.execute(f"LOAD '{extension}'")
    return con


def configure(con: duckdb.DuckDBPyConnection, stub: StubServer, scenario: str, **settings) -> None:
    con.execute("SET duckjeu_provider='typesafe'")
    con.execute(f"SET duckjeu_api_url='{stub.url(scenario)}'")
    con.execute("SET duckjeu_model='jev-latest'")
    con.execute("SET duckjeu_timeout_ms=5000")
    con.execute("SET duckjeu_max_response_bytes=1048576")
    for key, value in settings.items():
        con.execute(f"SET {key}={value}")


def expect_error(con: duckdb.DuckDBPyConnection, sql: str) -> str | None:
    try:
        con.execute(sql).fetchall()
    except duckdb.Error as err:  # noqa: PERF203
        return str(err)
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--extension", type=pathlib.Path, default=DEFAULT_EXTENSION)
    args = parser.parse_args()
    extension = args.extension.resolve()
    if not extension.exists():
        print(f"extension not found: {extension}\nrun `make release` first")
        return 2

    results = Results()
    os.environ.setdefault("TYPESAFE_API_KEY", "stub-token-for-tests")

    with StubServer() as stub:
        run_us1(results, extension, stub)
        run_us2(results, extension, stub)
        run_us3(results, extension, stub)

    return results.report()


def run_us1(results: Results, extension: pathlib.Path, stub: StubServer) -> None:
    print("\nUS1 — 逐行类型化概率结果")
    con = connect(extension)

    # 文本状态与 STRUCT 状态都能取到 [0,1] 概率（离线 mock）
    con.execute("SET duckjeu_provider='mock'")
    probability = con.execute("SELECT jev_prob('hello', 'is it a greeting?')").fetchone()[0]
    results.check("mock 文本状态返回 [0,1] 概率", 0.0 <= probability <= 1.0, str(probability))
    struct_probability = con.execute(
        "SELECT jev_prob(struct_pack(price:=1.5, load:=3), 'is demand high?')"
    ).fetchone()[0]
    results.check(
        "mock STRUCT 状态返回 [0,1] 概率", 0.0 <= struct_probability <= 1.0, str(struct_probability)
    )

    # 顶层 NULL 短路：结果为 NULL 且不调用 provider
    stub.reset()
    configure(con, stub, "ok")
    row = con.execute(
        "SELECT jev_prob(NULL, 'q'), jev_prob('s', NULL), jev_prob(NULL, NULL)"
    ).fetchone()
    results.check("顶层 NULL 结果为 NULL", row == (None, None, None), str(row))
    results.check("顶层 NULL 不发请求", len(stub.requests) == 0, str(len(stub.requests)))

    # 空数据集零请求
    stub.reset()
    count = con.execute(
        "SELECT count(*) FROM (SELECT jev_prob(state, 'q') AS p FROM (SELECT 'a' AS state) t WHERE 1=0)"
    ).fetchone()[0]
    results.check("空数据集零请求", count == 0 and len(stub.requests) == 0)

    # 概率边界 0 与 1 合法
    configure(con, stub, "boundary-0")
    results.check("概率 0.0 合法", con.execute("SELECT jev_prob('s','q')").fetchone()[0] == 0.0)
    configure(con, stub, "boundary-1")
    results.check("概率 1.0 合法", con.execute("SELECT jev_prob('s','q')").fetchone()[0] == 1.0)

    # STRUCT 内部 NULL 与缺失字段在 canonical state 中可区分
    stub.reset()
    configure(con, stub, "ok")
    con.execute("SELECT jev_prob(struct_pack(a:=1, b:=NULL::INTEGER), 'q')")
    con.execute("SELECT jev_prob(struct_pack(a:=1), 'q')")
    with_null = stub.requests[0]["body"]["state"]["rows"][0]
    without_null = stub.requests[1]["body"]["state"]["rows"][0]
    results.check(
        "STRUCT 内部 NULL 保留为 null 字段",
        with_null["f"]["b"]["t"] == "null",
        str(with_null),
    )
    results.check("缺失字段不出现", "b" not in without_null["f"], str(without_null))

    # 跨 DataChunk 边界行对齐：5000 行，stub 精确回显每行状态
    configure(con, stub, "echo-value")
    con.execute("CREATE TABLE big(state VARCHAR, criterion VARCHAR)")
    rows = [(f"{i / 20000:.6f}", f"q{i}") for i in range(5000)]
    con.executemany("INSERT INTO big VALUES (?, ?)", rows)
    stub.reset()
    produced = con.execute("SELECT jev_prob(state, criterion) FROM big").fetchall()
    expected = [float(state) for state, _ in rows]
    results.check(
        "5000 行跨 chunk 无错配",
        [value for (value,) in produced] == expected,
        f"first mismatch at {next((i for i, (p, e) in enumerate(zip(produced, expected)) if p[0] != e), None)}",
    )
    results.check("5000 行请求数与行数一致", len(stub.requests) == 5000, str(len(stub.requests)))

    # 混合 NULL 与重复状态的行对齐（常量响应，便于断言重复状态得到同一结果）
    configure(con, stub, "ok")
    con.execute("CREATE TABLE mixed(state VARCHAR, criterion VARCHAR)")
    con.executemany(
        "INSERT INTO mixed VALUES (?, ?)",
        [("dup", "q"), (None, "q"), ("dup", "q"), ("dup", None)] * 200,
    )
    stub.reset()
    mixed = con.execute("SELECT jev_prob(state, criterion) FROM mixed").fetchall()
    results.check(
        "混合 NULL/重复状态保持行位置",
        all(value is None for (value,) in mixed[1::4]) and all(value is None for (value,) in mixed[3::4]),
    )
    distinct = {value for (value,) in mixed if value is not None}
    results.check("重复状态得到同一结果", len(distinct) == 1, str(distinct))

    # provider 响应错误路径
    for scenario, name in [
        ("out-of-range", "越界概率 1.5 报错"),
        ("negative", "负概率报错"),
        ("nan", "非数值概率报错"),
        ("wrong-type", "概率类型错误报错"),
        ("missing", "缺失 answer 报错"),
        ("extra", "多余 answer 报错"),
        ("unauthorized", "401 报错"),
        ("slow", "超时报错"),
        ("huge", "超大响应报错"),
    ]:
        configure(con, stub, scenario)
        if scenario == "slow":
            con.execute("SET duckjeu_timeout_ms=200")
        if scenario == "huge":
            con.execute("SET duckjeu_max_response_bytes=1024")
        message = expect_error(con, "SELECT jev_prob('s','q')")
        results.check(name, message is not None, "expected an error")


def run_us2(results: Results, extension: pathlib.Path, stub: StubServer) -> None:
    print("\nUS2 — 布尔阈值与限定类别")
    con = connect(extension)

    configure(con, stub, "half")
    results.check("p=0.5 为 true（含边界）", con.execute("SELECT jev_bool('s','q')").fetchone()[0] is True)
    configure(con, stub, "just-below-half")
    results.check("p<0.5 为 false", con.execute("SELECT jev_bool('s','q')").fetchone()[0] is False)
    configure(con, stub, "boundary-1")
    results.check("p=1.0 为 true", con.execute("SELECT jev_bool('s','q')").fetchone()[0] is True)
    configure(con, stub, "boundary-0")
    results.check("p=0.0 为 false", con.execute("SELECT jev_bool('s','q')").fetchone()[0] is False)

    # jev_bool 与 jev_prob 是独立调用（不承诺共享请求）
    configure(con, stub, "ok")
    stub.reset()
    con.execute("SELECT jev_prob('s','q'), jev_bool('s','q')")
    results.check("jev_prob 与 jev_bool 各自发请求", len(stub.requests) == 2, str(len(stub.requests)))

    # choice 正常路径
    configure(con, stub, "ok")
    label = con.execute(
        "SELECT jev_choice('s', 'which regime?', ['normal','scarcity','oversupply'])"
    ).fetchone()[0]
    results.check("choice 返回候选之一", label in {"normal", "scarcity", "oversupply"}, str(label))

    # choice 候选与请求内容
    stub.reset()
    con.execute("SELECT jev_choice('s','which regime?',['normal','scarcity'])")
    body = stub.requests[0]["body"]
    results.check(
        "choice 请求携带候选映射",
        set(body["questions"]["r0"]["criteria"].keys()) == {"normal", "scarcity"},
        str(body["questions"]["r0"].get("criteria")),
    )
    results.check(
        "choice 请求包含调用方问题",
        "which regime?" in body["questions"]["r0"]["instructions"],
        str(body["questions"]["r0"].get("instructions")),
    )

    # choice 错误路径
    configure(con, stub, "unknown-label")
    results.check(
        "未知 label 报错",
        expect_error(con, "SELECT jev_choice('s','q',['normal','scarcity'])") is not None,
    )
    configure(con, stub, "missing")
    results.check(
        "choice 缺失 answer 报错",
        expect_error(con, "SELECT jev_choice('s','q',['normal','scarcity'])") is not None,
    )

    # 输入校验（提交该行前报错）
    configure(con, stub, "ok")
    for sql, name in [
        ("SELECT jev_choice('s','q',['only'])", "少于两个候选报错"),
        ("SELECT jev_choice('s','q',['a','a'])", "重复候选报错"),
        ("SELECT jev_choice('s','q',['a',' '])", "空白候选报错"),
        ("SELECT jev_choice('s','q',['a',NULL])", "候选含 NULL 报错"),
        ("SELECT jev_choice('s','   ',['a','b'])", "空白问题报错"),
        ("SELECT jev_prob('s','  ')", "空白标准报错"),
    ]:
        results.check(name, expect_error(con, sql) is not None)

    # 顶层 choices 为 NULL → NULL 且不发请求
    stub.reset()
    value = con.execute("SELECT jev_choice('s','q',NULL)").fetchone()[0]
    results.check("顶层 NULL choices 结果为 NULL", value is None)
    results.check("顶层 NULL choices 不发请求", len(stub.requests) == 0, str(len(stub.requests)))

    # 跨 chunk 的 choice 行对齐
    configure(con, stub, "echo-value")
    con.execute("CREATE TABLE regimes(state VARCHAR)")
    con.executemany(
        "INSERT INTO regimes VALUES (?)", [("normal",), ("scarcity",), ("oversupply",)] * 200
    )
    produced = [row[0] for row in con.execute(
        "SELECT jev_choice(state, 'q', ['normal','scarcity','oversupply']) FROM regimes"
    ).fetchall()]
    expected = [["normal", "scarcity", "oversupply"][i % 3] for i in range(600)]
    results.check("choice 跨 chunk 行对齐", produced == expected)


def run_us3(results: Results, extension: pathlib.Path, stub: StubServer) -> None:
    print("\nUS3 — 连接级配置、显式开启与错误脱敏")
    con = connect(extension)
    other = con.cursor()

    configure(con, stub, "echo-model")
    other.execute("SET duckjeu_provider='typesafe'")
    other.execute(f"SET duckjeu_api_url='{stub.url('echo-model')}'")
    other.execute("SET duckjeu_model='model-b'")

    con.execute("SELECT jev_prob('s','q')")
    other.execute("SELECT jev_prob('s','q')")
    models = [request["body"]["model"] for request in stub.requests[-2:]]
    results.check("连接级 model 隔离", models == ["jev-latest", "model-b"], str(models))

    # SET 之后重跑 prepared statement 采用新值（配置按执行读取）
    stub.reset()
    con.execute("PREPARE p AS SELECT jev_prob('s','q')")
    con.execute("SELECT 1")  # 让 prepare 生效
    con.execute("SET duckjeu_api_url='" + stub.url("boundary-0") + "'")
    con.execute("EXECUTE p")
    con.execute("SET duckjeu_api_url='" + stub.url("boundary-1") + "'")
    con.execute("EXECUTE p")
    scenarios = [request["scenario"] for request in stub.requests]
    results.check(
        "prepared statement 在 SET 后取新配置",
        scenarios == ["boundary-0", "boundary-1"],
        str(scenarios),
    )

    # 凭据来自环境变量并进入 Authorization header
    stub.reset()
    configure(con, stub, "ok")
    con.execute("SELECT jev_prob('s','q')")
    authorization = stub.requests[0]["authorization"]
    results.check(
        "凭据通过 Authorization header 发送",
        authorization == f"Bearer {os.environ['TYPESAFE_API_KEY']}",
        str(authorization),
    )

    # 401 错误信息不得泄漏凭据或远端正文中的敏感串
    configure(con, stub, "unauthorized")
    message = expect_error(con, "SELECT jev_prob('s','q')") or ""
    results.check("401 报错", bool(message))
    results.check("错误信息不含凭据", os.environ["TYPESAFE_API_KEY"] not in message, message)
    results.check("错误信息不含远端伪造凭据", FAKE_SECRET not in message, message)
    results.check("错误信息不含原始 state", "s" != message.strip(), message)

    # 远端错误正文不进入错误信息（服务端可能回显凭据）
    configure(con, stub, "big-error")
    message = expect_error(con, "SELECT jev_prob('s','q')") or ""
    results.check("远端错误正文不泄漏到错误信息", "yyyy" not in message, message)
    results.check("HTTP 状态码仍可见", "500" in message, message)

    # 缺少凭据时真实 provider 必须报错
    saved = os.environ.pop("TYPESAFE_API_KEY", None)
    override = os.environ.pop("DUCKJEU_API_KEY", None)
    try:
        con.execute("SET duckjeu_provider='typesafe'")
        message = expect_error(con, "SELECT jev_prob('s','q')") or ""
        results.check("缺少凭据时 typesafe 报错", "TYPESAFE_API_KEY" in message, message)
    finally:
        if saved is not None:
            os.environ["TYPESAFE_API_KEY"] = saved
        if override is not None:
            os.environ["DUCKJEU_API_KEY"] = override

    # 未显式开启时保持离线 mock
    con.execute("SET duckjeu_provider='mock'")
    stub.reset()
    con.execute("SELECT jev_prob('s','q')")
    results.check("默认 mock 不发网络请求", len(stub.requests) == 0, str(len(stub.requests)))

    # 未知 provider 名称报错
    con.execute("SET duckjeu_provider='nope'")
    results.check("未知 provider 报错", expect_error(con, "SELECT jev_prob('s','q')") is not None)


if __name__ == "__main__":
    sys.exit(main())
