from __future__ import annotations

import hashlib
import json
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
from datetime import date, datetime
from decimal import Decimal
from math import isfinite
from typing import Any, TypeAlias

from pydantic import BaseModel

from flowai_harness.tools import ToolSpec

TextSection: TypeAlias = str | Sequence[str] | None
StructuredSection: TypeAlias = (
    str
    | int
    | float
    | bool
    | Decimal
    | date
    | datetime
    | BaseModel
    | Mapping[str, Any]
    | Sequence[Any]
    | None
)
ToolPromptInput: TypeAlias = ToolSpec | Mapping[str, Any] | str
ToolPromptRow: TypeAlias = tuple[str, str, str]


@dataclass(frozen=True)
class LayeredPrompt:
    """Rendered layered prompt plus its deterministic prompt fingerprint.

    ``cache_key`` is the SHA-256 hash of ``text``. It is used by the harness
    to detect rendered prompt changes and to refresh the agent prompt
    fingerprint after runtime tool descriptions are merged into the prompt.
    """

    text: str
    cache_key: str

    def __str__(self) -> str:
        return self.text


def layered_prompt(
    *,
    identity: str,
    communication: TextSection = None,
    operational_rules: TextSection = None,
    tools: Iterable[ToolPromptInput] | None = None,
    domain_knowledge: StructuredSection = None,
    safety: TextSection = None,
    output_format: StructuredSection = None,
    examples: StructuredSection = None,
) -> LayeredPrompt:
    """Build the sanctioned Flow AI layered prompt shape.

    The returned text is deterministic for identical inputs. The cache key is
    the SHA-256 hash of the rendered prompt text, so changing any rendered
    section changes the key. The key is a stable fingerprint for
    change detection and traceability; it is not a provider credential or a
    provider-side cache directive. Empty sections are omitted.

    Args:
        identity: Who the agent is. Required; string or sequence of strings
            rendered as a bullet list.
        communication: Tone and audience guidance; string or sequence of
            strings.
        operational_rules: Behavioral rules; string or sequence of strings.
        tools: Tool rows rendered as a Markdown table: ``ToolSpec`` values,
            mappings with ``name`` / ``description`` / ``approval``, or
            plain names. Duplicate names keep the first row.
        domain_knowledge: Business context; a string, or any
            JSON-serializable structure rendered as a JSON code block.
        safety: Safety constraints; string or sequence of strings.
        output_format: Expected output shape; string or JSON-serializable
            structure.
        examples: Worked examples; string or JSON-serializable structure.

    Returns:
        A ``LayeredPrompt`` with the rendered text and its deterministic
        cache key.

    Raises:
        TypeError: If a text section is not a string or sequence of strings,
            or a structured section contains values that cannot be rendered
            as JSON.
    """

    sections = [
        _section("Identity", _render_text_section(identity, "identity")),
        _section("Communication", _render_text_section(communication, "communication")),
        _section(
            "Operational Rules",
            _render_text_section(operational_rules, "operational_rules"),
        ),
        _section("Tools", _render_tools(tools)),
        _section("Domain Knowledge", _render_structured(domain_knowledge, "domain_knowledge")),
        _section("Safety", _render_text_section(safety, "safety")),
        _section("Output Format", _render_structured(output_format, "output_format")),
        _section("Examples", _render_structured(examples, "examples")),
    ]
    text = "\n\n".join(section for section in sections if section)
    return LayeredPrompt(text=text, cache_key=_stable_hash(text))


def augment_prompt_tools(
    text: str,
    tools: Iterable[ToolPromptInput],
) -> LayeredPrompt:
    """Return prompt text with the supplied tools merged into `# Tools`.

    Explicit rows already present in a generated `# Tools` table win on
    duplicate names. This lets customers override descriptions in
    `layered_prompt(..., tools=...)` while runtime assembly still documents
    toolkit- and agent-bound tools that were only configured as metadata.
    """

    additions = [_tool_row(tool) for tool in tools]
    if not additions:
        return LayeredPrompt(text=text, cache_key=_stable_hash(text))

    sections = _split_prompt_sections(text)
    tools_index = _find_section_index(sections, "Tools")
    if tools_index is None:
        rows = _dedupe_tool_rows(additions)
        sections.insert(_tools_insertion_index(sections), _section("Tools", _render_tool_rows(rows)))
    else:
        existing_body = _section_body(sections[tools_index])
        existing_rows = _parse_tool_table(existing_body)
        rows = _dedupe_tool_rows([*existing_rows, *additions])
        sections[tools_index] = _section("Tools", _render_tool_rows(rows))

    rendered = "\n\n".join(section for section in sections if section)
    return LayeredPrompt(text=rendered, cache_key=_stable_hash(rendered))


def _section(heading: str, body: Any | None) -> str:
    if body is None or body == "":
        return ""
    return f"# {heading}\n{body}"


def _render_text_section(value: TextSection, field_name: str) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value.strip()
    if isinstance(value, Sequence) and not isinstance(value, (str, bytes, bytearray)):
        if all(isinstance(item, str) for item in value):
            return "\n".join(f"- {item}" for item in value)
    raise TypeError(f"{field_name} must be a string or a sequence of strings")


def _render_structured(value: StructuredSection, field_name: str) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value.strip()
    return "```json\n" + _stable_json(value, field_name, indent=2) + "\n```"


def _render_tools(tools: Iterable[ToolPromptInput] | None) -> str:
    if tools is None:
        return ""
    rows = [_tool_row(tool) for tool in tools]
    if not rows:
        return ""
    return _render_tool_rows(_dedupe_tool_rows(rows))


def _render_tool_rows(rows: Sequence[ToolPromptRow]) -> str:
    if not rows:
        return ""
    sorted_rows = sorted(rows, key=lambda row: row[0])
    lines = [
        "| Tool | Description | Approval |",
        "| --- | --- | --- |",
    ]
    for name, description, approval in sorted_rows:
        lines.append(
            f"| {_escape_table(name)} | {_escape_table(description)} | "
            f"{_escape_table(approval)} |"
        )
    return "\n".join(lines)


def _tool_row(tool: ToolPromptInput) -> tuple[str, str, str]:
    if isinstance(tool, ToolSpec):
        return (
            tool.name,
            tool.description,
            _approval_label(tool.approval),
        )
    if isinstance(tool, str):
        return (tool, "", "")
    name = str(tool.get("name", ""))
    description = str(tool.get("description", ""))
    approval = _approval_label(tool.get("approval", ""))
    return (name, description, approval)


def _approval_label(approval: Any) -> str:
    if approval == "":
        return ""
    if isinstance(approval, str):
        return approval
    if isinstance(approval, Mapping):
        kind = approval.get("kind")
        if isinstance(kind, str):
            return kind
    raise TypeError("tool approval must be a string or a mapping with a string `kind`")


def _escape_table(value: str) -> str:
    return value.replace("|", "\\|").replace("\n", " ").strip()


def _split_prompt_sections(text: str) -> list[str]:
    if not text:
        return []

    sections: list[list[str]] = []
    current: list[str] = []
    for line in text.splitlines():
        if line.startswith("# ") and current:
            sections.append(current)
            current = [line]
        else:
            current.append(line)
    if current:
        sections.append(current)
    return ["\n".join(section).strip() for section in sections if "\n".join(section).strip()]


def _find_section_index(sections: Sequence[str], heading: str) -> int | None:
    marker = f"# {heading}"
    for index, section in enumerate(sections):
        if section.splitlines()[0].strip() == marker:
            return index
    return None


def _section_body(section: str) -> str:
    lines = section.splitlines()
    if len(lines) <= 1:
        return ""
    return "\n".join(lines[1:])


def _tools_insertion_index(sections: Sequence[str]) -> int:
    if not any(section.startswith("# ") for section in sections):
        return len(sections)
    order = [
        "Identity",
        "Communication",
        "Operational Rules",
    ]
    index = 0
    for heading in order:
        found = _find_section_index(sections, heading)
        if found is not None:
            index = found + 1
    return index


def _parse_tool_table(body: str) -> list[ToolPromptRow]:
    rows: list[ToolPromptRow] = []
    for line in body.splitlines():
        line = line.strip()
        if not line.startswith("|") or not line.endswith("|"):
            continue
        cells = _split_table_row(line)
        if len(cells) < 3:
            continue
        name, description, approval = cells[:3]
        if name == "Tool" or set(name) <= {"-"}:
            continue
        if name:
            rows.append((name, description, approval))
    return rows


def _split_table_row(line: str) -> list[str]:
    cells: list[str] = []
    cell: list[str] = []
    escaped = False
    for char in line.strip()[1:-1]:
        if escaped:
            cell.append(char)
            escaped = False
            continue
        if char == "\\":
            escaped = True
            continue
        if char == "|":
            cells.append("".join(cell).strip())
            cell = []
            continue
        cell.append(char)
    if escaped:
        cell.append("\\")
    cells.append("".join(cell).strip())
    return cells


def _dedupe_tool_rows(rows: Iterable[ToolPromptRow]) -> list[ToolPromptRow]:
    by_name: dict[str, ToolPromptRow] = {}
    for name, description, approval in rows:
        key = name.strip()
        if key and key not in by_name:
            by_name[key] = (name, description, approval)
    return list(by_name.values())


def _stable_hash(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _stable_json(value: Any | None, field_name: str, *, indent: int | None = None) -> str:
    return json.dumps(
        _jsonable(value, field_name),
        ensure_ascii=False,
        sort_keys=True,
        separators=None if indent is not None else (",", ":"),
        indent=indent,
    )


def _jsonable(value: Any, field_name: str) -> Any:
    if value is None or isinstance(value, str | int | bool):
        return value
    if isinstance(value, float):
        if not isfinite(value):
            raise TypeError(f"{field_name} contains a non-finite float")
        return value
    if isinstance(value, Decimal):
        return str(value)
    if isinstance(value, datetime | date):
        return value.isoformat()
    if isinstance(value, BaseModel):
        return value.model_dump(by_alias=True, mode="json")
    if isinstance(value, Mapping):
        result: dict[str, Any] = {}
        for key, item in value.items():
            if not isinstance(key, str):
                raise TypeError(f"{field_name} mapping keys must be strings")
            result[key] = _jsonable(item, field_name)
        return result
    if isinstance(value, Sequence) and not isinstance(value, (str, bytes, bytearray)):
        return [_jsonable(item, field_name) for item in value]
    raise TypeError(f"{field_name} contains unsupported value {value!r}")
