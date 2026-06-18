"""Local mock inventory platform for the inventory scenario example."""

from inventory_scenario.support.mock_platform.api import create_app
from inventory_scenario.support.mock_platform.store import PlatformStore, seed_platform_db

__all__ = ["PlatformStore", "create_app", "seed_platform_db"]
