from __future__ import annotations

import sys
import time
from contextlib import contextmanager
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterator


@dataclass
class Progress:
    prefix: str = "flowai-e2e"
    started_at: float = field(default_factory=time.monotonic)
    heartbeat_secs: float = 15.0
    tty_path: Path = Path("/dev/tty")

    def log(self, message: str) -> None:
        elapsed = time.monotonic() - self.started_at
        self._write(f"[{self.prefix} +{elapsed:6.1f}s] {message}")

    @contextmanager
    def step(self, message: str) -> Iterator[None]:
        step_started = time.monotonic()
        self.log(f"start: {message}")
        try:
            yield
        except BaseException:
            elapsed = time.monotonic() - step_started
            self.log(f"failed after {elapsed:.1f}s: {message}")
            raise
        elapsed = time.monotonic() - step_started
        self.log(f"done in {elapsed:.1f}s: {message}")

    def heartbeat_due(self, last_heartbeat_at: float) -> bool:
        return time.monotonic() - last_heartbeat_at >= self.heartbeat_secs

    def _write(self, line: str) -> None:
        try:
            with self.tty_path.open("a", encoding="utf-8") as tty:
                print(line, file=tty, flush=True)
                return
        except OSError:
            print(line, file=sys.stderr, flush=True)
