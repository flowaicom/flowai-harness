from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Any


class StudioStaticBuildError(RuntimeError):
    """Raised when the Studio static artifact cannot be built or staged."""


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="build_studio_static.py",
        description="Build the shared Studio UI and stage it for Python packaging.",
    )
    parser.add_argument(
        "--studio-dir",
        help="Path to the shared Studio frontend source directory. Defaults to ./studio.",
    )
    parser.add_argument(
        "--static-dir",
        help=(
            "Destination directory for packaged static assets. Defaults to "
            "./py-flowai-harness/flowai_harness/studio/static."
        ),
    )
    parser.add_argument(
        "--bun",
        help="Path to the bun executable. Defaults to the first bun on PATH.",
    )
    parser.add_argument(
        "--skip-install",
        action="store_true",
        help="Skip `bun install` and run only `bun run build`.",
    )
    parser.add_argument(
        "--no-frozen-lockfile",
        action="store_true",
        help="Run `bun install` without --frozen-lockfile.",
    )
    options = parser.parse_args(argv)

    try:
        manifest = build_studio_static(
            studio_dir=Path(options.studio_dir) if options.studio_dir else None,
            static_dir=Path(options.static_dir) if options.static_dir else None,
            bun=options.bun,
            skip_install=options.skip_install,
            frozen_lockfile=not options.no_frozen_lockfile,
        )
    except StudioStaticBuildError as exc:
        print(str(exc), file=sys.stderr)
        return 2

    print(
        "Staged FlowAI Studio static artifact at "
        f"{manifest['destination']} from {manifest['source']}"
    )
    return 0


def build_studio_static(
    *,
    studio_dir: Path | None = None,
    static_dir: Path | None = None,
    bun: str | None = None,
    skip_install: bool = False,
    frozen_lockfile: bool = True,
) -> dict[str, Any]:
    source_dir = _resolve_studio_dir(studio_dir)
    destination = _resolve_static_dir(static_dir)
    bun_path = _resolve_bun(bun)

    if not skip_install:
        install_command = [bun_path, "install"]
        if frozen_lockfile:
            install_command.append("--frozen-lockfile")
        _run(install_command, cwd=source_dir)

    _run([bun_path, "run", "build"], cwd=source_dir)

    build_dir = source_dir / "build" / "client"
    _validate_build_dir(build_dir)

    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True, exist_ok=True)
    _copy_tree_contents(build_dir, destination)

    package = _read_package_json(source_dir / "package.json")
    manifest = {
        "source": str(build_dir),
        "destination": str(destination),
        "builtAt": datetime.now(timezone.utc).isoformat(),
        "studioPackageName": package.get("name"),
        "studioPackageVersion": package.get("version"),
    }
    (destination / "flowai-studio-static-manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    _validate_staged_dir(destination)
    return manifest


def _resolve_repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def _resolve_studio_dir(studio_dir: Path | None) -> Path:
    if studio_dir is not None:
        candidate = studio_dir.expanduser().resolve()
        if (candidate / "package.json").exists():
            return candidate
        raise StudioStaticBuildError(
            f"Studio source directory {candidate} does not contain package.json."
        )

    candidate = _resolve_repo_root() / "studio"
    if (candidate / "package.json").exists():
        return candidate
    raise StudioStaticBuildError(
        "Unable to locate Studio source directory. Pass --studio-dir."
    )


def _resolve_static_dir(static_dir: Path | None) -> Path:
    if static_dir is not None:
        return static_dir.expanduser().resolve()
    return (
        _resolve_repo_root()
        / "py-flowai-harness"
        / "flowai_harness"
        / "studio"
        / "static"
    )


def _resolve_bun(bun: str | None) -> str:
    candidate = bun or shutil.which("bun")
    if candidate:
        return candidate
    raise StudioStaticBuildError(
        "Unable to build Studio static assets: `bun` was not found on PATH."
    )


def _run(command: list[str], *, cwd: Path) -> None:
    try:
        subprocess.run(command, cwd=cwd, check=True)
    except FileNotFoundError as exc:
        raise StudioStaticBuildError(f"Unable to run {command[0]!r}.") from exc
    except subprocess.CalledProcessError as exc:
        rendered = " ".join(command)
        raise StudioStaticBuildError(
            f"Studio static build command failed: {rendered}"
        ) from exc


def _copy_tree_contents(source: Path, destination: Path) -> None:
    for child in source.iterdir():
        target = destination / child.name
        if child.is_dir():
            shutil.copytree(child, target)
        else:
            shutil.copy2(child, target)


def _validate_build_dir(build_dir: Path) -> None:
    if not build_dir.exists():
        raise StudioStaticBuildError(
            f"Studio build output was not found at {build_dir}."
        )
    _validate_staged_dir(build_dir)


def _validate_staged_dir(directory: Path) -> None:
    index = directory / "index.html"
    assets = directory / "assets"
    if not index.exists():
        raise StudioStaticBuildError(f"Studio artifact is missing {index}.")
    if not assets.is_dir():
        raise StudioStaticBuildError(f"Studio artifact is missing {assets}.")
    html = index.read_text(encoding="utf-8")
    if "/__flowai_config.js" not in html:
        raise StudioStaticBuildError(
            "Studio artifact index.html does not load /__flowai_config.js."
        )


def _read_package_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise StudioStaticBuildError(f"Unable to read {path}.") from exc
    except json.JSONDecodeError as exc:
        raise StudioStaticBuildError(f"Unable to parse {path}.") from exc
    if not isinstance(value, dict):
        raise StudioStaticBuildError(f"{path} must contain a JSON object.")
    return value


if __name__ == "__main__":
    raise SystemExit(main())
