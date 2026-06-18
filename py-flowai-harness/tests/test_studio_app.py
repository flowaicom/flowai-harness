import sys

import pytest

from flowai_harness import (
    FlowAIApp,
    WorkspaceRuntimeBinding,
    define_app,
    define_coordinator,
    define_runtime,
    define_specialist,
    define_tenant,
    define_workspace_runtime,
)
from flowai_harness.studio import AppImportError, resolve_app_reference


def _runtime_spec():
    specialist = define_specialist(
        name="insights",
        model="claude-sonnet-4-6",
        prompt="You inspect data.",
    )
    coordinator = define_coordinator(
        name="scenario_coordinator",
        model="claude-sonnet-4-6",
        prompt="You coordinate work.",
        routes=["insights"],
    )
    return define_runtime(
        tenant=define_tenant("acme", "v1"),
        agents=[coordinator, specialist],
        providers={"anthropic": {"apiKey": "must-not-leak"}},
    )


def test_define_app_single_runtime_sugar_creates_default_workspace():
    app = define_app(name="demo", runtime_spec=_runtime_spec())

    assert isinstance(app, FlowAIApp)
    assert app.default_workspace == "default"
    assert list(app.workspaces) == ["default"]
    assert isinstance(app.default_binding(), WorkspaceRuntimeBinding)


def test_workspace_runtime_is_lazy_and_cached():
    calls = []

    def build_runtime():
        calls.append("called")
        return {"runtime": len(calls)}

    binding = define_workspace_runtime(
        runtime_spec=_runtime_spec(),
        runtime_factory=build_runtime,
    )
    app = define_app(name="demo", workspaces={"default": binding})

    assert app.default_binding().runtime_constructed is False
    assert app.default_binding().get_runtime() == {"runtime": 1}
    assert app.default_binding().get_runtime() == {"runtime": 1}
    assert calls == ["called"]


def test_runtime_summary_redacts_provider_secrets():
    app = define_app(name="demo", runtime_spec=_runtime_spec())

    summary = app.default_binding().runtime_summary()

    assert summary["tenant"] == {"tenantId": "acme", "version": "v1"}
    assert summary["agents"][0]["agentId"] == "scenario_coordinator"
    assert summary["providers"] == [
        {
            "name": "anthropic",
            "configured": True,
            "credential": {"kind": "serverManaged"},
        }
    ]
    assert "must-not-leak" not in repr(summary)


def test_define_app_rejects_unknown_default_workspace():
    with pytest.raises(ValueError, match="not registered"):
        define_app(
            name="demo",
            default_workspace="missing",
            workspaces={"default": define_workspace_runtime(runtime_spec=_runtime_spec())},
        )


def test_resolve_app_reference_imports_flowai_app(tmp_path, monkeypatch):
    module_path = tmp_path / "studio_fixture.py"
    module_path.write_text(
        "from flowai_harness import define_app, define_runtime, define_tenant\n"
        "app = define_app(\n"
        "    name='fixture',\n"
        "    runtime_spec=define_runtime(tenant=define_tenant('acme', 'v1')),\n"
        ")\n",
        encoding="utf-8",
    )
    monkeypatch.syspath_prepend(str(tmp_path))

    app = resolve_app_reference("studio_fixture:app")

    assert app.name == "fixture"


def test_resolve_app_reference_returns_structured_import_errors():
    with pytest.raises(AppImportError) as exc_info:
        resolve_app_reference("missing_module:app")

    error = exc_info.value.to_error_response()["error"]
    assert error["code"] == "app_import.module_failed"
    assert error["details"]["module"] == "missing_module"
    assert error["details"]["symbol"] == "app"


def test_resolve_app_reference_rejects_invalid_reference():
    with pytest.raises(AppImportError) as exc_info:
        resolve_app_reference("missing-colon")

    assert exc_info.value.to_error_response()["error"]["code"] == "app_import.invalid_reference"


def test_resolve_app_reference_allows_zero_arg_factory(tmp_path, monkeypatch):
    module_path = tmp_path / "studio_factory_fixture.py"
    module_path.write_text(
        "from flowai_harness import define_app, define_runtime, define_tenant\n"
        "def build_app():\n"
        "    return define_app(\n"
        "        name='factory',\n"
        "        runtime_spec=define_runtime(tenant=define_tenant('acme', 'v1')),\n"
        "    )\n",
        encoding="utf-8",
    )
    monkeypatch.syspath_prepend(str(tmp_path))
    sys.modules.pop("studio_factory_fixture", None)

    app = resolve_app_reference("studio_factory_fixture:build_app")

    assert app.name == "factory"
