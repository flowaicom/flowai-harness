from __future__ import annotations

import json
from pathlib import Path


def test_public_artifact_validation_rejects_placeholder_metadata():
    from inventory_scenario.support.dataset_artifacts.verify_public_artifact import (
        PublicArtifactError,
        validate_manifest_release_metadata,
    )

    manifest = {
        "dataset_version": "2026-06",
        "artifacts": {
            "target_sqlite_zst": {
                "url": "https://github.com/flowaicom/flowai-harness/releases/download/inventory-scenario-v2026-06/inventory-scenario-v2026-06.target.sqlite.zst",
                "sha256": "0" * 64,
                "uncompressed_bytes": 0,
            }
        },
    }

    try:
        validate_manifest_release_metadata(manifest)
    except PublicArtifactError as exc:
        assert "placeholder SHA-256" in str(exc)
    else:  # pragma: no cover - makes the assertion failure clearer
        raise AssertionError("placeholder manifest metadata was accepted")


def test_default_manifest_path_points_to_example_data_directory():
    from inventory_scenario.support.dataset_artifacts.verify_public_artifact import (
        _default_manifest_path,
    )

    manifest_path = _default_manifest_path()

    assert manifest_path == Path(__file__).resolve().parents[1] / "data" / "manifest.example.json"
    assert manifest_path.exists()


def test_public_artifact_validation_accepts_published_release_metadata():
    from inventory_scenario.support.dataset_artifacts.verify_public_artifact import (
        validate_manifest_release_metadata,
    )

    manifest = {
        "dataset_version": "2026-06",
        "artifacts": {
            "target_sqlite_zst": {
                "url": "https://github.com/flowaicom/flowai-harness/releases/download/inventory-scenario-v2026-06/inventory-scenario-v2026-06.target.sqlite.zst",
                "sha256": "a" * 64,
                "uncompressed_bytes": 123456,
            }
        },
    }

    metadata = validate_manifest_release_metadata(manifest)

    assert metadata.url.endswith(".target.sqlite.zst")
    assert metadata.sha256 == "a" * 64
    assert metadata.uncompressed_bytes == 123456


def test_verify_public_artifact_main_reports_placeholder_manifest(
    tmp_path: Path,
    capsys,
):
    from inventory_scenario.support.dataset_artifacts.verify_public_artifact import main

    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        json.dumps(
            {
                "dataset_version": "2026-06",
                "artifacts": {
                    "target_sqlite_zst": {
                        "url": "https://github.com/flowaicom/flowai-harness/releases/download/inventory-scenario-v2026-06/inventory-scenario-v2026-06.target.sqlite.zst",
                        "sha256": "0" * 64,
                        "uncompressed_bytes": 0,
                    }
                },
            }
        )
    )

    exit_code = main(["--manifest", str(manifest_path)])

    assert exit_code == 2
    assert "placeholder SHA-256" in capsys.readouterr().err
