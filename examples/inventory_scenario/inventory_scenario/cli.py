from __future__ import annotations

import argparse
import sys
from collections.abc import Callable

from inventory_scenario.support import data_environment, seed, smoke
from inventory_scenario.support.dataset_artifacts import dump_neon, verify_public_artifact


Command = Callable[[list[str] | None], int]


COMMANDS: dict[str, Command] = {
    "seed": seed.main,
    "data-env": data_environment.main,
    "data-environment": data_environment.main,
    "smoke": smoke.main,
    "verify-artifact": verify_public_artifact.main,
    "dump-neon": dump_neon.main,
}


def main(argv: list[str] | None = None) -> int:
    args = list(sys.argv[1:] if argv is None else argv)
    if not args or args[0] in {"-h", "--help"}:
        _print_help()
        return 0

    command, *rest = args
    if command == "platform":
        return _run_platform(rest)
    try:
        handler = COMMANDS[command]
    except KeyError:
        print(f"unknown command: {command}", file=sys.stderr)
        _print_help(file=sys.stderr)
        return 2
    return handler(rest)


def _run_platform(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        prog="inventory-scenario platform",
        description="Run the optional FastAPI mock inventory platform.",
    )
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8123)
    args = parser.parse_args(argv)

    import uvicorn

    from inventory_scenario.support.mock_platform.api import create_default_app

    uvicorn.run(create_default_app(), host=args.host, port=args.port)
    return 0


def _print_help(*, file=sys.stdout) -> None:
    print(
        """usage: inventory-scenario <command> [args]

Commands:
  seed              Materialize the local target database and mock platform.
  data-env          Write the Flow AI data-environment descriptor.
  smoke             Run deterministic local smoke verification.
  platform          Run the optional FastAPI mock platform UI/API.
  verify-artifact   Validate the public artifact manifest.
  dump-neon         Maintainer-only dataset artifact generation flow.
""",
        file=file,
    )


if __name__ == "__main__":
    raise SystemExit(main())
