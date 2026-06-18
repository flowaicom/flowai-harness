from __future__ import annotations

import json
import logging
import sqlite3
from pathlib import Path

import pytest

from inventory_scenario.support.seed import (
    ArtifactChecksumError,
    SeedOptions,
    SeedError,
    _default_manifest_path,
    hash_file,
    run_seed,
    verify_sha256,
)
from fixtures.sqlite import create_tiny_target_db


def test_artifact_checksum_verification_accepts_and_rejects_bytes(tmp_path: Path):
    artifact = tmp_path / "artifact.sqlite"
    artifact.write_bytes(b"deterministic artifact bytes")
    digest = hash_file(artifact)

    assert verify_sha256(artifact, digest) == digest

    with pytest.raises(ArtifactChecksumError, match="SHA-256 mismatch"):
        verify_sha256(artifact, "0" * 64)


def test_public_manifest_points_to_public_artifact_and_hides_source_metadata():
    manifest_path = Path(__file__).resolve().parents[1] / "data" / "manifest.example.json"
    manifest = json.loads(manifest_path.read_text())

    artifact = manifest["artifacts"]["target_sqlite_zst"]

    assert "source" not in manifest
    assert artifact["url"].startswith(
        "https://flowai-public-data.hel1.your-objectstorage.com/inventory-scenario/"
    )
    assert artifact["url"].endswith(".target.sqlite.zst")
    assert len(artifact["sha256"]) == 64


def test_default_manifest_path_points_to_example_data_directory():
    manifest_path = _default_manifest_path()

    assert manifest_path == Path(__file__).resolve().parents[1] / "data" / "manifest.example.json"
    assert manifest_path.exists()


def test_seed_rejects_in_repo_builtin_artifact_generation(tmp_path: Path):
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        json.dumps(
            {
                "dataset_version": "test",
                "artifacts": {
                    "target_sqlite": {
                        "url": "builtin://inventory-scenario.target.sqlite",
                        "sha256": "0" * 64,
                    }
                },
                "tables": {},
            }
        )
    )

    with pytest.raises(SeedError, match="unsupported artifact URL scheme"):
        run_seed(SeedOptions(manifest_path=manifest_path, data_root=tmp_path / "data"))


def test_seed_is_idempotent_and_creates_local_state(tmp_path: Path):
    artifact = tmp_path / "inventory-scenario-test.sqlite"
    artifact_manifest = create_tiny_target_db(artifact)
    digest = hash_file(artifact)
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        json.dumps(
            {
                **artifact_manifest,
                "artifacts": {
                    "target_sqlite": {
                        "url": artifact.as_uri(),
                        "sha256": digest,
                        "uncompressed_bytes": artifact.stat().st_size,
                    }
                },
            },
            indent=2,
        )
    )
    data_root = tmp_path / "local-data"

    first = run_seed(
        SeedOptions(
            manifest_path=manifest_path,
            data_root=data_root,
        )
    )
    second = run_seed(
        SeedOptions(
            manifest_path=manifest_path,
            data_root=data_root,
        )
    )

    assert first.target_db == second.target_db
    assert first.row_counts == second.row_counts
    assert (data_root / "target.db").exists()
    assert (data_root / "platform.db").exists()
    assert not (data_root / "catalog.db").exists()
    assert not (data_root / "kv.db").exists()
    assert not (data_root / "catalog-index").exists()

    with sqlite3.connect(data_root / "platform.db") as conn:
        product_count = int(conn.execute("SELECT count(*) FROM products").fetchone()[0])

    assert product_count >= 3


def test_seed_reset_removes_stale_local_state(tmp_path: Path):
    artifact = tmp_path / "inventory-scenario-test.sqlite"
    artifact_manifest = create_tiny_target_db(artifact)
    digest = hash_file(artifact)
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        json.dumps(
            {
                **artifact_manifest,
                "artifacts": {
                    "target_sqlite": {
                        "url": artifact.as_uri(),
                        "sha256": digest,
                        "uncompressed_bytes": artifact.stat().st_size,
                    }
                },
            },
            indent=2,
        )
    )
    data_root = tmp_path / "local-data"

    run_seed(
        SeedOptions(
            manifest_path=manifest_path,
            data_root=data_root,
        )
    )
    stale_catalog = data_root / "catalog.db"
    stale_catalog.write_text("flow ai catalog state")
    stale_kv = data_root / "kv.db"
    stale_kv.write_text("flow ai kv state")
    stale_index_file = data_root / "catalog-index" / "stale.txt"
    stale_index_file.parent.mkdir(exist_ok=True)
    stale_index_file.write_text("stale")

    run_seed(
        SeedOptions(
            manifest_path=manifest_path,
            data_root=data_root,
            reset=True,
        )
    )

    assert (data_root / "target.db").exists()
    assert (data_root / "platform.db").exists()
    assert stale_catalog.read_text() == "flow ai catalog state"
    assert stale_kv.read_text() == "flow ai kv state"
    assert stale_index_file.read_text() == "stale"


def test_seed_logs_progress_for_cli_runs(tmp_path: Path, caplog: pytest.LogCaptureFixture):
    artifact = tmp_path / "inventory-scenario-test.sqlite"
    artifact_manifest = create_tiny_target_db(artifact)
    digest = hash_file(artifact)
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(
        json.dumps(
            {
                **artifact_manifest,
                "artifacts": {
                    "target_sqlite": {
                        "url": artifact.as_uri(),
                        "sha256": digest,
                        "uncompressed_bytes": artifact.stat().st_size,
                    }
                },
            },
            indent=2,
        )
    )
    data_root = tmp_path / "local-data"

    with caplog.at_level(logging.INFO, logger="inventory_scenario.support.seed"):
        run_seed(
            SeedOptions(
                manifest_path=manifest_path,
                data_root=data_root,
            )
        )

    messages = [record.getMessage() for record in caplog.records]
    assert any("Loading manifest" in message for message in messages)
    assert any("Fetching artifact" in message for message in messages)
    assert any("Verifying artifact checksum" in message for message in messages)
    assert any("Materializing target database" in message for message in messages)
    assert any("Seeding mock platform" in message for message in messages)
    assert any("Seed complete" in message for message in messages)
    assert not any("Seeding catalog" in message for message in messages)
    assert not any("Ingesting knowledge documents" in message for message in messages)
    assert not any("Rebuilding catalog index" in message for message in messages)
