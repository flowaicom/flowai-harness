from __future__ import annotations

from importlib.metadata import PackageNotFoundError, version

PACKAGE_NAME = "flowai-harness"
UNKNOWN_VERSION = "0+unknown"


def package_version() -> str:
    try:
        return version(PACKAGE_NAME)
    except PackageNotFoundError:
        return UNKNOWN_VERSION


__version__ = package_version()
