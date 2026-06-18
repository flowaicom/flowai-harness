import json
from pathlib import Path

import pytest

from flowai_harness import cli
from flowai_harness.docs_export import (
    DocsExportError,
    ExportOptions,
    export_fumadocs_docs,
)


def write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def test_export_fumadocs_docs_uses_mkdocs_nav_and_converts_markdown(tmp_path):
    docs_dir = tmp_path / "docs"
    output_dir = tmp_path / "dist" / "fumadocs" / "content" / "docs"
    mkdocs_config = tmp_path / "mkdocs.yml"

    write(
        mkdocs_config,
        """
site_name: Test Harness
nav:
  - Home: index.md
  - Concepts:
      - Overview: concepts/index.md
      - Runtime: concepts/runtime.md
  - Reference:
      - Overview: reference/index.md
      - Runtime: reference/runtime.md
""".strip(),
    )
    write(
        docs_dir / "index.md",
        """
# Test Harness

This is the docs homepage.

Plan declaration compiled to Plan<HarnessAction>.

!!! warning "Alpha"
    Pin versions in production.

Read the [runtime concept](concepts/runtime.md).

```text
RuntimeSpec --> Runtime
```

<div class="grid cards" markdown>

-   __Runtime__

    ---

    Learn the runtime.

    [:octicons-arrow-right-24: Runtime](concepts/runtime.md)

</div>
""".strip(),
    )
    write(
        docs_dir / "concepts" / "index.md",
        """
# Concepts

Concept overview.
""".strip(),
    )
    write(
        docs_dir / "concepts" / "runtime.md",
        """
# Runtime

Runtime concept.
""".strip(),
    )
    write(
        docs_dir / "reference" / "index.md",
        """
# Reference

See the [`Runtime`](runtime.md) returned by `create_runtime(...)`.
""".strip(),
    )
    write(
        docs_dir / "reference" / "runtime.md",
        """
# Runtime

Runtime reference.
""".strip(),
    )
    write(docs_dir / "assets" / "logo.svg", "<svg />")
    write(docs_dir / "stylesheets" / "extra.css", "mkdocs theme css")
    write(docs_dir / "stylesheets" / "brand-fonts.css", "private font css")
    write(docs_dir / "stylesheets" / "fonts" / "brand.woff2", "private font")

    result = export_fumadocs_docs(
        ExportOptions(
            source_dir=docs_dir,
            mkdocs_config=mkdocs_config,
            output_dir=output_dir,
        )
    )

    root_meta = json.loads((output_dir / "meta.json").read_text(encoding="utf-8"))
    concepts_meta = json.loads(
        (output_dir / "concepts" / "meta.json").read_text(encoding="utf-8")
    )
    reference_meta = json.loads(
        (output_dir / "reference" / "meta.json").read_text(encoding="utf-8")
    )
    manifest = json.loads(
        (output_dir / "export-manifest.json").read_text(encoding="utf-8")
    )
    index_mdx = (output_dir / "index.mdx").read_text(encoding="utf-8")

    assert result.pages_written == 5
    assert result.assets_written == 1
    assert root_meta == {
        "title": "Test Harness",
        "pages": ["index", "concepts", "reference"],
    }
    assert concepts_meta == {"title": "Concepts", "pages": ["index", "runtime"]}
    assert reference_meta == {"title": "Reference", "pages": ["index", "runtime"]}
    assert manifest["sourceDir"] == "docs"
    assert manifest["pages"] == [
        "concepts/index.md",
        "concepts/runtime.md",
        "index.md",
        "reference/index.md",
        "reference/runtime.md",
    ]
    assert index_mdx.startswith(
        '---\ntitle: "Test Harness"\ndescription: "This is the docs homepage."'
    )
    assert "Plan declaration compiled to Plan&lt;HarnessAction&gt;." in index_mdx
    assert '<Callout type="warn" title="Alpha">' in index_mdx
    assert "[runtime concept](/docs/concepts/runtime)" in index_mdx
    assert "```text\nRuntimeSpec --> Runtime\n```" in index_mdx
    assert '<Card title="Runtime" href="/docs/concepts/runtime"' in index_mdx
    assert (output_dir / "assets" / "logo.svg").read_text(encoding="utf-8") == "<svg />"
    assert not (output_dir / "stylesheets").exists()

    reference_mdx = (output_dir / "reference" / "index.mdx").read_text(
        encoding="utf-8"
    )
    assert (
        'description: "See the Runtime returned by create_runtime(...)."'
        in reference_mdx
    )
    assert "See the [`Runtime`](/docs/reference/runtime) returned by `create_runtime(...)`." in reference_mdx


def test_export_fumadocs_docs_expands_api_directives(tmp_path, monkeypatch):
    package_dir = tmp_path / "fakeapipkg"
    write(package_dir / "__init__.py", "")
    write(
        package_dir / "api.py",
        """
from pydantic import BaseModel, Field

from flowai_harness.runtime import RuntimeSpec


class Widget:
    \"\"\"Widget documentation.

    Compiles to Plan<HarnessAction>.
    \"\"\"

    def run(self, name: str) -> str:
        \"\"\"Run the widget.\"\"\"
        return name


def build_widget(name: str) -> Widget:
    \"\"\"Build a widget.\"\"\"
    return Widget()


def choose_resource(resource_id: str | None = None) -> str | None:
    \"\"\"Choose an optional resource.\"\"\"
    return resource_id


def use_runtime(spec: RuntimeSpec) -> RuntimeSpec:
    \"\"\"Use a runtime spec.\"\"\"
    return spec


class FieldModel(BaseModel):
    \"\"\"Model with documented fields.\"\"\"

    agents: dict[str, str] = Field(
        default_factory=dict,
        description=\"Per-agent approval patches keyed by agent name.\",
    )
    tools: dict[str, dict[str, str]] = Field(
        default_factory=dict,
        description=\"Per-tool approval rules keyed by agent name, then tool name.\",
    )
    providers: dict[str, str] = Field(
        default_factory=dict,
        description='Provider config, e.g. {"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}}.',
    )


class BareModel(BaseModel):
    \"\"\"Model with plain fields.\"\"\"

    name: str
    count: int = 0
""".strip(),
    )
    monkeypatch.syspath_prepend(str(tmp_path))

    docs_dir = tmp_path / "docs"
    output_dir = tmp_path / "dist" / "fumadocs" / "content" / "docs"
    mkdocs_config = tmp_path / "mkdocs.yml"
    write(
        mkdocs_config,
        """
site_name: Test Harness
nav:
  - Reference:
      - Widget: reference/widget.md
""".strip(),
    )
    write(
        docs_dir / "reference" / "widget.md",
        """
# Widget

Widget reference.

:::: fakeapipkg.api.Widget
    options:
      members:
        - run

| Field | Type | Description |
| --- | --- | --- |
| `name` | `str` | Manual field table preserved after API directive. |

:::: fakeapipkg.api.build_widget

:::: fakeapipkg.api.choose_resource

::: fakeapipkg.api.use_runtime

::: fakeapipkg.api.FieldModel

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `agents` | `manual` | `manual` | Manual table that should not be exported. |

::: fakeapipkg.api.BareModel
""".strip(),
    )

    export_fumadocs_docs(
        ExportOptions(
            source_dir=docs_dir,
            mkdocs_config=mkdocs_config,
            output_dir=output_dir,
        )
    )

    widget_mdx = (output_dir / "reference" / "widget.mdx").read_text(encoding="utf-8")

    assert ":::: fakeapipkg.api.Widget" not in widget_mdx
    assert "::: fakeapipkg.api.use_runtime" not in widget_mdx
    assert "{#fakeapipkg.api.Widget}" not in widget_mdx
    assert '<span id="fakeapipkg.api.Widget"></span>\n\n## `Widget`' in widget_mdx
    assert "Widget documentation." in widget_mdx
    assert "Compiles to Plan&lt;HarnessAction&gt;." in widget_mdx
    assert '<span id="fakeapipkg.api.Widget.run"></span>\n\n### `run`' in widget_mdx
    assert "`run(self, name: str) -> str`" in widget_mdx
    assert "| Field | Type | Description |" in widget_mdx
    assert (
        "| `name` | `str` | Manual field table preserved after API directive. |"
        in widget_mdx
    )
    assert "<th>Parameter</th>" in widget_mdx
    assert "<td><code>name</code></td>" in widget_mdx
    assert "<td><code>str</code></td>" in widget_mdx
    assert "<td>required</td>" in widget_mdx
    assert (
        "<p><strong>Returns:</strong> <code>str</code></p>"
        in widget_mdx
    )
    assert "## `build_widget`" in widget_mdx
    assert "`build_widget(name: str) -> Widget`" in widget_mdx
    assert "## `choose_resource`" in widget_mdx
    assert "`choose_resource(resource_id: str | None = None) -> str | None`" in widget_mdx
    assert "<td><code>resource_id</code></td>" in widget_mdx
    assert "<td><code>str | None</code></td>" in widget_mdx
    assert "<td><code>None</code></td>" in widget_mdx
    assert "| `resource_id` | `str | None` | `None` |" not in widget_mdx
    runtime_spec_link = (
        '<a href="/docs/reference/runtime#flowai_harness.runtime.RuntimeSpec">'
        "flowai_harness.runtime.RuntimeSpec</a>"
    )
    assert f"<td><code>{runtime_spec_link}</code></td>" in widget_mdx
    assert f"<p><strong>Returns:</strong> <code>{runtime_spec_link}</code></p>" in widget_mdx
    assert "## `FieldModel`" in widget_mdx
    assert "| Parameter | Type | Default | Description |" in widget_mdx
    assert (
        "| `agents` | `dict[str, str]` | `{}` | "
        "Per-agent approval patches keyed by agent name. |"
        in widget_mdx
    )
    assert (
        "| `tools` | `dict[str, dict[str, str]]` | `{}` | "
        "Per-tool approval rules keyed by agent name, then tool name. |"
        in widget_mdx
    )
    assert (
        "| `providers` | `dict[str, str]` | `{}` | Provider config, e.g. "
        "&#123;\"anthropic\": &#123;\"apiKeyEnv\": \"ANTHROPIC_API_KEY\"&#125;&#125;. |"
        in widget_mdx
    )
    assert '{"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}}' not in widget_mdx
    assert "Manual table that should not be exported." not in widget_mdx
    bare_model_section = widget_mdx.split("## `BareModel`", 1)[1]
    assert "| Parameter | Type | Default |\n| --- | --- | --- |" in bare_model_section
    assert "| Parameter | Type | Default | Description |" not in bare_model_section


def test_export_fumadocs_docs_does_not_emit_inherited_class_docstrings(
    tmp_path, monkeypatch
):
    package_dir = tmp_path / "fakedocpkg"
    write(package_dir / "__init__.py", "")
    write(
        package_dir / "models.py",
        """
class Base:
    \"\"\"Inherited base docs with a [missing link](/docs/concepts/models).\"\"\"


class Child(Base):
    pass
""".strip(),
    )
    monkeypatch.syspath_prepend(str(tmp_path))

    docs_dir = tmp_path / "docs"
    output_dir = tmp_path / "dist" / "fumadocs" / "content" / "docs"
    mkdocs_config = tmp_path / "mkdocs.yml"
    write(
        mkdocs_config,
        """
site_name: Test Harness
nav:
  - Reference:
      - Child: reference/child.md
""".strip(),
    )
    write(
        docs_dir / "reference" / "child.md",
        """
# Child

:::: fakedocpkg.models.Child
""".strip(),
    )

    export_fumadocs_docs(
        ExportOptions(
            source_dir=docs_dir,
            mkdocs_config=mkdocs_config,
            output_dir=output_dir,
        )
    )

    child_mdx = (output_dir / "reference" / "child.mdx").read_text(encoding="utf-8")

    assert "{#fakedocpkg.models.Child}" not in child_mdx
    assert '<span id="fakedocpkg.models.Child"></span>\n\n## `Child`' in child_mdx
    assert "/docs/concepts/models" not in child_mdx
    assert "Inherited base docs" not in child_mdx


def test_cli_docs_export_invokes_fumadocs_exporter(tmp_path, monkeypatch):
    calls = []

    def fake_export(options):
        calls.append(options)
        return type("Result", (), {"pages_written": 2, "assets_written": 1})()

    monkeypatch.setattr(cli, "export_fumadocs_docs", fake_export)

    code = cli.main(
        [
            "flowai-harness",
            "docs",
            "export",
            "--source-dir",
            str(tmp_path / "docs"),
            "--mkdocs-config",
            str(tmp_path / "mkdocs.yml"),
            "--out-dir",
            str(tmp_path / "out"),
            "--no-clean",
        ]
    )

    assert code == 0
    assert len(calls) == 1
    assert calls[0].source_dir == tmp_path / "docs"
    assert calls[0].mkdocs_config == tmp_path / "mkdocs.yml"
    assert calls[0].output_dir == tmp_path / "out"
    assert calls[0].clean is False


def _minimal_docs(tmp_path: Path, index_body: str) -> ExportOptions:
    docs_dir = tmp_path / "docs"
    mkdocs_config = tmp_path / "mkdocs.yml"
    write(mkdocs_config, "site_name: Test Harness\nnav:\n  - Home: index.md\n")
    write(docs_dir / "index.md", index_body)
    return ExportOptions(
        source_dir=docs_dir,
        mkdocs_config=mkdocs_config,
        output_dir=tmp_path / "dist",
    )


def test_export_fumadocs_docs_rejects_tabbed_blocks(tmp_path):
    options = _minimal_docs(
        tmp_path,
        '# Home\n\n=== "Dict shorthand"\n\n    body\n',
    )
    with pytest.raises(DocsExportError, match=r"index\.md:3: pymdownx tabbed"):
        export_fumadocs_docs(options)


def test_export_fumadocs_docs_rejects_mermaid_fences(tmp_path):
    options = _minimal_docs(
        tmp_path,
        "# Home\n\n```mermaid\nflowchart LR\n```\n",
    )
    with pytest.raises(DocsExportError, match=r"index\.md:3: mermaid"):
        export_fumadocs_docs(options)


def test_export_fumadocs_docs_rejects_collapsible_blocks(tmp_path):
    options = _minimal_docs(
        tmp_path,
        '# Home\n\n??? note "Details"\n\n    body\n',
    )
    with pytest.raises(DocsExportError, match=r"index\.md:3: pymdownx collapsible"):
        export_fumadocs_docs(options)


def test_export_fumadocs_docs_allows_tab_like_lines_inside_fences(tmp_path):
    options = _minimal_docs(
        tmp_path,
        '# Home\n\n```text\n=== "not a tab"\n??? not a collapsible\n```\n',
    )
    result = export_fumadocs_docs(options)
    assert result.pages_written == 1
