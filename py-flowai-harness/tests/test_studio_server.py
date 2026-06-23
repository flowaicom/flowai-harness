from fastapi.testclient import TestClient
import flowai_harness.studio.server as studio_server

from flowai_harness import (
    __version__,
    define_app,
    define_coordinator,
    define_runtime,
    define_specialist,
    define_tenant,
)
from flowai_harness.studio import create_studio_app

AUTH_TOKEN = "test-studio-token"


def _app():
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
    runtime = define_runtime(
        tenant=define_tenant("acme", "v1"),
        agents=[coordinator, specialist],
        providers={"anthropic": {"apiKeyEnv": "ANTHROPIC_API_KEY"}},
    )
    return define_app(name="demo", runtime_spec=runtime)


def _client(**kwargs):
    return TestClient(create_studio_app(_app(), auth_token=AUTH_TOKEN, **kwargs))


def _auth_headers(**headers):
    return {"X-FlowAI-Studio-Token": AUTH_TOKEN, **headers}


def test_studio_fastapi_app_exposes_initial_contract_endpoints():
    client = _client()

    status = client.get("/api/status", headers=_auth_headers())
    assert status.status_code == 200
    status_body = status.json()
    assert status_body["studioApiVersion"] == "harness-studio/v1"
    assert status_body["supportedVersions"] == ["harness-studio/v1"]
    assert status_body["implementation"] == {
        "name": "py-flowai-harness",
        "version": __version__,
        "mode": "local",
    }

    workspaces = client.get("/api/workspaces", headers=_auth_headers()).json()
    assert workspaces["defaultWorkspaceKey"] == "default"
    assert workspaces["workspaces"][0]["workspaceKey"] == "default"
    assert (
        client.get("/api/workspaces/default", headers=_auth_headers()).json()
        == workspaces["workspaces"][0]
    )

    runtime = client.get("/api/workspaces/default/runtime", headers=_auth_headers()).json()
    assert runtime["tenant"] == {"tenantId": "acme", "version": "v1"}
    assert runtime["providers"][0]["credential"] == {
        "kind": "env",
        "ref": "ANTHROPIC_API_KEY",
    }

    agents = client.get("/api/workspaces/default/agents", headers=_auth_headers()).json()
    assert [agent["agentId"] for agent in agents["agents"]] == [
        "scenario_coordinator",
        "insights",
    ]

    assert client.get("/api/runtime", headers=_auth_headers()).json() == runtime
    assert client.get("/api/agents", headers=_auth_headers()).json() == agents

    capabilities = client.get(
        "/api/workspaces/default/capabilities",
        headers=_auth_headers(),
    ).json()
    assert capabilities["capabilities"][0] == {
        "id": "runtime.inspect",
        "enabled": True,
        "scope": "local",
    }

    config = client.get("/__flowai_config.js")
    assert config.status_code == 200
    assert "window.__FLOWAI__" in config.text
    assert "harness-studio/v1" in config.text
    assert AUTH_TOKEN in config.text


def test_studio_fastapi_app_returns_standard_error_for_unknown_workspace():
    client = _client()

    response = client.get("/api/workspaces/missing/runtime", headers=_auth_headers())

    assert response.status_code == 404
    body = response.json()
    assert body["error"]["code"] == "workspace.not_found"
    assert body["error"]["retryable"] is False
    assert body["error"]["details"] == {"workspaceKey": "missing"}


def test_studio_fastapi_app_requires_api_authentication():
    client = _client()

    missing = client.get("/api/status")
    invalid = client.get("/api/status", headers={"X-FlowAI-Studio-Token": "wrong"})
    bearer = client.get("/api/status", headers={"Authorization": f"Bearer {AUTH_TOKEN}"})

    assert missing.status_code == 401
    assert invalid.status_code == 401
    assert bearer.status_code == 200


def test_studio_fastapi_app_rejects_disallowed_write_origins():
    client = _client()

    response = client.post(
        "/api/workspaces/default/runs/run-1/cancel",
        headers=_auth_headers(Origin="https://attacker.example"),
    )
    allowed = client.post(
        "/api/workspaces/default/runs/run-1/cancel",
        headers=_auth_headers(Origin="http://testserver"),
    )

    assert response.status_code == 403
    assert allowed.status_code == 200


def test_studio_fastapi_app_serves_packaged_static_ui(tmp_path, monkeypatch):
    static_dir = tmp_path / "static"
    assets_dir = static_dir / "assets"
    assets_dir.mkdir(parents=True)
    (static_dir / "index.html").write_text(
        '<script src="/__flowai_config.js"></script>'
        '<link rel="stylesheet" href="/assets/app.css" />'
        '<div id="root"></div>',
        encoding="utf-8",
    )
    (assets_dir / "app.css").write_text("body { color: white; }", encoding="utf-8")
    monkeypatch.setattr(studio_server, "_studio_static_dir", lambda: static_dir)

    client = _client(serve_studio=True)

    root = client.get("/")
    assert root.status_code == 200
    assert "/__flowai_config.js" in root.text
    assert root.headers["cache-control"] == "no-store"

    spa_route = client.get("/evals/eval-1")
    assert spa_route.status_code == 200
    assert "/assets/app.css" in spa_route.text

    asset = client.get("/assets/app.css")
    assert asset.status_code == 200
    assert asset.text == "body { color: white; }"
    assert "max-age=31536000" in asset.headers["cache-control"]


def test_studio_fastapi_app_reports_static_unavailable(tmp_path, monkeypatch):
    monkeypatch.setattr(studio_server, "_studio_static_dir", lambda: tmp_path / "missing")
    client = _client(serve_studio=True)

    response = client.get("/")

    assert response.status_code == 503
    assert response.json()["studioApiVersion"] == "harness-studio/v1"


def test_studio_fastapi_app_can_disable_static_ui_serving():
    client = _client(serve_studio=False)

    response = client.get("/")

    assert response.status_code == 503
    assert "disabled" in response.json()["message"]
