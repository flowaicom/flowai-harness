from __future__ import annotations

import json
import subprocess
import time
from dataclasses import dataclass
from typing import Callable, Sequence

from tests.e2e.progress import Progress


Runner = Callable[..., subprocess.CompletedProcess[str]]


@dataclass(frozen=True)
class NeonBranch:
    id: str
    name: str


class NeonClient:
    def __init__(
        self,
        *,
        project_id: str,
        runner: Runner | None = None,
        progress: Progress | None = None,
    ) -> None:
        project_id = project_id.strip()
        if not project_id:
            raise ValueError("project_id must not be blank")
        self.project_id = project_id
        self._runner = runner or subprocess.run
        self._progress = progress

    def create_branch(self, *, name: str, parent: str, expires_at: str) -> NeonBranch:
        completed = self._run(
            [
                "neonctl",
                "branches",
                "create",
                "--project-id",
                self.project_id,
                "--name",
                name,
                "--parent",
                parent,
                "--expires-at",
                expires_at,
                "-o",
                "json",
            ]
        )
        payload = json.loads(completed.stdout)
        branch = payload.get("branch", payload)
        return NeonBranch(id=branch["id"], name=branch["name"])

    def delete_branch(self, branch: NeonBranch | str) -> None:
        branch_ref = branch.id if isinstance(branch, NeonBranch) else branch
        self._run(
            [
                "neonctl",
                "branches",
                "delete",
                branch_ref,
                "--project-id",
                self.project_id,
            ],
            check=False,
        )

    def connection_string(
        self,
        *,
        branch: str,
        database: str,
        role: str,
        pooled: bool = False,
    ) -> str:
        args = [
            "neonctl",
            "connection-string",
            branch,
            "--project-id",
            self.project_id,
            "--database-name",
            database,
            "--role-name",
            role,
            "--ssl",
            "require",
        ]
        if pooled:
            args.append("--pooled")
        completed = self._run(args)
        return completed.stdout.strip()

    def _run(
        self,
        args: Sequence[str],
        *,
        check: bool = True,
    ) -> subprocess.CompletedProcess[str]:
        command = list(args)
        if self._progress:
            self._progress.log(f"running neonctl: {_safe_command(command)}")
        completed = self._run_subprocess(command)
        if self._progress:
            self._progress.log(
                f"neonctl exited {completed.returncode}: {_safe_command(command)}"
            )
        if check and completed.returncode != 0:
            raise RuntimeError(
                "neonctl command failed\n"
                f"command: {_safe_command(command)}\n"
                f"exit: {completed.returncode}\n"
                f"stdout:\n{completed.stdout}\n"
                f"stderr:\n{completed.stderr}"
            )
        return completed

    def _run_subprocess(self, command: list[str]) -> subprocess.CompletedProcess[str]:
        if self._runner is not subprocess.run:
            return self._runner(
                command,
                check=False,
                capture_output=True,
                text=True,
            )

        process = subprocess.Popen(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        last_heartbeat_at = time.monotonic()
        while process.poll() is None:
            time.sleep(0.5)
            if self._progress and self._progress.heartbeat_due(last_heartbeat_at):
                self._progress.log(f"still running neonctl: {_safe_command(command)}")
                last_heartbeat_at = time.monotonic()
        stdout, stderr = process.communicate()
        return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


def _safe_command(args: Sequence[str]) -> str:
    return " ".join(args)
