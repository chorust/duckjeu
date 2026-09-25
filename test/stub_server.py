"""脚本化 HTTP stub：为 SQL 契约测试提供可控的 provider 响应。

场景通过 URL path 选择，例如 `http://127.0.0.1:PORT/echo-value`。
测试用 `duckjeu_api_url` 指向不同场景来覆盖正常与全部错误路径。

支持的场景：
  ok              noul → 0.83；choice → 第一个候选
  boundary-0      noul → 0.0
  boundary-1      noul → 1.0
  half            noul → 0.5（jev_bool 的含边界阈值）
  just-below-half noul → 0.499999
  echo-value      noul → state.rows[0] 的数值；choice → state.rows[0] 文本
  echo-model      回显请求中的 model
  out-of-range    noul → 1.5
  negative        noul → -0.1
  nan             noul → "NaN"（字符串，类型错误）
  unknown-label   choice → 不在候选集内的 label
  missing         answers 为空对象
  extra           answers 含多余 key
  wrong-type      noul → 字符串
  unauthorized    401，正文含伪造凭据串
  slow            延迟 2s（触发超时）
  slow-short      延迟 1ms（并发上限合约）
  huge            返回超过 1MiB 的正文
  big-error       500，正文远超 300 字符（验证错误信息截断）
"""

from __future__ import annotations

import json
import re
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

FAKE_SECRET = "sk-stub-should-never-leak-0123456789"


def _answers_for(scenario: str, request: dict) -> tuple[int, dict]:
    model = request.get("model", "unknown")
    questions = request.get("questions") or {}
    state = request.get("state") or {}
    rows = state.get("rows") or [None]

    if scenario == "echo-batch":
        answers = {}
        for question_key, question in questions.items():
            kind = question.get("type", "noul")
            match = re.search(
                r"state\.rows\[(\d+)\]", question.get("instructions", "")
            )
            if match is None:
                return 400, {"error": "question must identify one row"}
            row_index = int(match.group(1))
            if row_index >= len(rows):
                return 400, {"error": "question row index is out of range"}
            row = rows[row_index]
            if kind == "choice":
                candidates = list((question.get("criteria") or {}).keys())
                value = row if row in candidates else "unknown"
                answers[question_key] = {
                    "type": "choice",
                    "choice": value,
                    "confidence": 1.0,
                }
            else:
                try:
                    value = float(row)
                except (TypeError, ValueError):
                    return 400, {"error": "noul echo row must be numeric text"}
                answers[question_key] = {"type": "noul", "noul": value}
        reversed_answers = dict(reversed(list(answers.items())))
        return 200, {"model": model, "answers": reversed_answers}

    if scenario == "ok" and len(questions) > 1:
        answers = {}
        for question_key, batch_question in questions.items():
            batch_kind = batch_question.get("type", "noul")
            if batch_kind == "choice":
                candidates = list((batch_question.get("criteria") or {}).keys())
                value = candidates[0] if candidates else "unknown"
                answers[question_key] = {
                    "type": "choice",
                    "choice": value,
                    "confidence": 1.0,
                    "probabilities": {
                        label: float(label == value) for label in candidates
                    },
                }
            else:
                answers[question_key] = {"type": "noul", "noul": 0.83}
        return 200, {"model": model, "answers": dict(reversed(list(answers.items())))}

    key = next(iter(questions)) if questions else "r0"
    question = questions.get(key) or {}
    kind = question.get("type", "noul")
    row = rows[0]

    def typed_answer(value):
        if kind == "choice":
            criteria = list((question.get("criteria") or {}).keys())
            probabilities = {
                label: (1.0 if label == value else 0.0) for label in criteria
            }
            return {
                "type": "choice",
                "choice": value,
                "confidence": 1.0,
                "probabilities": probabilities,
            }
        return {"type": "noul", "noul": value}

    def answer(value):
        return 200, {"model": model, "answers": {key: typed_answer(value)}}

    if scenario == "ok":
        if kind == "choice":
            criteria = list((question.get("criteria") or {}).keys())
            return answer(criteria[0] if criteria else "unknown")
        return answer(0.83)
    if scenario == "boundary-0":
        return answer(0.0)
    if scenario == "boundary-1":
        return answer(1.0)
    if scenario == "half":
        return answer(0.5)
    if scenario == "just-below-half":
        return answer(0.499999)
    if scenario == "echo-value":
        if kind == "choice":
            return answer(row if isinstance(row, str) else "unknown")
        return answer(float(row) if isinstance(row, str) else row)
    if scenario == "echo-model":
        return answer(0.5)
    if scenario == "out-of-range":
        return answer(1.5)
    if scenario == "negative":
        return answer(-0.1)
    if scenario == "nan":
        return answer("NaN")
    if scenario == "unknown-label":
        return answer("not-a-candidate")
    if scenario == "missing":
        return 200, {"model": model, "answers": {}}
    if scenario == "extra":
        value = typed_answer(0.5)
        return 200, {"model": model, "answers": {key: value, "r1": value}}
    if scenario == "wrong-type":
        return answer("0.5")
    if scenario == "unauthorized":
        return 401, {"error": "invalid api key", "received": FAKE_SECRET}
    if scenario == "slow":
        time.sleep(2.0)
        return answer(0.5)
    if scenario == "slow-short":
        time.sleep(0.001)
        return answer(0.5)
    if scenario == "big-error":
        return 500, {"error": "y" * 5000}
    if scenario == "huge":
        return 200, {
            "model": model,
            "padding": "x" * (2 * 1024 * 1024),
            "answers": {key: typed_answer(0.5)},
        }
    return 404, {"error": f"unknown scenario {scenario}"}


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):  # 保持测试输出干净
        pass

    def do_POST(self):  # noqa: N802
        scenario = self.path.strip("/").split("/")[-1] or "ok"
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length else b"{}"
        self.server.requests.append(  # type: ignore[attr-defined]
            {
                "scenario": scenario,
                "authorization": self.headers.get("Authorization"),
                "body": json.loads(raw.decode("utf-8") or "{}"),
            }
        )
        try:
            request = json.loads(raw.decode("utf-8") or "{}")
        except json.JSONDecodeError:
            request = {}
        with self.server.active_lock:  # type: ignore[attr-defined]
            self.server.active_requests += 1  # type: ignore[attr-defined]
            self.server.peak_active_requests = max(  # type: ignore[attr-defined]
                self.server.peak_active_requests,  # type: ignore[attr-defined]
                self.server.active_requests,  # type: ignore[attr-defined]
            )
        try:
            status, payload = _answers_for(scenario, request)
            encoded = json.dumps(payload).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            try:
                self.wfile.write(encoded)
            except (BrokenPipeError, ConnectionResetError):
                # Expected when a timeout test closes the client socket before the stub replies.
                pass
        finally:
            with self.server.active_lock:  # type: ignore[attr-defined]
                self.server.active_requests -= 1  # type: ignore[attr-defined]


class _QuietThreadingHTTPServer(ThreadingHTTPServer):
    def handle_error(self, request, client_address):
        error = sys.exc_info()[1]
        if isinstance(error, (BrokenPipeError, ConnectionResetError)):
            # Timeouts intentionally close sockets while a scripted response is pending.
            return
        super().handle_error(request, client_address)


class StubServer:
    """在后台线程提供 stub HTTP 服务。"""

    def __init__(self, host: str = "127.0.0.1", port: int = 0):
        self._server = _QuietThreadingHTTPServer((host, port), _Handler)
        self._server.requests = []  # type: ignore[attr-defined]
        self._server.active_lock = threading.Lock()  # type: ignore[attr-defined]
        self._server.active_requests = 0  # type: ignore[attr-defined]
        self._server.peak_active_requests = 0  # type: ignore[attr-defined]
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    def __enter__(self) -> "StubServer":
        self._thread.start()
        return self

    def __exit__(self, *_exc) -> None:
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)

    @property
    def port(self) -> int:
        return self._server.server_address[1]

    def url(self, scenario: str) -> str:
        return f"http://127.0.0.1:{self.port}/v1/{scenario}"

    @property
    def requests(self) -> list[dict]:
        return self._server.requests  # type: ignore[attr-defined]

    @property
    def request_count(self) -> int:
        """Actual HTTP attempts observed by the local server, including error responses."""
        return len(self.requests)

    @property
    def peak_concurrent_requests(self) -> int:
        """Highest number of requests being processed simultaneously since reset."""
        with self._server.active_lock:  # type: ignore[attr-defined]
            return self._server.peak_active_requests  # type: ignore[attr-defined]

    def reset(self) -> None:
        self.requests.clear()
        with self._server.active_lock:  # type: ignore[attr-defined]
            self._server.peak_active_requests = self._server.active_requests  # type: ignore[attr-defined]


if __name__ == "__main__":
    import sys

    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8000
    with StubServer(port=port) as server:
        print(f"stub listening on http://127.0.0.1:{server.port}")
        try:
            while True:
                time.sleep(1)
        except KeyboardInterrupt:
            pass
