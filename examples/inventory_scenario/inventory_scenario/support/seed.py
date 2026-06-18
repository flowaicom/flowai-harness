from __future__ import annotations

import argparse
import hashlib
import json
import logging
import shutil
import sqlite3
import subprocess
import sys
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from inventory_scenario.support.data_environment import default_data_root
from inventory_scenario.support.mock_platform.store import seed_platform_db


LOGGER = logging.getLogger(__name__)


class ArtifactChecksumError(RuntimeError):
    """Raised when a downloaded artifact does not match the manifest digest."""


class SeedError(RuntimeError):
    """Raised for invalid seed options or local setup failures."""


@dataclass(frozen=True)
class SeedOptions:
    manifest_path: Path | None = None
    data_root: Path = default_data_root()
    reset: bool = False


@dataclass(frozen=True)
class SeedResult:
    data_root: Path
    target_db: Path
    platform_db: Path
    row_counts: dict[str, int]
    manifest_path: Path


def run_seed(options: SeedOptions | None = None) -> SeedResult:
    options = options or SeedOptions()

    manifest_path = options.manifest_path or _default_manifest_path()
    LOGGER.info("Loading manifest: %s", manifest_path)
    manifest = _load_manifest(manifest_path)
    data_root = options.data_root
    LOGGER.info("Using data root: %s", data_root)
    if options.reset:
        LOGGER.info("Resetting generated local state under: %s", data_root)
        _reset_local_state(data_root)
    data_root.mkdir(parents=True, exist_ok=True)

    artifact_path = _ensure_artifact(manifest, data_root)
    target_db = data_root / "target.db"
    LOGGER.info("Materializing target database: %s", target_db)
    _materialize_target_database(artifact_path, target_db)
    if not _validate_row_counts(target_db, manifest):
        raise SeedError("target row counts do not match manifest")
    LOGGER.info(
        "Validated target row counts for %d tables",
        len(_manifest_row_counts(manifest)),
    )

    platform_db = data_root / "platform.db"

    LOGGER.info("Seeding mock platform: %s", platform_db)
    seed_platform_db(target_db, platform_db)
    LOGGER.info(
        "Seed complete: target=%s platform=%s",
        target_db,
        platform_db,
    )

    return SeedResult(
        data_root=data_root,
        target_db=target_db,
        platform_db=platform_db,
        row_counts=_manifest_row_counts(manifest),
        manifest_path=manifest_path,
    )


def hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def verify_sha256(path: Path, expected: str) -> str:
    actual = hash_file(path)
    if actual.lower() != expected.lower():
        raise ArtifactChecksumError(
            f"SHA-256 mismatch for {path}: expected {expected}, got {actual}"
        )
    return actual


def main(argv: list[str] | None = None) -> int:
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
    parser = argparse.ArgumentParser(
        description="Seed the local inventory scenario SQLite environment.",
    )
    parser.add_argument("--manifest", type=Path, dest="manifest_path")
    parser.add_argument("--data-root", type=Path, default=default_data_root())
    parser.add_argument(
        "--reset",
        action="store_true",
        help="Remove generated target/platform/artifact files before seeding.",
    )
    args = parser.parse_args(argv)

    try:
        result = run_seed(
            SeedOptions(
                manifest_path=args.manifest_path,
                data_root=args.data_root,
                reset=args.reset,
            )
        )
    except (ArtifactChecksumError, SeedError) as exc:
        print(str(exc), file=sys.stderr)
        return 2

    print(
        json.dumps(
            {
                "data_root": str(result.data_root),
                "target_db": str(result.target_db),
                "platform_db": str(result.platform_db),
                "row_counts": result.row_counts,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


def _ensure_artifact(manifest: dict[str, Any], data_root: Path) -> Path:
    artifact = _target_artifact(manifest)
    url = artifact["url"]
    artifact_dir = data_root / "artifacts"
    artifact_dir.mkdir(parents=True, exist_ok=True)
    parsed_url = urllib.parse.urlparse(url)
    filename = Path(parsed_url.path).name or parsed_url.netloc or "target.sqlite"
    local_path = artifact_dir / filename

    if parsed_url.scheme not in {"https", "http", "file"}:
        raise SeedError(
            f"unsupported artifact URL scheme '{parsed_url.scheme}'; use https://, http://, or file://"
        )

    LOGGER.info("Fetching artifact: %s", url)
    if not local_path.exists():
        _download(url, local_path)
    else:
        LOGGER.info("Using cached artifact: %s", local_path)
    LOGGER.info("Verifying artifact checksum: %s", local_path)
    verify_sha256(local_path, artifact["sha256"])
    LOGGER.info("Artifact checksum ok: %s", local_path)
    return local_path


def _reset_local_state(data_root: Path) -> None:
    for name in (
        "target.db",
        "platform.db",
        "artifacts",
    ):
        path = data_root / name
        if path.is_dir():
            shutil.rmtree(path)
        elif path.exists():
            path.unlink()


def _materialize_target_database(artifact_path: Path, target_db: Path) -> None:
    target_db.parent.mkdir(parents=True, exist_ok=True)
    if artifact_path.suffix == ".zst":
        LOGGER.info("Decompressing zstd artifact: %s", artifact_path)
        _decompress_zst(artifact_path, target_db)
    else:
        LOGGER.info("Copying SQLite artifact: %s", artifact_path)
        shutil.copyfile(artifact_path, target_db)


def _decompress_zst(artifact_path: Path, target_db: Path) -> None:
    try:
        import zstandard as zstd  # type: ignore[import-not-found]
    except ImportError:
        zstd = None

    if zstd is not None:
        with artifact_path.open("rb") as source, target_db.open("wb") as target:
            zstd.ZstdDecompressor().copy_stream(source, target)
        return

    zstd_bin = shutil.which("zstd")
    if zstd_bin is None:
        raise SeedError(
            "zstd artifact requires either the `zstandard` Python package or the `zstd` CLI"
        )
    subprocess.run(
        [zstd_bin, "-d", "-f", str(artifact_path), "-o", str(target_db)],
        check=True,
    )


def _download(url: str, local_path: Path) -> None:
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme == "file":
        LOGGER.info("Copying artifact from file URL to cache: %s -> %s", url, local_path)
        shutil.copyfile(Path(urllib.request.url2pathname(parsed.path)), local_path)
        return
    LOGGER.info(
        "Downloading artifact from S3-compatible object storage: %s -> %s",
        url,
        local_path,
    )
    with urllib.request.urlopen(url, timeout=60) as response:
        with local_path.open("wb") as handle:
            shutil.copyfileobj(response, handle)
    LOGGER.info("Downloaded artifact cache: %s", local_path)


def _target_artifact(manifest: dict[str, Any]) -> dict[str, Any]:
    artifacts = manifest.get("artifacts", {})
    for key in ("target_sqlite_zst", "target_sqlite"):
        artifact = artifacts.get(key)
        if isinstance(artifact, dict) and artifact.get("url"):
            return artifact
    raise SeedError("manifest must define artifacts.target_sqlite_zst or target_sqlite")


def _load_manifest(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def _manifest_row_counts(manifest: dict[str, Any]) -> dict[str, int]:
    return {
        table: int(value["rows"])
        for table, value in manifest.get("tables", {}).items()
    }


def _validate_row_counts(db_path: Path, manifest: dict[str, Any]) -> bool:
    for table, expected in manifest.get("tables", {}).items():
        actual = _table_row_count(db_path, table)
        if actual != expected.get("rows"):
            return False
    return True


def _table_row_count(db_path: Path, table: str) -> int:
    with sqlite3.connect(db_path) as conn:
        return int(conn.execute(f"SELECT count(*) FROM {table}").fetchone()[0])


def _default_manifest_path() -> Path:
    return Path(__file__).resolve().parents[2] / "data" / "manifest.example.json"


if __name__ == "__main__":
    raise SystemExit(main())
