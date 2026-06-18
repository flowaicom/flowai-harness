import json
import sqlite3

from flowai_harness import cli


def test_cli_help_returns_success():
    assert cli.main(["flowai-harness", "--help"]) == 0


def test_cli_profile_and_catalog_export_exercise_rust_path(tmp_path):
    """Profile a sqlite target, then export the catalog, both through the
    shipped console script — proving the Rust CLI path runs end to end without
    a Python reimplementation, an Anthropic key, or network access."""
    target_path = tmp_path / "target.db"
    catalog_path = tmp_path / "catalog.db"
    env_path = tmp_path / "data_environment.json"
    out_path = tmp_path / "catalog.entries.json"

    conn = sqlite3.connect(target_path)
    conn.executescript(
        "CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL, price REAL NOT NULL);"
        "INSERT INTO products (id, name, price) VALUES (1, 'Tea', 4.5), (2, 'Coffee', 7.0);"
    )
    conn.close()

    env_path.write_text(
        json.dumps(
            {
                "tenantId": None,
                "workspaceId": None,
                "kv": None,
                "catalog": {
                    "kind": "sqlite",
                    "url": f"sqlite:{catalog_path}",
                    "ensureSchema": True,
                },
                "catalogSearch": None,
                "targetDatabase": {"kind": "sqlite", "url": f"sqlite:{target_path}"},
            }
        )
    )

    assert (
        cli.main(
            [
                "flowai-harness",
                "--data-environment",
                str(env_path),
                "--output",
                "ndjson",
                "data",
                "profile",
                "database",
                "--database-id",
                "acme",
                "--schema-only",
            ]
        )
        == 0
    )

    assert (
        cli.main(
            [
                "flowai-harness",
                "--data-environment",
                str(env_path),
                "data",
                "catalog",
                "export",
                "--out",
                str(out_path),
            ]
        )
        == 0
    )

    entries = json.loads(out_path.read_text())
    assert isinstance(entries, list) and entries
    assert all("itemType" in entry for entry in entries)
    assert any(entry.get("name") == "products" for entry in entries)


def test_cli_knowledge_help_returns_success():
    assert cli.main(["flowai-harness", "data", "knowledge", "ingest", "--help"]) == 0


def test_cli_catalog_graph_help_returns_success():
    assert cli.main(["flowai-harness", "data", "catalog", "graph", "--help"]) == 0


def test_cli_dev_imports_app_and_runs_server(tmp_path, monkeypatch):
    module_path = tmp_path / "studio_cli_fixture.py"
    module_path.write_text(
        "from flowai_harness import define_app, define_runtime, define_tenant\n"
        "app = define_app(\n"
        "    name='fixture',\n"
        "    runtime_spec=define_runtime(tenant=define_tenant('acme', 'v1')),\n"
        ")\n",
        encoding="utf-8",
    )
    monkeypatch.syspath_prepend(str(tmp_path))
    calls = []
    frontend_calls = []

    def fake_run_studio_server(app, **kwargs):
        calls.append((app.name, kwargs))

    def fake_start_frontend(options):
        frontend_calls.append(
            {
                "backend_port": options.port,
                "frontend_port": options.frontend_port,
            }
        )
        return object()

    def fake_stop_frontend(process):
        frontend_calls.append({"stopped": process is not None})

    monkeypatch.setattr(cli, "run_studio_server", fake_run_studio_server)
    monkeypatch.setattr(cli, "_start_frontend_dev_server", fake_start_frontend)
    monkeypatch.setattr(cli, "_terminate_frontend_dev_server", fake_stop_frontend)

    assert (
        cli.main(
            [
                "flowai-harness",
                "dev",
                "--app",
                "studio_cli_fixture:app",
                "--port",
                "0",
            ]
        )
        == 0
    )
    assert frontend_calls == []
    assert calls == [
        (
            "fixture",
            {"host": "127.0.0.1", "port": 0, "serve_studio": True},
        )
    ]


def test_cli_dev_with_studio_dir_starts_source_frontend(tmp_path, monkeypatch):
    module_path = tmp_path / "studio_cli_fixture.py"
    studio_dir = tmp_path / "studio"
    studio_dir.mkdir()
    (studio_dir / "package.json").write_text("{}", encoding="utf-8")
    module_path.write_text(
        "from flowai_harness import define_app, define_runtime, define_tenant\n"
        "app = define_app(\n"
        "    name='fixture',\n"
        "    runtime_spec=define_runtime(tenant=define_tenant('acme', 'v1')),\n"
        ")\n",
        encoding="utf-8",
    )
    monkeypatch.syspath_prepend(str(tmp_path))
    calls = []
    frontend_calls = []

    def fake_run_studio_server(app, **kwargs):
        calls.append((app.name, kwargs))

    def fake_start_frontend(options):
        frontend_calls.append(
            {
                "backend_port": options.port,
                "frontend_host": options.frontend_host,
                "frontend_port": options.frontend_port,
                "studio_dir": options.studio_dir,
            }
        )
        return object()

    def fake_stop_frontend(process):
        frontend_calls.append({"stopped": process is not None})

    monkeypatch.setattr(cli, "run_studio_server", fake_run_studio_server)
    monkeypatch.setattr(cli, "_start_frontend_dev_server", fake_start_frontend)
    monkeypatch.setattr(cli, "_terminate_frontend_dev_server", fake_stop_frontend)

    assert (
        cli.main(
            [
                "flowai-harness",
                "dev",
                "--app",
                "studio_cli_fixture:app",
                "--studio-dir",
                str(studio_dir),
                "--frontend-host",
                "0.0.0.0",
                "--frontend-port",
                "3001",
            ]
        )
        == 0
    )
    assert frontend_calls == [
        {
            "backend_port": 4111,
            "frontend_host": "0.0.0.0",
            "frontend_port": 3001,
            "studio_dir": str(studio_dir),
        },
        {"stopped": True},
    ]
    assert calls == [
        (
            "fixture",
            {"host": "127.0.0.1", "port": 4111, "serve_studio": True},
        )
    ]


def test_cli_dev_can_skip_frontend(tmp_path, monkeypatch):
    module_path = tmp_path / "studio_cli_fixture.py"
    module_path.write_text(
        "from flowai_harness import define_app, define_runtime, define_tenant\n"
        "app = define_app(\n"
        "    name='fixture',\n"
        "    runtime_spec=define_runtime(tenant=define_tenant('acme', 'v1')),\n"
        ")\n",
        encoding="utf-8",
    )
    monkeypatch.syspath_prepend(str(tmp_path))
    calls = []
    frontend_calls = []

    def fake_run_studio_server(app, **kwargs):
        calls.append((app.name, kwargs))

    monkeypatch.setattr(cli, "run_studio_server", fake_run_studio_server)
    monkeypatch.setattr(
        cli,
        "_start_frontend_dev_server",
        lambda _options: frontend_calls.append("started"),
    )

    assert (
        cli.main(
            [
                "flowai-harness",
                "dev",
                "--app",
                "studio_cli_fixture:app",
                "--no-frontend",
            ]
        )
        == 0
    )
    assert frontend_calls == []
    assert calls == [
        (
            "fixture",
            {"host": "127.0.0.1", "port": 4111, "serve_studio": True},
        )
    ]


def test_cli_serve_does_not_start_frontend(tmp_path, monkeypatch):
    module_path = tmp_path / "studio_cli_fixture.py"
    module_path.write_text(
        "from flowai_harness import define_app, define_runtime, define_tenant\n"
        "app = define_app(\n"
        "    name='fixture',\n"
        "    runtime_spec=define_runtime(tenant=define_tenant('acme', 'v1')),\n"
        ")\n",
        encoding="utf-8",
    )
    monkeypatch.syspath_prepend(str(tmp_path))
    calls = []
    frontend_calls = []

    def fake_run_studio_server(app, **kwargs):
        calls.append((app.name, kwargs))

    monkeypatch.setattr(cli, "run_studio_server", fake_run_studio_server)
    monkeypatch.setattr(
        cli,
        "_start_frontend_dev_server",
        lambda _options: frontend_calls.append("started"),
    )

    assert cli.main(["flowai-harness", "serve", "--app", "studio_cli_fixture:app"]) == 0
    assert frontend_calls == []
    assert calls == [
        (
            "fixture",
            {"host": "127.0.0.1", "port": 4111, "serve_studio": True},
        )
    ]


def test_cli_dev_bad_app_returns_structured_error(capsys):
    code = cli.main(["flowai-harness", "dev", "--app", "missing_module:app"])

    captured = capsys.readouterr()
    assert code == 2
    assert "app_import.module_failed" in captured.err


def test_cli_serve_rejects_frontend_flags(capsys):
    code = cli.main(
        [
            "flowai-harness",
            "serve",
            "--app",
            "studio_cli_fixture:app",
            "--frontend-port",
            "3001",
        ]
    )

    captured = capsys.readouterr()
    assert code == 2
    assert "unrecognized arguments: --frontend-port 3001" in captured.err


def test_cli_dev_rejects_frontend_port_without_studio_dir(tmp_path, monkeypatch, capsys):
    module_path = tmp_path / "studio_cli_fixture.py"
    module_path.write_text(
        "from flowai_harness import define_app, define_runtime, define_tenant\n"
        "app = define_app(\n"
        "    name='fixture',\n"
        "    runtime_spec=define_runtime(tenant=define_tenant('acme', 'v1')),\n"
        ")\n",
        encoding="utf-8",
    )
    monkeypatch.syspath_prepend(str(tmp_path))

    code = cli.main(
        [
            "flowai-harness",
            "dev",
            "--app",
            "studio_cli_fixture:app",
            "--frontend-port",
            "3001",
        ]
    )

    captured = capsys.readouterr()
    assert code == 2
    assert "--frontend-host and --frontend-port require --studio-dir" in captured.err
