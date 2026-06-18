from __future__ import annotations

from collections.abc import Callable
from typing import Any, TypeVar

from flowai_harness import _internal

EXPECTED_NATIVE_API_VERSION = 4

_T = TypeVar("_T")


def assert_native_api_version() -> None:
    native_version = getattr(_internal, "native_api_version", None)
    if not callable(native_version):
        raise _stale_extension_error("native API version function is missing")

    actual = native_version()
    if actual != EXPECTED_NATIVE_API_VERSION:
        raise _stale_extension_error(
            f"expected native API version {EXPECTED_NATIVE_API_VERSION}, got {actual}"
        )


def call_native(function: Callable[..., _T], *args: Any, **kwargs: Any) -> _T:
    assert_native_api_version()
    try:
        return function(*args, **kwargs)
    except ValueError as error:
        if _looks_like_schema_drift_error(str(error)):
            raise _stale_extension_error(str(error)) from error
        raise


def _looks_like_schema_drift_error(message: str) -> bool:
    return "unknown field" in message


def _stale_extension_error(raw_detail: str) -> RuntimeError:
    return RuntimeError(
        "flowai_harness._internal is stale or schema-incompatible with the "
        "Python facade; rebuild/reinstall the extension. "
        f"Raw native detail: {raw_detail}"
    )
