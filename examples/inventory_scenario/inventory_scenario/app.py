from __future__ import annotations

from flowai_harness import define_app

from inventory_scenario.runtime import build_runtime, build_runtime_spec
from inventory_scenario.support.data_environment import (
    build_data_environment,
    default_data_root,
)


def runtime():
    """Studio/MCP import target for the prepared local inventory scenario."""

    data_environment = build_data_environment(default_data_root())
    return define_app(
        name="inventory-scenario",
        description="Inventory scenario planning example with local catalog and mock platform.",
        runtime_spec=build_runtime_spec(),
        runtime_factory=lambda: build_runtime(data_environment=data_environment),
        data_environment=data_environment,
    )
