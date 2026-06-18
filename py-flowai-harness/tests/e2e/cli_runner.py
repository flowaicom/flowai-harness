from __future__ import annotations

import os
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import IO, Mapping, Sequence

from tests.e2e.progress import Progress


@dataclass(frozen=True)
class CliRun:
    args: list[str]
    returncode: int
    stdout: str
    stderr: str


def run_flowai_harness(
    args: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str] | None = None,
    timeout: int = 120,
    check: bool = True,
    progress: Progress | None = None,
) -> CliRun:
    command = [sys.executable, "-m", "flowai_harness.cli", *args]
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)
    if progress:
        progress.log(f"running CLI: {' '.join(args)}")
    process = subprocess.Popen(
        command,
        cwd=cwd,
        env=merged_env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    stdout_lines: list[str] = []
    stderr_lines: list[str] = []
    stdout_thread = _start_drain_thread(process.stdout, stdout_lines, progress, "stdout")
    stderr_thread = _start_drain_thread(process.stderr, stderr_lines, progress, "stderr")
    started_at = time.monotonic()
    last_heartbeat_at = started_at
    try:
        while process.poll() is None:
            time.sleep(0.5)
            if time.monotonic() - started_at >= timeout:
                raise subprocess.TimeoutExpired(command, timeout)
            if progress and progress.heartbeat_due(last_heartbeat_at):
                progress.log(f"still running CLI: {' '.join(args)}")
                last_heartbeat_at = time.monotonic()
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)
        _join_drain_thread(stdout_thread)
        _join_drain_thread(stderr_thread)
        stdout = "".join(stdout_lines)
        stderr = "".join(stderr_lines)
        if progress:
            progress.log(f"timed out after {timeout}s: {' '.join(args)}")
        raise AssertionError(
            f"flowai-harness timed out after {timeout}s\n"
            f"command: {' '.join(command)}\n"
            f"stdout:\n{stdout}\n"
            f"stderr:\n{stderr}"
        ) from None
    _join_drain_thread(stdout_thread)
    _join_drain_thread(stderr_thread)
    stdout = "".join(stdout_lines)
    stderr = "".join(stderr_lines)
    result = CliRun(
        args=command,
        returncode=process.returncode,
        stdout=stdout,
        stderr=stderr,
    )
    if progress:
        progress.log(
            f"CLI exited {process.returncode}: {' '.join(args)}"
        )
    if check and process.returncode != 0:
        raise AssertionError(
            f"flowai-harness exited {process.returncode}\n"
            f"command: {' '.join(command)}\n"
            f"stdout:\n{stdout}\n"
            f"stderr:\n{stderr}"
        )
    return result


def _start_drain_thread(
    stream: IO[str] | None,
    lines: list[str],
    progress: Progress | None,
    label: str,
) -> threading.Thread | None:
    if stream is None:
        return None
    thread = threading.Thread(
        target=_drain_stream,
        args=(stream, lines, progress, label),
        daemon=True,
    )
    thread.start()
    return thread


def _join_drain_thread(thread: threading.Thread | None) -> None:
    if thread is not None:
        thread.join(timeout=5)


def _drain_stream(
    stream: IO[str],
    lines: list[str],
    progress: Progress | None,
    label: str,
) -> None:
    for line in iter(stream.readline, ""):
        lines.append(line)
        if progress and line.strip():
            progress.log(f"CLI {label}: {line.rstrip()[:500]}")
