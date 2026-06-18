import json
import select
import subprocess
import sys
import time
from pathlib import Path

import pytest

from flowai_harness import cli


def test_mcp_python_help_exits_successfully(capsys):
    with pytest.raises(SystemExit) as exc:
        cli.main(["flowai-harness", "mcp", "python", "--help"])

    assert exc.value.code == 0
    assert "MODULE:OBJECT" in capsys.readouterr().out


def test_non_mcp_command_delegates_to_rust_cli(monkeypatch):
    calls = []

    def fake_run_cli(args):
        calls.append(args)
        return 7

    monkeypatch.setattr(cli._internal, "run_cli", fake_run_cli)

    assert cli.main(["flowai-harness", "data", "--help"]) == 7
    assert calls == [["flowai-harness", "data", "--help"]]


def test_mcp_toolkit_command_delegates_to_rust_cli(monkeypatch):
    calls = []

    def fake_run_cli(args):
        calls.append(args)
        return 0

    monkeypatch.setattr(cli._internal, "run_cli", fake_run_cli)

    assert cli.main(["flowai-harness", "mcp", "toolkit", "--help"]) == 0
    assert calls == [["flowai-harness", "mcp", "toolkit", "--help"]]


def test_import_resolver_accepts_runtime_object():
    runtime = cli._load_runtime_target("tests.fixtures.mcp_app:runtime")

    assert callable(runtime.list_mcp_tools)


def test_import_resolver_accepts_runtime_factory():
    runtime = cli._load_runtime_target("tests.fixtures.mcp_app:build_runtime")

    assert callable(runtime.list_mcp_tools)


def test_mcp_python_stdio_subprocess_lists_and_calls_custom_tool():
    proc = _start_python_mcp("--transport", "stdio")
    try:
        _write_json(
            proc,
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "pytest", "version": "0.1.0"},
                },
            },
        )
        initialize = _read_json(proc)
        assert initialize["id"] == 1
        assert "result" in initialize

        _write_json(proc, {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        tools = _read_json(proc)
        assert {tool["name"] for tool in tools["result"]["tools"]} >= {"echo", "async_echo"}

        _write_json(
            proc,
            {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {"name": "echo", "arguments": {"message": "hello"}},
            },
        )
        call = _read_json(proc)
        content = json.loads(call["result"]["content"][0]["text"])
        assert content["message"] == "hello"
    finally:
        _terminate(proc)


def test_mcp_python_streamable_http_subprocess_lists_and_calls_custom_tool():
    proc = _start_python_mcp("--transport", "streamable-http", "--port", "0")
    try:
        endpoint = _read_endpoint(proc)
        initialize = _post_mcp(
            endpoint,
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "pytest", "version": "0.1.0"},
                },
            },
        )
        assert initialize["id"] == 1

        tools = _post_mcp(
            endpoint,
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        )
        assert {tool["name"] for tool in tools["result"]["tools"]} >= {"echo", "async_echo"}

        call = _post_mcp(
            endpoint,
            {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {"name": "echo", "arguments": {"message": "hello"}},
            },
        )
        content = json.loads(call["result"]["content"][0]["text"])
        assert content["message"] == "hello"
    finally:
        _terminate(proc)


def _start_python_mcp(*extra_args):
    return subprocess.Popen(
        [
            sys.executable,
            "-m",
            "flowai_harness.cli",
            "mcp",
            "python",
            "tests.fixtures.mcp_app:build_runtime",
            "--agent",
            "mcp",
            *extra_args,
        ],
        cwd=Path(__file__).resolve().parents[1],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )


def _write_json(proc, payload):
    assert proc.stdin is not None
    proc.stdin.write(json.dumps(payload) + "\n")
    proc.stdin.flush()


def _read_json(proc):
    assert proc.stdout is not None
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        ready, _, _ = select.select([proc.stdout], [], [], 0.1)
        if not ready:
            _assert_running(proc)
            continue
        line = proc.stdout.readline()
        if not line:
            _assert_running(proc)
            continue
        if line.startswith("data: "):
            line = line.removeprefix("data: ")
        return json.loads(line)
    raise AssertionError(_process_output(proc, "timed out waiting for stdout JSON"))


def _read_endpoint(proc):
    assert proc.stderr is not None
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        ready, _, _ = select.select([proc.stderr], [], [], 0.1)
        if not ready:
            _assert_running(proc)
            continue
        line = proc.stderr.readline()
        if "http://" in line:
            return line[line.index("http://") :].strip()
    raise AssertionError(_process_output(proc, "timed out waiting for MCP endpoint"))


def _post_mcp(endpoint, payload):
    from urllib import request

    req = request.Request(
        endpoint,
        data=json.dumps(payload).encode(),
        headers={
            "Accept": "application/json, text/event-stream",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    with request.urlopen(req, timeout=10) as response:
        body = response.read().decode()
    line = next((line.removeprefix("data: ") for line in body.splitlines() if line), body)
    return json.loads(line)


def _assert_running(proc):
    if proc.poll() is not None:
        raise AssertionError(_process_output(proc, "MCP subprocess exited early"))


def _process_output(proc, message):
    stdout = _drain_available(proc.stdout)
    stderr = _drain_available(proc.stderr)
    return f"{message}; exit={proc.poll()} stdout={stdout!r} stderr={stderr!r}"


def _terminate(proc):
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)


def _drain_available(pipe):
    if pipe is None:
        return ""
    chunks = []
    while True:
        ready, _, _ = select.select([pipe], [], [], 0)
        if not ready:
            break
        line = pipe.readline()
        if not line:
            break
        chunks.append(line)
    return "".join(chunks)
