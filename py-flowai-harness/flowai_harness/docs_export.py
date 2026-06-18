from __future__ import annotations

from dataclasses import dataclass
import html
import importlib
import inspect
import json
from pathlib import Path, PurePosixPath
import posixpath
import re
import shutil
import textwrap
from types import UnionType
from typing import Any, Literal, get_args, get_origin

from pydantic import BaseModel

@dataclass(frozen=True)
class ExportOptions:
    source_dir: Path
    mkdocs_config: Path
    output_dir: Path
    clean: bool = True
    base_url: str = "/docs"


@dataclass(frozen=True)
class ExportResult:
    pages_written: int
    assets_written: int


@dataclass(frozen=True)
class NavNode:
    title: str
    path: str | None = None
    children: tuple["NavNode", ...] = ()


class DocsExportError(RuntimeError):
    """Raised when docs cannot be exported to a website artifact."""


# MkDocs-only theme assets that must not leak into the website artifact.
_MKDOCS_ONLY_ASSET_PREFIXES = ("stylesheets/",)

_REFERENCE_TYPE_TARGETS: dict[str, str] = {
    "ActionPayloadMatchMode": "/docs/reference/evals#actionpayloadmatchmode",
    "AgentSpec": "/docs/reference/runtime#flowai_harness.runtime.AgentSpec",
    "AggregationStrategy": "/docs/reference/evals#aggregationstrategy",
    "ApprovalOverrides": "/docs/reference/runtime#flowai_harness.runtime.ApprovalOverrides",
    "ApprovalPolicies": "/docs/reference/runtime#flowai_harness.runtime.ApprovalPolicies",
    "ApprovalPolicyPatch": "/docs/reference/runtime#flowai_harness.runtime.ApprovalPolicyPatch",
    "ArtifactMetadata": "/docs/reference/evals#flowai_harness.evals.ArtifactMetadata",
    "CostAgentBreakdown": "/docs/reference/evals#flowai_harness.evals.CostAgentBreakdown",
    "DataEnvironmentConfig": "/docs/reference/runtime#flowai_harness.runtime.DataEnvironmentConfig",
    "EvalArtifact": "/docs/reference/evals#flowai_harness.evals.EvalArtifact",
    "EvalArtifactSummary": "/docs/reference/evals#flowai_harness.evals.EvalArtifactSummary",
    "EvalConfig": "/docs/reference/evals#flowai_harness.evals.EvalConfig",
    "EvalMode": "/docs/reference/evals#evalmode",
    "EvalRequest": "/docs/reference/evals#flowai_harness.evals.EvalRequest",
    "EvalTestCase": "/docs/reference/evals#flowai_harness.evals.EvalTestCase",
    "ExpectedAction": "/docs/reference/evals#flowai_harness.evals.ExpectedAction",
    "FinalResponseEval": "/docs/reference/evals#flowai_harness.evals.FinalResponseEval",
    "FinalResponseScorerConfig": "/docs/reference/evals#flowai_harness.evals.FinalResponseScorerConfig",
    "FlowAIApp": "/docs/reference/studio#flowai_harness.studio.FlowAIApp",
    "GroundTruth": "/docs/reference/evals#flowai_harness.evals.GroundTruth",
    "HarnessEvalEventEnvelope": "/docs/reference/evals#flowai_harness.evals.HarnessEvalEventEnvelope",
    "JudgeVerdict": "/docs/reference/evals#flowai_harness.evals.JudgeVerdict",
    "LayeredPrompt": "/docs/reference/prompts#flowai_harness.prompts.LayeredPrompt",
    "ModelInvocation": "/docs/reference/evals#flowai_harness.evals.ModelInvocation",
    "ModelSpec": "/docs/reference/runtime#flowai_harness.runtime.ModelSpec",
    "PassAtKResult": "/docs/reference/evals#flowai_harness.evals.PassAtKResult",
    "PlanDisplayAlias": "/docs/reference/plans#flowai_harness.plans.PlanDisplayAlias",
    "PlanSpec": "/docs/reference/plans#flowai_harness.plans.PlanSpec",
    "RawSampleOutput": "/docs/reference/evals#flowai_harness.evals.RawSampleOutput",
    "ReferenceSpec": "/docs/reference/references#flowai_harness.references.ReferenceSpec",
    "ResolvedAction": "/docs/reference/evals#flowai_harness.evals.ResolvedAction",
    "ResponseScorer": "/docs/reference/evals#flowai_harness.evals.ResponseScorer",
    "ResponseScorerMethod": "/docs/reference/evals#responsescorermethod",
    "Runtime": "/docs/reference/runtime#flowai_harness.runtime.Runtime",
    "RuntimeSpec": "/docs/reference/runtime#flowai_harness.runtime.RuntimeSpec",
    "SampleArtifact": "/docs/reference/evals#flowai_harness.evals.SampleArtifact",
    "SampleCost": "/docs/reference/evals#flowai_harness.evals.SampleCost",
    "SampleLatency": "/docs/reference/evals#flowai_harness.evals.SampleLatency",
    "ScoreWeights": "/docs/reference/evals#flowai_harness.evals.ScoreWeights",
    "ScoredSample": "/docs/reference/evals#flowai_harness.evals.ScoredSample",
    "ScorerName": "/docs/reference/evals#scorername",
    "ScorerPreset": "/docs/reference/evals#flowai_harness.evals.ScorerPreset",
    "ScorerPresetName": "/docs/reference/evals#scorerpresetname",
    "ScorerResult": "/docs/reference/evals#flowai_harness.evals.ScorerResult",
    "StorageFactories": "/docs/reference/runtime#flowai_harness.runtime.StorageFactories",
    "StorageFactorySpec": "/docs/reference/runtime#flowai_harness.runtime.StorageFactorySpec",
    "SummaryCost": "/docs/reference/evals#flowai_harness.evals.SummaryCost",
    "SummaryLatency": "/docs/reference/evals#flowai_harness.evals.SummaryLatency",
    "TaggedUnion": "/docs/reference/unions#flowai_harness.unions.TaggedUnion",
    "TenantIdentity": "/docs/reference/tenant#flowai_harness.tenant.TenantIdentity",
    "TestCaseArtifact": "/docs/reference/evals#flowai_harness.evals.TestCaseArtifact",
    "TestingConfig": "/docs/reference/runtime#flowai_harness.runtime.TestingConfig",
    "TokenUsageSummary": "/docs/reference/evals#flowai_harness.evals.TokenUsageSummary",
    "ToolSpec": "/docs/reference/tools#flowai_harness.tools.ToolSpec",
    "ToolkitSpec": "/docs/reference/runtime#flowai_harness.runtime.ToolkitSpec",
    "TrajectoryMode": "/docs/reference/evals#trajectorymode",
    "TrajectoryScorerConfig": "/docs/reference/evals#flowai_harness.evals.TrajectoryScorerConfig",
    "WorkspaceRuntimeBinding": "/docs/reference/studio#flowai_harness.studio.WorkspaceRuntimeBinding",
}


def export_fumadocs_docs(options: ExportOptions) -> ExportResult:
    source_dir = options.source_dir
    mkdocs_config = options.mkdocs_config
    output_dir = options.output_dir

    if not source_dir.exists():
        raise DocsExportError(f"Docs source directory not found: {source_dir}")
    if not mkdocs_config.exists():
        raise DocsExportError(f"MkDocs config not found: {mkdocs_config}")

    site_name, nav = _load_mkdocs_nav(mkdocs_config)
    if options.clean:
        _clean_output_dir(output_dir, source_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    pages = _walk_files(source_dir, ".md")
    pages_written = 0
    for source_file in pages:
        relative_path = source_file.relative_to(source_dir)
        target_file = output_dir / relative_path.with_suffix(".mdx")
        markdown = source_file.read_text(encoding="utf-8")
        converted = convert_markdown_to_fumadocs(
            markdown,
            relative_path=relative_path,
            base_url=options.base_url,
        )
        target_file.parent.mkdir(parents=True, exist_ok=True)
        target_file.write_text(converted, encoding="utf-8")
        pages_written += 1

    assets_written = 0
    for source_file in _walk_asset_files(source_dir):
        relative_path = source_file.relative_to(source_dir)
        target_file = output_dir / relative_path
        target_file.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source_file, target_file)
        assets_written += 1

    _write_meta_files(output_dir, site_name=site_name, nav=nav)
    _write_manifest(
        output_dir,
        pages=pages,
        assets_written=assets_written,
        source_dir=source_dir,
    )

    return ExportResult(pages_written=pages_written, assets_written=assets_written)


def convert_markdown_to_fumadocs(
    markdown: str,
    *,
    relative_path: Path,
    base_url: str = "/docs",
) -> str:
    converted = markdown.replace("\r\n", "\n")
    _check_unsupported_syntax(converted, relative_path)
    converted = _convert_links(
        converted,
        relative_path=relative_path,
        base_url=base_url,
    )
    converted = _convert_admonitions(converted)
    converted = _convert_card_grids(converted, base_url=base_url)
    converted = _convert_api_directives(converted, base_url=base_url)
    converted = _convert_links(
        converted,
        relative_path=relative_path,
        base_url=base_url,
    )
    converted = _escape_jsx_generics(converted)
    converted = _add_frontmatter(converted, relative_path)
    return converted


def _check_unsupported_syntax(markdown: str, relative_path: Path) -> None:
    """Reject MkDocs syntax that has no Fumadocs conversion.

    These constructs render in MkDocs Material but would pass through the
    export as garbled text on the website, so the export fails loudly instead.
    """
    in_fence = False
    for line_number, line in enumerate(markdown.split("\n"), start=1):
        if line.startswith("```"):
            if not in_fence and line.startswith("```mermaid"):
                raise DocsExportError(
                    f"{relative_path}:{line_number}: mermaid fences are not supported by "
                    "the Fumadocs export; use a plain-text diagram fence or a static SVG asset."
                )
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        if line.startswith("=== "):
            raise DocsExportError(
                f"{relative_path}:{line_number}: pymdownx tabbed blocks ('=== \"...\"') are not "
                "supported by the Fumadocs export; use plain '###' subsections instead."
            )
        if line.startswith("??? "):
            raise DocsExportError(
                f"{relative_path}:{line_number}: pymdownx collapsible blocks ('??? ...') are not "
                "supported by the Fumadocs export; use a '!!!' admonition instead."
            )


def _walk_files(root: Path, suffix: str) -> list[Path]:
    return sorted(path for path in root.rglob(f"*{suffix}") if path.is_file())


def _walk_asset_files(root: Path) -> list[Path]:
    return sorted(
        path
        for path in root.rglob("*")
        if path.is_file() and path.suffix != ".md" and _is_public_asset(path, root)
    )


def _is_public_asset(path: Path, root: Path) -> bool:
    relative_path = path.relative_to(root).as_posix()
    return not any(
        relative_path.startswith(prefix) for prefix in _MKDOCS_ONLY_ASSET_PREFIXES
    )


def _clean_output_dir(output_dir: Path, source_dir: Path) -> None:
    if not output_dir.exists():
        return
    resolved_output = output_dir.resolve()
    resolved_source = source_dir.resolve()
    if resolved_output in {
        Path("/").resolve(),
        resolved_source,
        resolved_source.parent,
        Path.home().resolve(),
    }:
        raise DocsExportError(f"Refusing to clean unsafe docs output directory: {output_dir}")
    shutil.rmtree(output_dir)


def _load_mkdocs_nav(path: Path) -> tuple[str, list[NavNode]]:
    lines = path.read_text(encoding="utf-8").splitlines()
    site_name = path.parent.name
    nav_start: int | None = None

    for index, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith("site_name:"):
            site_name = _unquote_yaml_scalar(stripped.split(":", 1)[1].strip())
        if stripped == "nav:":
            nav_start = index + 1
            break

    if nav_start is None:
        raise DocsExportError(f"MkDocs config has no nav section: {path}")

    nav_lines: list[tuple[int, str]] = []
    for raw_line in lines[nav_start:]:
        if not raw_line.strip() or raw_line.lstrip().startswith("#"):
            continue
        indent = len(raw_line) - len(raw_line.lstrip(" "))
        if indent == 0 and not raw_line.startswith(" "):
            break
        nav_lines.append((indent, raw_line.strip()))

    nav, next_index = _parse_nav_items(nav_lines, 0, nav_lines[0][0] if nav_lines else 0)
    if next_index != len(nav_lines):
        remaining = nav_lines[next_index][1]
        raise DocsExportError(f"Unable to parse MkDocs nav entry: {remaining}")
    return site_name, nav


def _parse_nav_items(
    lines: list[tuple[int, str]],
    start_index: int,
    expected_indent: int,
) -> tuple[list[NavNode], int]:
    nodes: list[NavNode] = []
    index = start_index

    while index < len(lines):
        indent, stripped = lines[index]
        if indent < expected_indent:
            break
        if indent > expected_indent:
            break
        if not stripped.startswith("- "):
            break

        item = stripped[2:].strip()
        if ":" not in item:
            raise DocsExportError(f"Unsupported MkDocs nav item: {item}")
        raw_title, raw_value = item.split(":", 1)
        title = _unquote_yaml_scalar(raw_title.strip())
        value = raw_value.strip()

        if value:
            nodes.append(NavNode(title=title, path=_unquote_yaml_scalar(value)))
            index += 1
            continue

        child_index = index + 1
        if child_index >= len(lines):
            raise DocsExportError(f"MkDocs nav section has no children: {title}")
        child_indent = lines[child_index][0]
        children, index = _parse_nav_items(lines, child_index, child_indent)
        nodes.append(NavNode(title=title, children=tuple(children)))

    return nodes, index


def _unquote_yaml_scalar(value: str) -> str:
    if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
        return value[1:-1]
    return value


def _write_meta_files(output_dir: Path, *, site_name: str, nav: list[NavNode]) -> None:
    root_pages: list[str] = []
    section_meta: dict[PurePosixPath, dict[str, Any]] = {}

    for node in nav:
        if node.path:
            root_pages.append(_page_id(Path(node.path), root=PurePosixPath(".")))
            continue
        section_dir = _section_dir(node)
        root_pages.append(section_dir.as_posix())
        _collect_section_meta(node, section_dir, section_meta)

    _write_json(output_dir / "meta.json", {"title": site_name, "pages": root_pages})
    for section_dir, meta in sorted(section_meta.items(), key=lambda item: item[0].as_posix()):
        _write_json(output_dir / section_dir / "meta.json", meta)


def _collect_section_meta(
    node: NavNode,
    section_dir: PurePosixPath,
    meta: dict[PurePosixPath, dict[str, Any]],
) -> None:
    pages: list[str] = []
    for child in node.children:
        if child.path:
            pages.append(_page_id(Path(child.path), root=section_dir))
            continue
        child_dir = _section_dir(child)
        pages.append(_relative_page_id(child_dir, section_dir))
        _collect_section_meta(child, child_dir, meta)
    meta[section_dir] = {"title": node.title, "pages": pages}


def _section_dir(node: NavNode) -> PurePosixPath:
    paths = [PurePosixPath(path) for path in _leaf_paths(node)]
    parents = [path.parent for path in paths if path.parent.as_posix() != "."]
    if not parents:
        return PurePosixPath(_slugify(node.title))
    common = PurePosixPath(posixpath.commonpath([parent.as_posix() for parent in parents]))
    return common


def _leaf_paths(node: NavNode) -> list[str]:
    if node.path:
        return [node.path]
    paths: list[str] = []
    for child in node.children:
        paths.extend(_leaf_paths(child))
    return paths


def _page_id(path: Path, *, root: PurePosixPath) -> str:
    pure_path = PurePosixPath(path.as_posix()).with_suffix("")
    if root.as_posix() not in {".", ""}:
        try:
            pure_path = pure_path.relative_to(root)
        except ValueError as exc:
            raise DocsExportError(f"Nav page {path} is outside section {root}") from exc
    return pure_path.as_posix()


def _relative_page_id(path: PurePosixPath, root: PurePosixPath) -> str:
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        return path.as_posix()


def _slugify(value: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-")
    return slug or "section"


def _write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f"{json.dumps(value, indent=2)}\n", encoding="utf-8")


def _write_manifest(
    output_dir: Path,
    *,
    pages: list[Path],
    assets_written: int,
    source_dir: Path,
) -> None:
    manifest = {
        "schemaVersion": 1,
        "source": "flowai-harness docs export",
        "sourceDir": source_dir.name,
        "pages": [path.relative_to(source_dir).as_posix() for path in pages],
        "assetsWritten": assets_written,
    }
    _write_json(output_dir / "export-manifest.json", manifest)


def _convert_links(markdown: str, *, relative_path: Path, base_url: str) -> str:
    return re.sub(
        r"\]\(((?!https?:\/\/|mailto:|#|/)[^) \n]+?)\.md(#[^)]+)?\)",
        lambda match: (
            f"]({_resolve_docs_href(match.group(1), relative_path, base_url)}"
            f"{match.group(2) or ''})"
        ),
        markdown,
    )


def _resolve_docs_href(href: str, relative_path: Path, base_url: str) -> str:
    current_dir = PurePosixPath(relative_path.as_posix()).parent
    resolved = PurePosixPath(posixpath.normpath((current_dir / href).as_posix()))
    normalized = _normalize_markdown_href(resolved.as_posix())
    if normalized == ".":
        return base_url.rstrip("/")
    return f"{base_url.rstrip('/')}/{normalized.lstrip('/')}"


def _normalize_markdown_href(href: str) -> str:
    return href.removesuffix("/index")


def _convert_admonitions(markdown: str) -> str:
    lines = markdown.split("\n")
    out: list[str] = []
    in_fence = False
    index = 0

    while index < len(lines):
        line = lines[index]
        if line.startswith("```"):
            in_fence = not in_fence

        if not in_fence and line.startswith("!!! "):
            match = re.match(r'^!!!\s+(\w+)(?:\s+"([^"]+)")?', line)
            kind = match.group(1) if match else "info"
            title = match.group(2) if match and match.group(2) else _default_callout_title(kind)
            callout_type = {"warning": "warn", "danger": "error"}.get(kind, "info")
            body: list[str] = []
            index += 1
            while index < len(lines):
                next_line = lines[index]
                if next_line.startswith("    "):
                    body.append(next_line[4:])
                    index += 1
                    continue
                if next_line.strip() == "":
                    body.append("")
                    index += 1
                    continue
                break
            out.extend(
                [
                    f'<Callout type="{callout_type}" title="{html.escape(title, quote=True)}">',
                    "",
                    *body,
                    "",
                    "</Callout>",
                    "",
                ]
            )
            continue

        out.append(line)
        index += 1

    return "\n".join(out)


def _default_callout_title(kind: str) -> str:
    return {
        "info": "Info",
        "note": "Note",
        "tip": "Tip",
        "warning": "Warning",
        "danger": "Danger",
    }.get(kind, "Note")


def _convert_card_grids(markdown: str, *, base_url: str) -> str:
    return re.sub(
        r'\n?<div class="grid cards" markdown>\n(?P<body>[\s\S]*?)\n</div>',
        lambda match: _render_cards(match.group("body"), base_url=base_url),
        markdown,
    )


def _render_cards(block: str, *, base_url: str) -> str:
    cards: list[str] = []
    pattern = re.compile(
        r"-\s+__(?P<title>[^_]+)__\s*\n(?P<body>.*?)(?=\n-\s+__|\Z)",
        re.DOTALL,
    )
    for match in pattern.finditer(block.strip()):
        title = " ".join(match.group("title").split())
        body = match.group("body").replace("---", "")
        links = list(re.finditer(r"\[(?P<label>[^\]]+)\]\((?P<href>[^)]+)\)", body))
        if not links:
            continue
        href = _fumadocs_href(
            _normalize_markdown_href(links[-1].group("href").removesuffix(".md")),
            base_url,
        )
        description = body[: links[-1].start()]
        description = " ".join(line.strip() for line in description.splitlines() if line.strip())
        description = _markdown_to_plain_text(description)
        cards.append(
            "  "
            f'<Card title="{html.escape(title, quote=True)}" '
            f'href="{html.escape(href, quote=True)}" '
            f'description="{html.escape(description, quote=True)}" />'
        )

    if not cards:
        return ""
    return "\n\n<Cards>\n" + "\n".join(cards) + "\n</Cards>\n"


def _fumadocs_href(href: str, base_url: str) -> str:
    if href.startswith(("http://", "https://", "mailto:", "#", "/")):
        return href
    return f"{base_url.rstrip('/')}/{href.lstrip('/')}"


def _convert_api_directives(markdown: str, *, base_url: str = "/docs") -> str:
    lines = markdown.split("\n")
    out: list[str] = []
    in_fence = False
    index = 0

    while index < len(lines):
        line = lines[index]
        if line.startswith("```"):
            in_fence = not in_fence

        match = None if in_fence else re.match(r"^:{3,4}\s+([A-Za-z_][\w.]+)\s*$", line)
        if match is None:
            out.append(line)
            index += 1
            continue

        symbol = match.group(1)
        obj = _resolve_symbol(symbol)
        option_lines: list[str] = []
        index += 1
        while index < len(lines):
            next_line = lines[index]
            if next_line.startswith("    ") or next_line.startswith("      "):
                option_lines.append(next_line)
                index += 1
                continue
            if next_line.strip() == "":
                option_lines.append(next_line)
                index += 1
                continue
            # Non-indented Markdown after a directive belongs to the page.
            # This preserves manual field tables that augment generated API docs.
            break

        out.append(
            _render_api_reference(
                symbol,
                obj,
                _parse_api_options(option_lines),
                base_url=base_url,
            )
        )
        if _is_pydantic_model(obj):
            index = _skip_following_model_field_table(lines, index)

    return re.sub(r"\n{3,}", "\n\n", "\n".join(out))


def _skip_following_model_field_table(lines: list[str], index: int) -> int:
    table_start = index
    while table_start < len(lines) and lines[table_start].strip() == "":
        table_start += 1
    if table_start >= len(lines):
        return index

    normalized_header = re.sub(r"\s+", " ", lines[table_start].strip())
    if normalized_header not in {
        "| Field | Type | Default |",
        "| Field | Type | Default | Description |",
        "| Parameter | Type | Default |",
        "| Parameter | Type | Default | Description |",
    }:
        return index

    cursor = table_start + 1
    if cursor >= len(lines) or not lines[cursor].lstrip().startswith("|"):
        return index
    cursor += 1
    while cursor < len(lines) and lines[cursor].lstrip().startswith("|"):
        cursor += 1
    return cursor


def _escape_jsx_generics(markdown: str) -> str:
    lines = markdown.split("\n")
    out: list[str] = []
    in_fence = False

    for line in lines:
        if line.startswith("```"):
            in_fence = not in_fence
            out.append(line)
            continue
        if in_fence:
            out.append(line)
            continue
        out.append(_escape_jsx_generics_outside_inline_code(line))

    return "\n".join(out)


def _escape_jsx_generics_outside_inline_code(line: str) -> str:
    parts = re.split(r"(`[^`]*`)", line)
    for index, part in enumerate(parts):
        if index % 2 == 1:
            continue
        parts[index] = re.sub(
            r"\b([A-Za-z_][\w.]*)<([A-Za-z_][\w.,\s|[\]'\"-]*)>",
            lambda match: f"{match.group(1)}&lt;{match.group(2)}&gt;",
            part,
        )
    return "".join(parts)


def _parse_api_options(lines: list[str]) -> dict[str, Any]:
    members: list[str] = []
    in_members = False
    for line in lines:
        stripped = line.strip()
        if stripped == "members:":
            in_members = True
            continue
        if in_members and stripped.startswith("- "):
            members.append(stripped[2:].strip())
            continue
        if stripped and not stripped.startswith("-") and stripped != "options:":
            in_members = False
    return {"members": members}


def _render_api_reference(
    symbol: str,
    obj: Any,
    options: dict[str, Any],
    *,
    base_url: str,
) -> str:
    short_name = symbol.rsplit(".", 1)[-1]
    lines = _render_api_heading(2, symbol, short_name)
    signature = _signature(obj)
    if signature:
        lines.extend([f"`{short_name}{signature}`", ""])
    lines.extend(_render_signature_details(obj, base_url=base_url))
    doc = _local_docstring(obj)
    if doc:
        lines.extend([_format_docstring(doc), ""])
    if _is_pydantic_model(obj):
        lines.extend(_render_model_field_table(obj))

    if inspect.isclass(obj) and options.get("members"):
        for member_name in options["members"]:
            member = getattr(obj, member_name, None)
            if member is None:
                raise DocsExportError(f"API member {member_name} not found on {symbol}")
            lines.extend(
                _render_api_member(
                    f"{symbol}.{member_name}",
                    member_name,
                    member,
                    base_url=base_url,
                )
            )

    return "\n".join(lines).rstrip() + "\n"


def _is_pydantic_model(obj: Any) -> bool:
    return inspect.isclass(obj) and issubclass(obj, BaseModel)


def _render_model_field_table(model: type[BaseModel]) -> list[str]:
    fields = getattr(model, "model_fields", {})
    if not fields:
        return []

    rows: list[tuple[str, str, str, str]] = []
    for name, field in fields.items():
        field_type = _format_annotation(field.annotation, qualify=False)
        default = _model_field_default(field)
        description = field.description or ""
        rows.append((name, field_type, default, description))

    has_descriptions = any(description for _, _, _, description in rows)
    if has_descriptions:
        lines = [
            "| Parameter | Type | Default | Description |",
            "| --- | --- | --- | --- |",
        ]
    else:
        lines = [
            "| Parameter | Type | Default |",
            "| --- | --- | --- |",
        ]

    for name, field_type, default, description in rows:
        if has_descriptions:
            lines.append(
                "| "
                f"{_markdown_code(name)} | "
                f"{_markdown_code(field_type)} | "
                f"{_markdown_code(default)} | "
                f"{_markdown_table_text(description)} |"
            )
            continue
        lines.append(
            "| "
            f"{_markdown_code(name)} | "
            f"{_markdown_code(field_type)} | "
            f"{_markdown_code(default)} |"
        )
    lines.append("")
    return lines


def _model_field_default(field: Any) -> str:
    if field.is_required():
        return "required"
    default_factory = getattr(field, "default_factory", None)
    if default_factory is not None:
        try:
            if default_factory in {dict, list, tuple, set, frozenset}:
                return repr(default_factory())
        except TypeError:
            pass
        return "<factory>"
    return repr(field.default)


def _markdown_code(value: str) -> str:
    text = _markdown_table_text(value, escape_mdx=False)
    if "`" in text:
        escaped = html.escape(text, quote=False)
        escaped = escaped.replace("{", "&#123;").replace("}", "&#125;")
        return f"<code>{escaped}</code>"
    return f"`{text}`"


def _markdown_table_text(value: str, *, escape_mdx: bool = True) -> str:
    text = re.sub(r"\s+", " ", str(value)).strip().replace("|", "&#124;")
    if escape_mdx:
        text = text.replace("{", "&#123;").replace("}", "&#125;")
    return text


def _render_api_member(
    symbol: str,
    name: str,
    member: Any,
    *,
    base_url: str,
) -> list[str]:
    lines = _render_api_heading(3, symbol, name)
    signature = _signature(member)
    if signature:
        lines.extend([f"`{name}{signature}`", ""])
    lines.extend(_render_signature_details(member, base_url=base_url))
    doc = _local_docstring(member)
    if doc:
        lines.extend([_format_docstring(doc), ""])
    return lines


def _render_api_heading(level: int, symbol: str, title: str) -> list[str]:
    hashes = "#" * level
    escaped_symbol = html.escape(symbol, quote=True)
    return ["", f'<span id="{escaped_symbol}"></span>', "", f"{hashes} `{title}`", ""]


def _resolve_symbol(symbol: str) -> Any:
    parts = symbol.split(".")
    for split_at in range(len(parts) - 1, 0, -1):
        module_name = ".".join(parts[:split_at])
        object_path = parts[split_at:]
        try:
            obj: Any = importlib.import_module(module_name)
        except ModuleNotFoundError:
            continue
        try:
            for part in object_path:
                obj = getattr(obj, part)
        except AttributeError:
            continue
        return obj
    raise DocsExportError(f"Unable to import API symbol: {symbol}")


def _signature(obj: Any) -> str:
    try:
        signature = str(inspect.signature(obj))
    except (TypeError, ValueError):
        return ""
    module = getattr(obj, "__module__", "")
    if module:
        signature = signature.replace(f"{module}.", "")
    return signature


def _render_signature_details(obj: Any, *, base_url: str) -> list[str]:
    try:
        signature = inspect.signature(obj)
    except (TypeError, ValueError):
        return []

    parameters = [
        param
        for param in signature.parameters.values()
        if param.name not in {"self", "cls"}
    ]
    lines: list[str] = []
    if parameters:
        lines.extend(
            [
                "<table>",
                "<thead>",
                "<tr>",
                "<th>Parameter</th>",
                "<th>Type</th>",
                "<th>Default</th>",
                "</tr>",
                "</thead>",
                "<tbody>",
            ]
        )
        for param in parameters:
            lines.extend(_render_parameter_row(param, base_url=base_url))
        lines.extend(["</tbody>", "</table>"])
        lines.append("")

    if signature.return_annotation is not inspect.Signature.empty:
        returns = _annotation_html(signature.return_annotation, base_url=base_url)
        lines.extend([f"<p><strong>Returns:</strong> <code>{returns}</code></p>", ""])

    return lines


def _render_parameter_row(param: inspect.Parameter, *, base_url: str) -> list[str]:
    return [
        "<tr>",
        _code_cell(param.name),
        _annotation_cell(param.annotation, base_url=base_url),
        _default_cell(param.default),
        "</tr>",
    ]


def _code_cell(value: str) -> str:
    escaped = html.escape(value, quote=False)
    return f"<td><code>{escaped}</code></td>"


def _annotation_cell(annotation: Any, *, base_url: str) -> str:
    return f"<td><code>{_annotation_html(annotation, base_url=base_url)}</code></td>"


def _annotation_html(annotation: Any, *, base_url: str) -> str:
    escaped = html.escape(_format_annotation(annotation), quote=False)
    return _link_reference_types(escaped, base_url=base_url)


def _link_reference_types(escaped_annotation: str, *, base_url: str) -> str:
    linked = escaped_annotation
    targets = _reference_type_targets_with_qualified_aliases()
    for token in sorted(targets, key=len, reverse=True):
        href = targets[token]
        label = html.escape(token, quote=False)
        target = html.escape(_reference_href(href, base_url), quote=True)
        link = f'<a href="{target}">{label}</a>'
        linked = re.sub(
            rf"(?<![\w.]){re.escape(label)}(?![\w])",
            link,
            linked,
        )
    return linked


def _reference_href(href: str, base_url: str) -> str:
    if href.startswith("/docs/"):
        return f"{base_url.rstrip('/')}/{href.removeprefix('/docs/')}"
    return href


def _reference_type_targets_with_qualified_aliases() -> dict[str, str]:
    targets = dict(_REFERENCE_TYPE_TARGETS)
    for token, href in _REFERENCE_TYPE_TARGETS.items():
        if href.startswith("/docs/reference/runtime#flowai_harness.runtime."):
            targets[f"flowai_harness.runtime.{token}"] = href
        elif href.startswith("/docs/reference/evals#flowai_harness.evals."):
            targets[f"flowai_harness.evals.{token}"] = href
        elif href.startswith("/docs/reference/plans#flowai_harness.plans."):
            targets[f"flowai_harness.plans.{token}"] = href
        elif href.startswith("/docs/reference/references#flowai_harness.references."):
            targets[f"flowai_harness.references.{token}"] = href
        elif href.startswith("/docs/reference/tools#flowai_harness.tools."):
            targets[f"flowai_harness.tools.{token}"] = href
        elif href.startswith("/docs/reference/prompts#flowai_harness.prompts."):
            targets[f"flowai_harness.prompts.{token}"] = href
        elif href.startswith("/docs/reference/tenant#flowai_harness.tenant."):
            targets[f"flowai_harness.tenant.{token}"] = href
        elif href.startswith("/docs/reference/studio#flowai_harness.studio."):
            targets[f"flowai_harness.studio.{token}"] = href
        elif href.startswith("/docs/reference/unions#flowai_harness.unions."):
            targets[f"flowai_harness.unions.{token}"] = href
    return targets


def _default_cell(default: Any) -> str:
    if default is inspect.Signature.empty:
        return "<td>required</td>"
    escaped = html.escape(f"{default!r}", quote=False)
    return f"<td><code>{escaped}</code></td>"


def _format_annotation(annotation: Any, *, qualify: bool = True) -> str:
    if annotation is inspect.Signature.empty:
        return "Any"
    if isinstance(annotation, str):
        return annotation
    if annotation is Any:
        return "Any"
    if annotation is None or annotation is type(None):
        return "None"

    origin = get_origin(annotation)
    args = get_args(annotation)
    if origin is not None:
        if str(origin) == "typing.Annotated" and args:
            return _format_annotation(args[0], qualify=qualify)
        if origin is Literal:
            return "Literal[" + ", ".join(repr(arg) for arg in args) + "]"
        if origin is UnionType or str(origin) == "typing.Union":
            return " | ".join(_format_annotation(arg, qualify=qualify) for arg in args)

        origin_name = _format_annotation(origin, qualify=qualify)
        if args:
            formatted_args = ", ".join(
                "..." if arg is Ellipsis else _format_annotation(arg, qualify=qualify)
                for arg in args
            )
            return f"{origin_name}[{formatted_args}]"
        return origin_name

    if isinstance(annotation, UnionType):
        return " | ".join(
            _format_annotation(arg, qualify=qualify) for arg in annotation.__args__
        )

    module = getattr(annotation, "__module__", "")
    qualname = getattr(annotation, "__qualname__", None)
    if qualname:
        if module == "builtins":
            return qualname
        if not qualify:
            return qualname
        return f"{module}.{qualname}"
    return str(annotation).replace("typing.", "")


def _local_docstring(obj: Any) -> str:
    doc = getattr(obj, "__doc__", None)
    return inspect.cleandoc(doc) if isinstance(doc, str) and doc.strip() else ""


def _format_docstring(docstring: str) -> str:
    return textwrap.dedent(docstring).strip()


def _add_frontmatter(markdown: str, relative_path: Path) -> str:
    title = _extract_title(markdown) or relative_path.stem.replace("-", " ").title()
    description = _extract_description(markdown)
    body = _remove_first_heading(markdown).strip()
    frontmatter = ["---", f"title: {json.dumps(title)}"]
    if description:
        frontmatter.append(f"description: {json.dumps(description)}")
    frontmatter.extend(["---", ""])
    return "\n".join([*frontmatter, body, ""])


def _extract_title(markdown: str) -> str | None:
    match = re.search(r"^#\s+(.+)$", markdown, flags=re.MULTILINE)
    return match.group(1).strip() if match else None


def _remove_first_heading(markdown: str) -> str:
    return re.sub(r"^#\s+.+\n+", "", markdown, count=1)


def _extract_description(markdown: str) -> str:
    without_heading = _remove_first_heading(markdown)
    for block in re.split(r"\n{2,}", without_heading):
        trimmed = block.strip()
        if not trimmed:
            continue
        if trimmed.startswith(("#", "```", "!!!", ":::", "<")):
            continue
        return _truncate_description(_markdown_to_plain_text(trimmed))
    return ""


def _markdown_to_plain_text(markdown: str) -> str:
    text = markdown
    text = re.sub(r"!\[([^\]]*)\]\([^)]+\)", r"\1", text)
    text = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", text)
    text = re.sub(r"`([^`]*)`", r"\1", text)
    text = re.sub(r"<[^>]+>", "", text)
    text = html.unescape(text)
    return re.sub(r"\s+", " ", text).strip()


def _truncate_description(text: str, *, limit: int = 180) -> str:
    if len(text) <= limit:
        return text
    truncated = text[:limit].rstrip()
    if " " in truncated:
        truncated = truncated.rsplit(" ", 1)[0]
    return truncated.rstrip(".,;:") + "..."
