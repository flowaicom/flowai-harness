from __future__ import annotations

import argparse
import asyncio
import importlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
from typing import Any

from flowai_harness import _internal
from flowai_harness import mcp
from flowai_harness.docs_export import (
    DocsExportError,
    ExportOptions,
    export_fumadocs_docs,
)
from flowai_harness.studio import AppImportError, resolve_app_reference, run_studio_server


class StudioFrontendError(RuntimeError):
    """Raised when the local Studio frontend dev server cannot be launched."""


def main(argv: list[str] | None = None) -> int:
    args = list(argv if argv is not None else sys.argv)
    if len(args) > 1 and args[1] in {"dev", "serve"}:
        return _run_studio_command(args)
    if len(args) >= 3 and args[1] == "docs" and args[2] == "export":
        return _run_docs_export_command(args[0], args[3:])
    if len(args) >= 3 and args[1] == "mcp" and args[2] == "python":
        return _run_mcp_python(args[0], args[3:])
    return int(_internal.run_cli(args))


def _run_docs_export_command(program: str, args: list[str]) -> int:
    default_root = Path.cwd()
    parser = argparse.ArgumentParser(
        prog=f"{program} docs export",
        description="Export FlowAI Harness docs as a Fumadocs-compatible content artifact.",
    )
    parser.add_argument(
        "--format",
        choices=("fumadocs",),
        default="fumadocs",
        help="Docs artifact format to generate",
    )
    parser.add_argument(
        "--source-dir",
        type=Path,
        default=default_root / "docs",
        help="Source docs directory containing MkDocs markdown",
    )
    parser.add_argument(
        "--mkdocs-config",
        type=Path,
        default=default_root / "mkdocs.yml",
        help="MkDocs config used as the nav source of truth",
    )
    parser.add_argument(
        "--out-dir",
        type=Path,
        default=default_root / "dist" / "fumadocs" / "content" / "docs",
        help="Output directory for generated Fumadocs content",
    )
    parser.add_argument(
        "--no-clean",
        action="store_true",
        help="Do not delete the output directory before writing generated docs",
    )

    try:
        options = parser.parse_args(args)
    except SystemExit as exc:
        return int(exc.code or 0)

    try:
        result = export_fumadocs_docs(
            ExportOptions(
                source_dir=options.source_dir,
                mkdocs_config=options.mkdocs_config,
                output_dir=options.out_dir,
                clean=not options.no_clean,
            )
        )
    except DocsExportError as exc:
        print(str(exc), file=sys.stderr)
        return 2

    print(
        "Exported "
        f"{result.pages_written} docs pages and {result.assets_written} assets "
        f"to {options.out_dir}",
        file=sys.stderr,
    )
    return 0


def _run_studio_command(argv: list[str]) -> int:
    program = argv[0] if argv else "flowai-harness"
    command = argv[1]
    parser = argparse.ArgumentParser(
        prog=f"{program} {command}",
        description="Run the local FlowAI Harness Studio adapter.",
    )
    parser.add_argument("--app", required=True, help="Studio app reference: package.module:symbol")
    parser.add_argument("--host", default="127.0.0.1", help="Host to bind")
    parser.add_argument("--port", type=int, default=4111, help="Port to bind")
    parser.add_argument(
        "--no-studio",
        action="store_true",
        help="Serve API routes only and do not launch or serve Studio UI assets",
    )
    if command == "dev":
        parser.add_argument(
            "--no-frontend",
            action="store_true",
            help="Do not launch the React Studio source frontend dev server.",
        )
        parser.add_argument(
            "--frontend-host",
            help="Host for the React Studio source frontend dev server. Requires --studio-dir.",
        )
        parser.add_argument(
            "--frontend-port",
            type=int,
            help="Port for the React Studio source frontend dev server. Requires --studio-dir.",
        )
        parser.add_argument(
            "--studio-dir",
            help=(
                "Path to the Studio frontend source directory containing package.json. "
                "When provided, dev starts Bun/Vite on a separate frontend port."
            ),
        )

    try:
        options = parser.parse_args(argv[2:])
        _validate_studio_command_options(parser, command, options)
        app = resolve_app_reference(options.app)
    except AppImportError as exc:
        print(json.dumps(exc.to_error_response(), sort_keys=True), file=sys.stderr)
        return 2
    except SystemExit as exc:
        return int(exc.code or 0)

    frontend_process = None
    if _should_start_frontend_dev_server(command, options):
        _apply_frontend_defaults(options)
        try:
            frontend_process = _start_frontend_dev_server(options)
        except StudioFrontendError as exc:
            print(str(exc), file=sys.stderr)
            return 2

    try:
        run_studio_server(
            app,
            host=options.host,
            port=options.port,
            serve_studio=not options.no_studio,
        )
    finally:
        if frontend_process is not None:
            _terminate_frontend_dev_server(frontend_process)
    return 0


def _validate_studio_command_options(
    parser: argparse.ArgumentParser,
    command: str,
    options: argparse.Namespace,
) -> None:
    if command != "dev":
        return

    source_frontend_options = [
        option
        for option in ("studio_dir", "frontend_host", "frontend_port")
        if getattr(options, option, None) is not None
    ]
    if options.no_studio and (source_frontend_options or options.no_frontend):
        parser.error("--no-studio cannot be combined with source frontend options")

    if options.no_frontend and source_frontend_options:
        parser.error("--no-frontend cannot be combined with source frontend options")

    if (options.frontend_host is not None or options.frontend_port is not None) and not options.studio_dir:
        parser.error("--frontend-host and --frontend-port require --studio-dir")


def _should_start_frontend_dev_server(command: str, options: argparse.Namespace) -> bool:
    return (
        command == "dev"
        and bool(getattr(options, "studio_dir", None))
        and not options.no_frontend
        and not options.no_studio
    )


def _apply_frontend_defaults(options: argparse.Namespace) -> None:
    if options.frontend_host is None:
        options.frontend_host = "127.0.0.1"
    if options.frontend_port is None:
        options.frontend_port = 3000


def _run_mcp_python(program: str, args: list[str]) -> int:
    parser = argparse.ArgumentParser(
        prog=f"{program} mcp python",
        description="Serve a Python Flow AI runtime object as an MCP server.",
    )
    parser.add_argument("target", metavar="MODULE:OBJECT")
    parser.add_argument("--agent", required=True)
    parser.add_argument(
        "--transport",
        choices=("stdio", "streamable-http"),
        default="stdio",
    )
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--path", default="/mcp")
    parser.add_argument("--allow-origin", action="append", dest="allowed_origins")
    parser.add_argument("--no-origin-check", action="store_true")
    parser.add_argument(
        "--auth-token",
        help="Required auth token for streamable-http. Can also be set with FLOWAI_MCP_HTTP_TOKEN.",
    )
    parser.add_argument("--thread-id")
    parser.add_argument("--call-timeout-secs", type=float, default=30.0)
    parser.add_argument(
        "--expose-agent-tools",
        action="store_true",
        help="Reserved for future recursive agent tools; the agents toolkit is not supported in this mode.",
    )
    parsed = parser.parse_args(args)
    auth_token = parsed.auth_token or os.environ.get("FLOWAI_MCP_HTTP_TOKEN")
    if parsed.transport == "streamable-http" and (auth_token is None or auth_token.strip() == ""):
        parser.error("streamable-http requires --auth-token or FLOWAI_MCP_HTTP_TOKEN")

    runtime = _load_runtime_target(parsed.target)
    if parsed.transport == "stdio":
        _require_method(runtime, "serve_mcp_stdio")
        asyncio.run(
            mcp.serve_stdio(
                runtime,
                agent=parsed.agent,
                thread_id=parsed.thread_id,
                call_timeout_secs=parsed.call_timeout_secs,
                expose_agent_tools=parsed.expose_agent_tools,
            )
        )
    else:
        _require_method(runtime, "serve_mcp_http")
        asyncio.run(
            mcp.serve_http(
                runtime,
                agent=parsed.agent,
                host=parsed.host,
                port=parsed.port,
                path=parsed.path,
                transport="streamable-http",
                thread_id=parsed.thread_id,
                call_timeout_secs=parsed.call_timeout_secs,
                expose_agent_tools=parsed.expose_agent_tools,
                allowed_origins=parsed.allowed_origins,
                require_origin=not parsed.no_origin_check,
                auth_token=auth_token,
            )
        )
    return 0


def _load_runtime_target(target: str) -> Any:
    if ":" not in target:
        raise ValueError("Python MCP target must have the form MODULE:OBJECT")
    module_name, object_path = target.split(":", 1)
    if not module_name or not object_path:
        raise ValueError("Python MCP target must have the form MODULE:OBJECT")
    module = importlib.import_module(module_name)
    obj: Any = module
    for part in object_path.split("."):
        obj = getattr(obj, part)
    return obj() if callable(obj) else obj


def _require_method(runtime: Any, name: str) -> None:
    if not callable(getattr(runtime, name, None)):
        raise TypeError(f"Python MCP runtime object must provide {name}()")


def _start_frontend_dev_server(options: argparse.Namespace) -> subprocess.Popen[bytes]:
    studio_dir = _resolve_studio_dir(options.studio_dir)
    bun = shutil.which("bun")
    if bun is None:
        raise StudioFrontendError(
            "Unable to launch Studio frontend: `bun` was not found on PATH. "
            "Install bun or rerun with --no-frontend."
        )

    backend_url = f"http://{_local_url_host(options.host)}:{options.port}"
    frontend_url = f"http://{_local_url_host(options.frontend_host)}:{options.frontend_port}"
    env = os.environ.copy()
    env["FLOWAI_STUDIO_BACKEND_URL"] = backend_url

    cmd = [
        bun,
        "run",
        "dev",
        "--",
        "--host",
        options.frontend_host,
        "--port",
        str(options.frontend_port),
    ]
    print(f"Starting FlowAI Studio frontend at {frontend_url}", file=sys.stderr)
    print(f"Proxying Studio API requests to {backend_url}", file=sys.stderr)
    process = subprocess.Popen(
        cmd,
        cwd=studio_dir,
        env=env,
        start_new_session=os.name != "nt",
    )
    time.sleep(0.25)
    if process.poll() is not None:
        raise StudioFrontendError(
            "Studio frontend dev server exited during startup. "
            f"Run `cd {studio_dir} && bun run dev` for detailed diagnostics."
        )
    return process


def _resolve_studio_dir(studio_dir: str | None) -> Path:
    if studio_dir:
        candidate = Path(studio_dir).expanduser().resolve()
        if (candidate / "package.json").exists():
            return candidate
        raise StudioFrontendError(
            f"Unable to launch Studio frontend: {candidate} does not contain package.json."
        )

    search_roots = [Path(__file__).resolve(), Path.cwd().resolve()]
    for root in search_roots:
        for parent in root.parents if root.is_file() else (root, *root.parents):
            candidate = parent / "studio"
            if (candidate / "package.json").exists():
                return candidate

    raise StudioFrontendError(
        "Unable to locate Studio frontend directory. Pass --studio-dir or rerun with --no-frontend."
    )


def _local_url_host(host: str) -> str:
    if host in {"0.0.0.0", "::", "[::]"}:
        return "127.0.0.1"
    return host


def _terminate_frontend_dev_server(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    try:
        if os.name != "nt":
            os.killpg(process.pid, signal.SIGTERM)
        else:
            process.terminate()
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            if os.name != "nt":
                os.killpg(process.pid, signal.SIGKILL)
            else:
                process.kill()
        except ProcessLookupError:
            return
        process.wait(timeout=5)


if __name__ == "__main__":
    raise SystemExit(main())
