from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess


REPO_ROOT = Path(__file__).resolve().parents[2]
BASH = shutil.which("bash") or "/bin/bash"


def _write_tool(bin_dir: Path, name: str, body: str) -> None:
    path = bin_dir / name
    path.write_text("#!/bin/sh\nset -eu\n" + body, encoding="utf-8")
    path.chmod(0o755)


def _run_script(script: str, *, path: Path, args: list[str] | None = None) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env["PATH"] = str(path)
    return subprocess.run(
        [BASH, str(REPO_ROOT / "scripts" / script), *(args or [])],
        cwd=REPO_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )


def test_check_env_accepts_required_preview_tool_versions(tmp_path: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    _write_tool(bin_dir, "rustc", 'echo "rustc 1.88.0 (abcdef 2025-06-01)"\n')
    _write_tool(bin_dir, "cargo", 'echo "cargo 1.88.0 (abcdef 2025-06-01)"\n')
    _write_tool(bin_dir, "python3.12", 'echo "Python 3.12.13"\n')
    _write_tool(bin_dir, "uv", 'echo "uv 0.10.9"\n')
    _write_tool(bin_dir, "bun", 'echo "1.3.5"\n')

    result = _run_script("check-env.sh", path=bin_dir)

    assert result.returncode == 0, result.stderr
    assert "rustc: ok (1.88.0)" in result.stdout
    assert "cargo: ok (1.88.0)" in result.stdout
    assert "python: ok (3.12.13)" in result.stdout
    assert "uv: ok (0.10.9)" in result.stdout
    assert "bun: ok (1.3.5)" in result.stdout
    assert "Flow AI preview environment is ready." in result.stdout


def test_check_env_reports_incompatible_rust_before_install(tmp_path: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    _write_tool(bin_dir, "rustc", 'echo "rustc 1.87.0 (abcdef 2025-01-01)"\n')
    _write_tool(bin_dir, "cargo", 'echo "cargo 1.94.0 (85eff7c80 2026-01-15)"\n')
    _write_tool(bin_dir, "python3.12", 'echo "Python 3.12.13"\n')
    _write_tool(bin_dir, "uv", 'echo "uv 0.10.9"\n')
    _write_tool(bin_dir, "bun", 'echo "1.3.5"\n')

    result = _run_script("check-env.sh", path=bin_dir)

    assert result.returncode == 1
    assert "rustc: incompatible (1.87.0; expected >= 1.88.0)" in result.stdout
    assert "Run ./scripts/setup-env.sh to install missing or incompatible tools." in result.stdout


def test_check_env_requires_cargo_for_rust_source_builds(tmp_path: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    _write_tool(bin_dir, "rustc", 'echo "rustc 1.94.0 (4a4ef493e 2026-03-02)"\n')
    _write_tool(bin_dir, "python3.12", 'echo "Python 3.12.13"\n')
    _write_tool(bin_dir, "uv", 'echo "uv 0.10.9"\n')
    _write_tool(bin_dir, "bun", 'echo "1.3.5"\n')

    result = _run_script("check-env.sh", path=bin_dir)

    assert result.returncode == 1
    assert "rustc: ok (1.94.0)" in result.stdout
    assert "cargo: missing (expected >= 1.88.0)" in result.stdout


def test_setup_env_dry_run_lists_pinned_installs_when_tools_are_missing(tmp_path: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()

    result = _run_script("setup-env.sh", path=bin_dir, args=["--dry-run"])

    assert result.returncode == 0, result.stderr
    assert "Would install uv 0.10.9" in result.stdout
    assert "Would install Python 3.12.13 via uv" in result.stdout
    assert "Would install Rust 1.94.0 via rustup" in result.stdout
    assert "Would install Bun 1.3.5" in result.stdout


def test_setup_env_keeps_manually_installed_minimum_rust_toolchain(tmp_path: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    _write_tool(bin_dir, "rustc", 'echo "rustc 1.88.0 (abcdef 2025-06-01)"\n')
    _write_tool(bin_dir, "cargo", 'echo "cargo 1.88.0 (abcdef 2025-06-01)"\n')

    result = _run_script("setup-env.sh", path=bin_dir, args=["--dry-run"])

    assert result.returncode == 0, result.stderr
    assert "Rust: already compatible" in result.stdout
    assert "Would install Rust 1.94.0 via rustup" not in result.stdout


def test_install_dry_run_stops_when_environment_check_fails(tmp_path: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    _write_tool(bin_dir, "rustc", 'echo "rustc 1.87.0 (abcdef 2025-01-01)"\n')
    _write_tool(bin_dir, "cargo", 'echo "cargo 1.94.0 (85eff7c80 2026-01-15)"\n')
    _write_tool(bin_dir, "python3.12", 'echo "Python 3.12.13"\n')
    _write_tool(bin_dir, "uv", 'echo "uv 0.10.9"\n')
    _write_tool(bin_dir, "bun", 'echo "1.3.5"\n')

    result = _run_script("install.sh", path=bin_dir, args=["--dry-run"])

    assert result.returncode == 1
    assert "Environment check failed." in result.stdout
    assert "Fix the dependency above, or run ./scripts/setup-env.sh." in result.stdout
    assert "Would run:" not in result.stdout


def test_install_dry_run_shows_studio_build_and_harness_install(tmp_path: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    _write_tool(bin_dir, "rustc", 'echo "rustc 1.94.0 (4a4ef493e 2026-03-02)"\n')
    _write_tool(bin_dir, "cargo", 'echo "cargo 1.94.0 (85eff7c80 2026-01-15)"\n')
    _write_tool(bin_dir, "python3.12", 'echo "Python 3.12.13"\n')
    _write_tool(bin_dir, "uv", 'echo "uv 0.10.9"\n')
    _write_tool(bin_dir, "bun", 'echo "1.3.5"\n')

    result = _run_script("install.sh", path=bin_dir, args=["--dry-run"])

    assert result.returncode == 0, result.stderr
    expected_commands = [
        "bun install --cwd studio --frozen-lockfile",
        "uv venv .venv --python 3.12 --clear",
        ".venv/bin/python scripts/build_studio_static.py --skip-install",
        "uv pip install --python .venv/bin/python ./py-flowai-harness",
        ".venv/bin/flowai-harness --version",
    ]
    for command in expected_commands:
        assert f"Would run: {command}" in result.stdout
    assert "Dry run complete. Re-run without --dry-run to install." in result.stdout
