from __future__ import annotations

import importlib.util
import json
from pathlib import Path


def _load_static_build_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "build_studio_static.py"
    spec = importlib.util.spec_from_file_location("build_studio_static", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Unable to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_build_studio_static_stages_build_client(tmp_path, monkeypatch):
    static_build = _load_static_build_module()
    studio_dir = tmp_path / "studio"
    build_dir = studio_dir / "build" / "client"
    assets_dir = build_dir / "assets"
    assets_dir.mkdir(parents=True)
    (studio_dir / "package.json").write_text(
        json.dumps({"name": "flowai-harness-studio", "version": "1.0.0-alpha.1"}),
        encoding="utf-8",
    )
    (build_dir / "index.html").write_text(
        '<script src="/__flowai_config.js"></script>',
        encoding="utf-8",
    )
    (assets_dir / "entry.js").write_text("console.log('studio');", encoding="utf-8")

    calls = []

    def fake_run(command, *, cwd, check):
        calls.append((command, cwd, check))

    monkeypatch.setattr(static_build.subprocess, "run", fake_run)

    destination = tmp_path / "package-static"
    manifest = static_build.build_studio_static(
        studio_dir=studio_dir,
        static_dir=destination,
        bun="/usr/local/bin/bun",
    )

    assert calls == [
        (["/usr/local/bin/bun", "install", "--frozen-lockfile"], studio_dir, True),
        (["/usr/local/bin/bun", "run", "build"], studio_dir, True),
    ]
    assert (destination / "index.html").exists()
    assert (destination / "assets" / "entry.js").exists()
    assert manifest["source"] == str(build_dir)
    assert manifest["destination"] == str(destination)
    staged_manifest = json.loads(
        (destination / "flowai-studio-static-manifest.json").read_text(encoding="utf-8")
    )
    assert staged_manifest["studioPackageName"] == "flowai-harness-studio"
    assert staged_manifest["studioPackageVersion"] == "1.0.0-alpha.1"


def test_build_studio_static_can_skip_install(tmp_path, monkeypatch):
    static_build = _load_static_build_module()
    studio_dir = tmp_path / "studio"
    build_dir = studio_dir / "build" / "client"
    assets_dir = build_dir / "assets"
    assets_dir.mkdir(parents=True)
    (studio_dir / "package.json").write_text('{"name":"studio"}', encoding="utf-8")
    (build_dir / "index.html").write_text(
        '<script src="/__flowai_config.js"></script>',
        encoding="utf-8",
    )
    (assets_dir / "entry.js").write_text("", encoding="utf-8")

    calls = []

    def fake_run(command, *, cwd, check):
        calls.append(command)

    monkeypatch.setattr(static_build.subprocess, "run", fake_run)

    static_build.build_studio_static(
        studio_dir=studio_dir,
        static_dir=tmp_path / "static",
        bun="bun",
        skip_install=True,
    )

    assert calls == [["bun", "run", "build"]]
