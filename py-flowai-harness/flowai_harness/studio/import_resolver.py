from __future__ import annotations

import importlib
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from flowai_harness.studio.app import FlowAIApp


@dataclass(frozen=True)
class AppImportError(Exception):
    """Structured import failure for `--app package.module:symbol`."""

    reference: str
    module: str | None
    symbol: str | None
    code: str
    message: str

    def __str__(self) -> str:
        return self.message

    def to_error_response(self) -> dict[str, Any]:
        return {
            "error": {
                "code": self.code,
                "message": self.message,
                "retryable": False,
                "details": {
                    "reference": self.reference,
                    "module": self.module,
                    "symbol": self.symbol,
                },
            }
        }


def resolve_app_reference(reference: str) -> FlowAIApp:
    """Import a `FlowAIApp` from a `package.module:symbol` reference."""

    module_name, symbol = _split_reference(reference)
    cwd = str(Path.cwd())
    if cwd not in sys.path:
        sys.path.insert(0, cwd)
    try:
        module = importlib.import_module(module_name)
    except Exception as exc:  # noqa: BLE001 - diagnostics must preserve import failure.
        raise AppImportError(
            reference=reference,
            module=module_name,
            symbol=symbol,
            code="app_import.module_failed",
            message=f"Failed to import Studio app module {module_name!r}: {exc}",
        ) from exc

    try:
        value = getattr(module, symbol)
    except AttributeError as exc:
        raise AppImportError(
            reference=reference,
            module=module_name,
            symbol=symbol,
            code="app_import.symbol_missing",
            message=f"Studio app module {module_name!r} has no symbol {symbol!r}.",
        ) from exc

    if callable(value) and not isinstance(value, FlowAIApp):
        value = value()
    if not isinstance(value, FlowAIApp):
        raise AppImportError(
            reference=reference,
            module=module_name,
            symbol=symbol,
            code="app_import.invalid_type",
            message=(
                f"Studio app symbol {module_name}:{symbol} must be a FlowAIApp "
                f"or zero-argument factory returning FlowAIApp."
            ),
        )
    return value


def _split_reference(reference: str) -> tuple[str, str]:
    if not isinstance(reference, str) or ":" not in reference:
        raise AppImportError(
            reference=str(reference),
            module=None,
            symbol=None,
            code="app_import.invalid_reference",
            message="Studio app reference must use 'package.module:symbol'.",
        )
    module_name, symbol = reference.split(":", 1)
    if not module_name or not symbol:
        raise AppImportError(
            reference=reference,
            module=module_name or None,
            symbol=symbol or None,
            code="app_import.invalid_reference",
            message="Studio app reference must include both module and symbol.",
        )
    return module_name, symbol
