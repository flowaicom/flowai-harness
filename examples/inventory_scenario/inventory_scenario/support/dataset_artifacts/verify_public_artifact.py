from __future__ import annotations

import argparse
import json
import string
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any


class PublicArtifactError(RuntimeError):
    """Raised when the public artifact manifest is not release-ready."""


@dataclass(frozen=True)
class PublicArtifactMetadata:
    url: str
    sha256: str
    uncompressed_bytes: int


def validate_manifest_release_metadata(
    manifest: dict[str, Any],
) -> PublicArtifactMetadata:
    artifacts = manifest.get("artifacts", {})
    artifact = artifacts.get("target_sqlite_zst")
    if not isinstance(artifact, dict):
        raise PublicArtifactError("manifest must define artifacts.target_sqlite_zst")

    url = str(artifact.get("url", ""))
    sha256 = str(artifact.get("sha256", ""))
    uncompressed_bytes = int(artifact.get("uncompressed_bytes", 0) or 0)

    if not url.startswith(("https://", "http://")):
        raise PublicArtifactError("artifact URL must use a public HTTP(S) location")
    if "..." in url or "example" in url:
        raise PublicArtifactError("artifact URL still looks like a placeholder")
    if not url.endswith(".target.sqlite.zst"):
        raise PublicArtifactError("artifact URL must point at a .target.sqlite.zst file")
    if len(sha256) != 64 or any(char not in string.hexdigits for char in sha256):
        raise PublicArtifactError("artifact SHA-256 must be a 64-character hex digest")
    if sha256 == "0" * 64:
        raise PublicArtifactError("artifact manifest still contains a placeholder SHA-256")
    if uncompressed_bytes <= 0:
        raise PublicArtifactError(
            "artifact manifest must include a positive uncompressed byte size"
        )

    return PublicArtifactMetadata(
        url=url,
        sha256=sha256.lower(),
        uncompressed_bytes=uncompressed_bytes,
    )


def check_artifact_url(url: str, *, timeout: int = 30) -> None:
    request = urllib.request.Request(url, method="HEAD")
    try:
        with urllib.request.urlopen(request, timeout=timeout):
            return
    except urllib.error.HTTPError as exc:
        if exc.code == 405:
            _check_artifact_url_with_get(url, timeout=timeout)
            return
        raise PublicArtifactError(
            f"artifact URL returned HTTP {exc.code}: {url}"
        ) from exc
    except OSError as exc:
        raise PublicArtifactError(f"artifact URL check failed: {exc}") from exc


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Verify that the public inventory scenario artifact is published.",
    )
    parser.add_argument("--manifest", type=Path, default=_default_manifest_path())
    parser.add_argument(
        "--check-url",
        action="store_true",
        help="Also verify the manifest artifact URL is reachable.",
    )
    args = parser.parse_args(argv)

    try:
        metadata = validate_manifest_release_metadata(
            json.loads(args.manifest.read_text())
        )
        if args.check_url:
            check_artifact_url(metadata.url)
    except PublicArtifactError as exc:
        print(str(exc), file=sys.stderr)
        return 2

    print(
        json.dumps(
            {
                "url": metadata.url,
                "sha256": metadata.sha256,
                "uncompressed_bytes": metadata.uncompressed_bytes,
                "url_check": "ok" if args.check_url else "skipped",
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


def _check_artifact_url_with_get(url: str, *, timeout: int) -> None:
    request = urllib.request.Request(url, headers={"Range": "bytes=0-0"})
    try:
        with urllib.request.urlopen(request, timeout=timeout):
            return
    except urllib.error.HTTPError as exc:
        raise PublicArtifactError(
            f"artifact URL returned HTTP {exc.code}: {url}"
        ) from exc
    except OSError as exc:
        raise PublicArtifactError(f"artifact URL check failed: {exc}") from exc


def _default_manifest_path() -> Path:
    return Path(__file__).resolve().parents[3] / "data" / "manifest.example.json"


if __name__ == "__main__":
    raise SystemExit(main())
