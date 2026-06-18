from __future__ import annotations

import asyncio
import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping
from uuid import uuid4

import uvicorn
from fastapi import FastAPI, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse, JSONResponse, Response, StreamingResponse

from pydantic import ValidationError

import flowai_harness._internal as _internal
from flowai_harness._version import package_version
from flowai_harness.evals import (
    EvalConfig,
    EvalRequest,
    EvalTestCase,
    RawSampleOutput,
    define_expected_actions,
    define_test_case,
    score_sample,
)
from flowai_harness.studio.app import FlowAIApp, STUDIO_API_VERSION
from flowai_harness.studio.sse import (
    encode_sse,
    normalize_chat_request,
    project_runtime_event,
    studio_event,
)
from flowai_harness.studio.store import StudioStore


@dataclass(frozen=True)
class StudioApiError(Exception):
    status_code: int
    code: str
    message: str
    details: dict[str, Any] | None = None


@dataclass
class _ActiveChatRun:
    stream: Any
    thread_id: str
    agent_id: str
    cancel_recorded: bool = False


def create_studio_app(
    app: FlowAIApp,
    *,
    serve_studio: bool = True,
    store: StudioStore | None = None,
    store_path: str | Path | None = ".flowai/studio.db",
) -> FastAPI:
    """Create the FlowAI Harness Studio FastAPI application."""

    studio_store = store or StudioStore(store_path)
    api = FastAPI(
        title="FlowAI Harness Studio API",
        version=STUDIO_API_VERSION,
        docs_url="/api/docs",
        redoc_url="/api/redoc",
        openapi_url="/api/openapi.json",
    )
    api.state.flowai_app = app
    api.state.serve_studio = serve_studio
    api.state.studio_store = studio_store
    active_chat_runs: dict[tuple[str, str], _ActiveChatRun] = {}
    api.state.active_chat_runs = active_chat_runs
    api.add_middleware(
        CORSMiddleware,
        allow_origins=[
            "http://127.0.0.1:3000",
            "http://localhost:3000",
            "http://127.0.0.1:4111",
            "http://localhost:4111",
        ],
        allow_methods=["*"],
        allow_headers=["*"],
    )

    @api.middleware("http")
    async def _csp_report_only(_request: Request, call_next):
        # Content-Security-Policy-Report-Only: never blocks, never alters
        # execution. Violations are logged to the browser console only (no
        # report endpoint is configured, so no network egress). This collects
        # the violation set needed to switch on an *enforced* CSP with zero
        # risk in a later release. See SECURITY.md.
        response = await call_next(_request)
        response.headers["Content-Security-Policy-Report-Only"] = (
            "default-src 'self'; "
            "script-src 'self'; "
            "style-src 'self' 'unsafe-inline'; "
            "img-src 'self' data: blob:; "
            "connect-src 'self'; "
            "font-src 'self' data:; "
            "object-src 'none'; "
            "base-uri 'self'; "
            "frame-ancestors 'none'"
        )
        return response

    @api.exception_handler(StudioApiError)
    async def _studio_api_error_handler(
        _request: Request,
        exc: StudioApiError,
    ) -> JSONResponse:
        return _error_response(
            exc.status_code,
            code=exc.code,
            message=exc.message,
            details=exc.details,
        )

    @api.exception_handler(KeyError)
    async def _key_error_handler(_request: Request, exc: KeyError) -> JSONResponse:
        return _error_response(
            404,
            code="workspace.not_found",
            message=str(exc).strip("'"),
        )

    @api.get("/__flowai_config.js", include_in_schema=False)
    async def flowai_config() -> Response:
        return Response(
            app.config_js(),
            media_type="application/javascript; charset=utf-8",
            headers={"Cache-Control": "no-store"},
        )

    @api.get("/assets/{asset_path:path}", include_in_schema=False)
    async def studio_asset(asset_path: str) -> Response:
        if not serve_studio:
            return _studio_static_unavailable_response(serve_studio=False)
        return _studio_static_file_response(
            f"assets/{asset_path}",
            cache_control="public, max-age=31536000, immutable",
        )

    @api.get("/", include_in_schema=False)
    async def studio_root() -> Response:
        if not serve_studio:
            return _studio_static_unavailable_response(serve_studio=False)
        return _studio_index_response()

    @api.get("/api/status")
    async def status() -> dict[str, Any]:
        return {
            "studioApiVersion": STUDIO_API_VERSION,
            "supportedVersions": [STUDIO_API_VERSION],
            "status": "ready",
            "implementation": {
                "name": "py-flowai-harness",
                "version": package_version(),
                "mode": "local",
            },
        }

    @api.get("/api/workspaces")
    async def workspaces() -> dict[str, Any]:
        return app.workspaces_response()

    @api.get("/api/workspaces/{workspace_key}")
    async def workspace(workspace_key: str) -> dict[str, Any]:
        return _binding(app, workspace_key).workspace_summary()

    @api.get("/api/workspaces/{workspace_key}/runtime")
    async def workspace_runtime(workspace_key: str) -> dict[str, Any]:
        return _binding(app, workspace_key).runtime_summary()

    @api.get("/api/workspaces/{workspace_key}/capabilities")
    async def workspace_capabilities(workspace_key: str) -> dict[str, Any]:
        return _binding(app, workspace_key).capability_registry()

    @api.get("/api/workspaces/{workspace_key}/agents")
    async def workspace_agents(workspace_key: str) -> dict[str, Any]:
        return _binding(app, workspace_key).agents_response()

    @api.get("/api/workspaces/{workspace_key}/eval-capabilities")
    async def workspace_eval_capabilities(workspace_key: str) -> dict[str, Any]:
        return _binding(app, workspace_key).eval_capabilities_response()

    @api.get("/api/workspaces/{workspace_key}/data/sources")
    async def list_data_sources(workspace_key: str) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        return {
            "workspaceKey": workspace_key,
            "sources": [_workspace_runtime_source(binding)],
        }

    @api.get("/api/workspaces/{workspace_key}/data/discovery/schemas")
    async def list_data_schemas(
        workspace_key: str,
        sourceId: str | None = None,
    ) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        _validate_source_id(sourceId)
        return _native_data_json(
            _internal.data_list_schemas,
            binding.data_environment_json(),
        )

    @api.get("/api/workspaces/{workspace_key}/data/discovery/tables")
    async def list_data_tables(
        workspace_key: str,
        schema: str | None = None,
        sourceId: str | None = None,
    ) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        _validate_source_id(sourceId)
        return _native_data_json(
            _internal.data_list_tables,
            binding.data_environment_json(),
            schema,
        )

    @api.get("/api/workspaces/{workspace_key}/data/discovery/tables/{table_name}/columns")
    async def get_data_table_columns(
        workspace_key: str,
        table_name: str,
        schema: str | None = None,
        sourceId: str | None = None,
    ) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        _validate_source_id(sourceId)
        detail = _native_data_json(
            _internal.data_get_table_detail,
            binding.data_environment_json(),
            table_name,
            schema,
        )
        table = detail.get("table")
        return {
            "workspaceKey": workspace_key,
            "tableName": table_name,
            "columns": table.get("columns", []) if isinstance(table, dict) else [],
        }

    @api.get("/api/workspaces/{workspace_key}/data/discovery/tables/{table_name}/sample")
    async def sample_data_table(
        workspace_key: str,
        table_name: str,
        schema: str | None = None,
        sourceId: str | None = None,
        limit: int | None = None,
    ) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        _validate_source_id(sourceId)
        return _native_data_json(
            _internal.data_sample_table,
            binding.data_environment_json(),
            table_name,
            schema,
            limit,
        )

    @api.get("/api/workspaces/{workspace_key}/data/discovery/tables/{table_name}")
    async def get_data_table_detail(
        workspace_key: str,
        table_name: str,
        schema: str | None = None,
        sourceId: str | None = None,
    ) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        _validate_source_id(sourceId)
        detail = _native_data_json(
            _internal.data_get_table_detail,
            binding.data_environment_json(),
            table_name,
            schema,
        )
        return {
            "workspaceKey": workspace_key,
            **detail,
        }

    @api.post("/api/workspaces/{workspace_key}/data/profile/estimate")
    async def estimate_data_profile(workspace_key: str, request: Request) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        body = await _json_body(request)
        _validate_source_id(_string_or_none(body.get("sourceId")))
        return _native_data_json(
            _internal.data_profile_estimate,
            binding.data_environment_json(),
            _string_or_none(body.get("databaseId")) or "workspace-runtime",
            _string_or_none(body.get("schemaName") or body.get("schema")),
            json.dumps(_string_list(body.get("tables"))),
            _string_or_none(body.get("modelId") or body.get("model")),
            _positive_int_or_none(body.get("sampleSize")),
        )

    @api.post("/api/workspaces/{workspace_key}/data/profile/table")
    async def profile_data_table(workspace_key: str, request: Request) -> StreamingResponse:
        binding = _data_binding(app, workspace_key)
        body = await _json_body(request)
        _validate_source_id(_string_or_none(body.get("sourceId")))
        table_name = _required_body_string(body, "tableName")
        result = _native_data_json(
            _internal.data_profile_table,
            binding.data_environment_json(),
            _string_or_none(body.get("databaseId")) or "workspace-runtime",
            table_name,
            _string_or_none(body.get("schemaName") or body.get("schema")),
            _string_or_none(body.get("modelId") or body.get("model")),
            _positive_int_or_none(body.get("sampleSize")),
        )
        _persist_data_events(
            app=app,
            store=studio_store,
            workspace_key=workspace_key,
            run_id=str(result.get("jobId") or f"profile-table-{uuid4().hex}"),
            operation="data.profile.table",
            events=_events(result),
        )
        return StreamingResponse(
            _json_sse_events(_events(result)),
            media_type="text/event-stream",
            headers={"Cache-Control": "no-store", "X-Accel-Buffering": "no"},
        )

    @api.post("/api/workspaces/{workspace_key}/data/profile/database")
    async def profile_data_database(workspace_key: str, request: Request) -> StreamingResponse:
        binding = _data_binding(app, workspace_key)
        body = await _json_body(request)
        _validate_source_id(_string_or_none(body.get("sourceId")))
        result = _native_data_json(
            _internal.data_profile_database,
            binding.data_environment_json(),
            _string_or_none(body.get("databaseId")) or "workspace-runtime",
            _string_or_none(body.get("schemaName") or body.get("schema")),
            json.dumps(_string_list(body.get("tables"))),
            _string_or_none(body.get("modelId") or body.get("model")),
            _positive_int_or_none(body.get("sampleSize")),
        )
        _persist_data_events(
            app=app,
            store=studio_store,
            workspace_key=workspace_key,
            run_id=str(result.get("jobId") or f"profile-db-{uuid4().hex}"),
            operation="data.profile.database",
            events=_events(result),
        )
        return StreamingResponse(
            _json_sse_events(_events(result)),
            media_type="text/event-stream",
            headers={"Cache-Control": "no-store", "X-Accel-Buffering": "no"},
        )

    @api.post("/api/workspaces/{workspace_key}/data/knowledge/ingest")
    async def ingest_data_knowledge(workspace_key: str, request: Request) -> StreamingResponse:
        binding = _data_binding(app, workspace_key)
        body = await _json_body(request)
        source = body.get("source")
        if not isinstance(source, dict):
            raise StudioApiError(
                400,
                "data.knowledge.invalid_source",
                "knowledge ingest requires a source object",
            )
        result = _native_data_json(
            _internal.data_ingest_knowledge,
            binding.data_environment_json(),
            binding.runtime_spec.tenant.resource_id,
            _string_or_none(body.get("databaseId")) or "workspace-runtime",
            json.dumps(source),
            bool(body.get("extractKnowledge") or body.get("extract_knowledge")),
        )
        events = [_project_knowledge_ingest_event(event) for event in _events(result)]
        _persist_data_events(
            app=app,
            store=studio_store,
            workspace_key=workspace_key,
            run_id=str(result.get("jobId") or f"knowledge-ingest-{uuid4().hex}"),
            operation="data.knowledge.ingest",
            events=events,
        )
        return StreamingResponse(
            _json_sse_events(events),
            media_type="text/event-stream",
            headers={"Cache-Control": "no-store", "X-Accel-Buffering": "no"},
        )

    @api.get("/api/workspaces/{workspace_key}/data/knowledge/documents")
    async def list_knowledge_documents(workspace_key: str) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        return _native_data_json(
            _internal.data_list_knowledge_documents,
            binding.data_environment_json(),
            binding.runtime_spec.tenant.resource_id,
        )

    @api.get("/api/workspaces/{workspace_key}/data/knowledge/items")
    async def list_knowledge_items(
        workspace_key: str,
        sourceId: str | None = None,
    ) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        _validate_source_id(sourceId)
        return _native_data_json(
            _internal.data_list_knowledge_items,
            binding.data_environment_json(),
            binding.runtime_spec.tenant.resource_id,
        )

    @api.post("/api/workspaces/{workspace_key}/data/search")
    async def search_data_catalog(workspace_key: str, request: Request) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        body = await _json_body(request)
        _validate_source_id(_string_or_none(body.get("sourceId")))
        return _native_data_json(
            _internal.data_search_catalog,
            binding.data_environment_json(),
            json.dumps(
                {
                    "query": _required_body_string(body, "query"),
                    "mode": _string_or_none(body.get("mode")),
                    "limit": _positive_int_or_none(body.get("limit")),
                }
            ),
        )

    @api.get("/api/workspaces/{workspace_key}/tools")
    async def list_workspace_tools(workspace_key: str) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        return _native_data_json(
            _internal.data_list_tools,
            binding.data_environment_json(),
        )

    @api.post("/api/workspaces/{workspace_key}/tools/{tool_id}/execute")
    async def execute_workspace_tool(
        workspace_key: str,
        tool_id: str,
        request: Request,
    ) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        body = await _json_body(request)
        _validate_source_id(_string_or_none(body.get("sourceId")))
        input_payload = body.get("input")
        if input_payload is None:
            input_payload = body
        if not isinstance(input_payload, dict):
            raise StudioApiError(
                400,
                "tool.invalid_input",
                "tool execution input must be a JSON object",
            )
        return _native_data_json(
            _internal.data_execute_tool,
            binding.data_environment_json(),
            tool_id,
            json.dumps(input_payload),
        )

    @api.get("/api/workspaces/{workspace_key}/data/metrics")
    async def list_data_metrics(
        workspace_key: str,
        sourceId: str | None = None,
        query: str | None = None,
        limit: int | None = None,
    ) -> dict[str, Any]:
        binding = _data_binding(app, workspace_key)
        _validate_source_id(sourceId)
        return _native_data_json(
            _internal.data_list_metrics,
            binding.data_environment_json(),
            json.dumps(
                {
                    "query": query,
                    "limit": _positive_int_or_none(limit),
                }
            ),
        )

    @api.post("/api/workspaces/{workspace_key}/agents/{agent_id}/stream")
    async def stream_agent(
        workspace_key: str,
        agent_id: str,
        request: Request,
    ) -> StreamingResponse:
        binding = _binding(app, workspace_key)
        agent = _agent(binding, agent_id)
        if agent.role not in {"coordinator", "specialist"}:
            raise StudioApiError(
                status_code=409,
                code="agent.unsupported_entrypoint",
                message=(
                    f"Direct Studio chat for {agent.role} agent {agent_id!r} is not "
                    "supported until an explicit runtime API exists."
                ),
                details={"agentId": agent_id, "role": agent.role},
            )
        try:
            body = await request.json()
            if not isinstance(body, dict):
                raise ValueError("request body must be a JSON object")
            chat = normalize_chat_request(body)
        except ValueError as exc:
            raise StudioApiError(
                status_code=400,
                code="chat.invalid_request",
                message=str(exc),
            ) from exc

        return StreamingResponse(
            _stream_chat(
                app=app,
                binding=binding,
                agent=agent,
                prompt=chat.prompt,
                thread_id=chat.thread_id,
                run_id=chat.run_id,
                request=request,
                store=studio_store,
                legacy_messages=chat.legacy_messages,
                active_chat_runs=active_chat_runs,
            ),
            media_type="text/event-stream",
            headers={
                "Cache-Control": "no-store",
                "X-Accel-Buffering": "no",
            },
        )

    @api.post("/api/workspaces/{workspace_key}/runs/{run_id}/cancel")
    async def cancel_run(workspace_key: str, run_id: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        active_run = active_chat_runs.get((workspace_key, run_id))
        cancelled = False
        if active_run is not None:
            cancelled = _cancel_runtime_stream(active_run.stream)
            if not active_run.cancel_recorded:
                _record_chat_cancelled(
                    app=app,
                    store=studio_store,
                    workspace_key=workspace_key,
                    run_id=run_id,
                    active_run=active_run,
                )
        return {
            "workspaceKey": workspace_key,
            "runId": run_id,
            "status": "cancelled",
            "cancelled": cancelled,
        }

    @api.get("/api/workspaces/{workspace_key}/threads")
    async def list_threads(workspace_key: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        return {
            "workspaceKey": workspace_key,
            "threads": studio_store.list_threads(
                app_id=app.app_id,
                workspace_key=workspace_key,
            ),
        }

    @api.get("/api/workspaces/{workspace_key}/threads/{thread_id}")
    async def get_thread(workspace_key: str, thread_id: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        thread = _get_thread_or_404(
            studio_store,
            app_id=app.app_id,
            workspace_key=workspace_key,
            thread_id=thread_id,
        )
        return {"workspaceKey": workspace_key, "thread": thread}

    @api.get("/api/workspaces/{workspace_key}/threads/{thread_id}/messages")
    async def list_messages(workspace_key: str, thread_id: str) -> dict[str, Any]:
        binding = _binding(app, workspace_key)
        _get_thread_or_404(
            studio_store,
            app_id=app.app_id,
            workspace_key=workspace_key,
            thread_id=thread_id,
        )
        messages = studio_store.list_messages(
            app_id=app.app_id,
            workspace_key=workspace_key,
            thread_id=thread_id,
        )
        messages = _messages_with_rehydrated_tool_args(
            studio_store,
            app_id=app.app_id,
            workspace_key=workspace_key,
            messages=messages,
        )
        messages = _messages_with_rehydrated_eval_trace_parts(
            studio_store,
            app_id=app.app_id,
            workspace_key=workspace_key,
            messages=messages,
        )
        hidden_lifecycle_agent_names = _entrypoint_lifecycle_agent_names(binding)
        return {
            "workspaceKey": workspace_key,
            "threadId": thread_id,
            "messages": [
                _message_with_exposed_parts(
                    message,
                    hidden_lifecycle_agent_names=hidden_lifecycle_agent_names,
                )
                for message in messages
            ],
        }

    @api.delete("/api/workspaces/{workspace_key}/threads/{thread_id}")
    async def delete_thread(workspace_key: str, thread_id: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        try:
            studio_store.delete_thread(
                app_id=app.app_id,
                workspace_key=workspace_key,
                thread_id=thread_id,
            )
        except KeyError as exc:
            raise StudioApiError(
                status_code=404,
                code="thread.not_found",
                message=f"Thread {thread_id!r} was not found.",
                details={"threadId": thread_id, "workspaceKey": workspace_key},
            ) from exc
        return {"workspaceKey": workspace_key, "threadId": thread_id, "deleted": True}

    # ------------------------------------------------------------------
    # Tests (Studio tests API)
    # ------------------------------------------------------------------

    @api.get("/api/workspaces/{workspace_key}/tests")
    async def list_tests(workspace_key: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        return {
            "workspaceKey": workspace_key,
            "tests": studio_store.list_test_cases(
                app_id=app.app_id, workspace_key=workspace_key
            ),
        }

    @api.post("/api/workspaces/{workspace_key}/tests")
    async def create_test(workspace_key: str, request: Request) -> dict[str, Any]:
        _binding(app, workspace_key)
        body = await _json_object_body(request, code="test.invalid")
        test_case = _validate_test_case(body)
        saved = studio_store.upsert_test_case(
            app_id=app.app_id,
            workspace_key=workspace_key,
            test_case_id=test_case.id,
            payload=test_case.model_dump(by_alias=True, mode="json"),
        )
        return {"workspaceKey": workspace_key, "test": saved}

    @api.get("/api/workspaces/{workspace_key}/tests/tools")
    async def list_test_tools(workspace_key: str) -> dict[str, Any]:
        binding = _binding(app, workspace_key)
        return {"workspaceKey": workspace_key, "tools": _agent_scoped_tools(binding)}

    @api.get(
        "/api/workspaces/{workspace_key}/tests/builder/threads/{thread_id}/trace"
    )
    async def thread_trace(workspace_key: str, thread_id: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        _get_thread_or_404(
            studio_store, app_id=app.app_id, workspace_key=workspace_key, thread_id=thread_id
        )
        events = studio_store.list_thread_run_events(
            app_id=app.app_id, workspace_key=workspace_key, thread_id=thread_id
        )
        return {
            "workspaceKey": workspace_key,
            "threadId": thread_id,
            "trace": _extract_trace(events),
        }

    @api.post("/api/workspaces/{workspace_key}/tests/from-chat")
    async def test_from_chat(workspace_key: str, request: Request) -> dict[str, Any]:
        _binding(app, workspace_key)
        body = await _json_object_body(request, code="test.invalid")
        thread_id = _string_or_none(body.get("threadId"))
        if not thread_id:
            raise StudioApiError(
                status_code=400, code="test.invalid", message="threadId is required"
            )
        _get_thread_or_404(
            studio_store, app_id=app.app_id, workspace_key=workspace_key, thread_id=thread_id
        )
        events = studio_store.list_thread_run_events(
            app_id=app.app_id, workspace_key=workspace_key, thread_id=thread_id
        )
        trace = _extract_trace(events)
        input_text = (
            _string_or_none(body.get("input"))
            or _first_user_message(studio_store, app, workspace_key, thread_id)
            or ""
        )
        test_id = _string_or_none(body.get("id")) or f"tc-{uuid4().hex[:8]}"
        planned_actions = _extract_planned_actions(events)
        draft = define_test_case(
            test_id,
            input_text,
            expected_trajectory=trace["trajectory"],
            ground_truth=(
                define_expected_actions(
                    planned_actions=planned_actions, payload_match="subset"
                )
                if planned_actions
                else None
            ),
            source_thread_id=thread_id,
        )
        saved = studio_store.upsert_test_case(
            app_id=app.app_id,
            workspace_key=workspace_key,
            test_case_id=test_id,
            payload=draft.model_dump(by_alias=True, mode="json"),
        )
        return {"workspaceKey": workspace_key, "test": saved}

    @api.get("/api/workspaces/{workspace_key}/tests/{test_id}")
    async def get_test(workspace_key: str, test_id: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        return {
            "workspaceKey": workspace_key,
            "test": _get_test_or_404(studio_store, app, workspace_key, test_id),
        }

    @api.put("/api/workspaces/{workspace_key}/tests/{test_id}")
    async def update_test(
        workspace_key: str, test_id: str, request: Request
    ) -> dict[str, Any]:
        _binding(app, workspace_key)
        body = await _json_object_body(request, code="test.invalid")
        body.setdefault("id", test_id)
        test_case = _validate_test_case(body)
        if test_case.id != test_id:
            raise StudioApiError(
                status_code=400,
                code="test.id_mismatch",
                message=f"Test case id {test_case.id!r} does not match path id {test_id!r}.",
            )
        saved = studio_store.upsert_test_case(
            app_id=app.app_id,
            workspace_key=workspace_key,
            test_case_id=test_id,
            payload=test_case.model_dump(by_alias=True, mode="json"),
        )
        return {"workspaceKey": workspace_key, "test": saved}

    @api.delete("/api/workspaces/{workspace_key}/tests/{test_id}")
    async def delete_test(workspace_key: str, test_id: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        try:
            studio_store.delete_test_case(
                app_id=app.app_id, workspace_key=workspace_key, test_case_id=test_id
            )
        except KeyError as exc:
            raise StudioApiError(
                status_code=404,
                code="test.not_found",
                message=f"Test case {test_id!r} was not found.",
                details={"testId": test_id, "workspaceKey": workspace_key},
            ) from exc
        return {"workspaceKey": workspace_key, "testId": test_id, "deleted": True}

    @api.post("/api/workspaces/{workspace_key}/tests/{test_id}/validate")
    async def validate_test(
        workspace_key: str, test_id: str, request: Request
    ) -> dict[str, Any]:
        _binding(app, workspace_key)
        stored = _get_test_or_404(studio_store, app, workspace_key, test_id)
        body = await _json_object_body(request, code="test.invalid")
        sample = body.get("sample") if isinstance(body.get("sample"), dict) else body
        try:
            output = RawSampleOutput.model_validate(sample)
            scored = score_sample(
                stored["testCase"],
                output,
                scorer_preset=_string_or_none(body.get("scorerPreset")),
                mode=_string_or_none(body.get("mode")),
            )
        except (ValidationError, ValueError) as exc:
            raise StudioApiError(
                status_code=400,
                code="test.invalid",
                message=_validation_message(exc),
            ) from exc
        return {
            "workspaceKey": workspace_key,
            "testId": test_id,
            "scored": scored.model_dump(by_alias=True, mode="json"),
        }

    # ------------------------------------------------------------------
    # Evals (Studio evals API)
    # ------------------------------------------------------------------

    @api.get("/api/workspaces/{workspace_key}/evals")
    async def list_evals(workspace_key: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        evals = []
        for row in studio_store.list_eval_runs(
            app_id=app.app_id, workspace_key=workspace_key
        ):
            latest_summary = studio_store.latest_eval_summary(
                app_id=app.app_id, workspace_key=workspace_key, eval_id=row["id"]
            )
            evals.append(_eval_response(row, latest_summary=latest_summary))
        return {"workspaceKey": workspace_key, "evals": evals}

    @api.post("/api/workspaces/{workspace_key}/evals")
    async def create_eval(workspace_key: str, request: Request) -> dict[str, Any]:
        _binding(app, workspace_key)
        body = await _json_object_body(request, code="eval.invalid_config")
        definition, test_case_ids = _validate_eval_definition(body)
        eval_id = _string_or_none(body.get("id")) or f"eval-{uuid4().hex}"
        saved = studio_store.upsert_eval_run(
            app_id=app.app_id,
            workspace_key=workspace_key,
            eval_id=eval_id,
            config=definition,
            test_case_ids=test_case_ids,
            status="created",
        )
        return {"workspaceKey": workspace_key, "eval": _eval_response(saved)}

    @api.get("/api/workspaces/{workspace_key}/evals/compare")
    async def compare_evals(
        workspace_key: str, left: str, right: str
    ) -> dict[str, Any]:
        _binding(app, workspace_key)
        left_artifact = _get_artifact_or_404(studio_store, app, workspace_key, left)
        right_artifact = _get_artifact_or_404(studio_store, app, workspace_key, right)
        return {
            "workspaceKey": workspace_key,
            "comparison": _compare_artifacts(left_artifact, right_artifact),
        }

    @api.get("/api/workspaces/{workspace_key}/evals/{eval_id}")
    async def get_eval(workspace_key: str, eval_id: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        row = _get_eval_or_404(studio_store, app, workspace_key, eval_id)
        return {
            "workspaceKey": workspace_key,
            "eval": _eval_response(row),
            "runs": studio_store.list_eval_artifacts(
                app_id=app.app_id, workspace_key=workspace_key, eval_id=eval_id
            ),
        }

    @api.post("/api/workspaces/{workspace_key}/evals/{eval_id}/run")
    @api.post("/api/workspaces/{workspace_key}/evals/{eval_id}/rerun")
    async def run_eval(workspace_key: str, eval_id: str) -> dict[str, Any]:
        binding = _binding(app, workspace_key)
        row = _get_eval_or_404(studio_store, app, workspace_key, eval_id)
        request_model = _resolve_eval_request(
            studio_store, app, binding, workspace_key, row
        )
        runtime = _eval_runtime(binding)
        studio_store.update_eval_run_status(
            app_id=app.app_id, workspace_key=workspace_key, eval_id=eval_id, status="running"
        )
        try:
            artifact = await runtime.run_eval(request_model)
        except Exception as exc:  # noqa: BLE001 - map runner failure to API error.
            studio_store.update_eval_run_status(
                app_id=app.app_id, workspace_key=workspace_key, eval_id=eval_id, status="failed"
            )
            raise StudioApiError(
                status_code=400,
                code="eval.run_failed",
                message=str(exc),
            ) from exc
        run_id = str(artifact.get("runId") or f"eval-{uuid4().hex}")
        studio_store.append_eval_artifact(
            app_id=app.app_id,
            workspace_key=workspace_key,
            eval_id=eval_id,
            run_id=run_id,
            artifact=artifact,
        )
        _persist_eval_artifact_traces(
            store=studio_store,
            app=app,
            workspace_key=workspace_key,
            runtime=runtime,
            artifact=artifact,
        )
        _persist_eval_artifact_threads(
            store=studio_store,
            app=app,
            workspace_key=workspace_key,
            artifact=artifact,
        )
        studio_store.update_eval_run_status(
            app_id=app.app_id, workspace_key=workspace_key, eval_id=eval_id, status="completed"
        )
        return {
            "workspaceKey": workspace_key,
            "evalId": eval_id,
            "runId": run_id,
            "artifact": artifact,
        }

    @api.delete("/api/workspaces/{workspace_key}/evals/{eval_id}")
    async def delete_eval(workspace_key: str, eval_id: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        try:
            studio_store.delete_eval_run(
                app_id=app.app_id, workspace_key=workspace_key, eval_id=eval_id
            )
        except KeyError as exc:
            raise StudioApiError(
                status_code=404,
                code="eval.not_found",
                message=f"Eval {eval_id!r} was not found.",
                details={"evalId": eval_id, "workspaceKey": workspace_key},
            ) from exc
        return {"workspaceKey": workspace_key, "evalId": eval_id, "deleted": True}

    @api.post("/api/workspaces/{workspace_key}/evals/{eval_id}/cancel")
    async def cancel_eval(workspace_key: str, eval_id: str) -> dict[str, Any]:
        # Tier-1 cancel (Studio evals API): record cancellation intent. A live streaming
        # run stops cooperatively when its SSE consumer disconnects (see
        # ``_stream_eval``); first-class token cancellation lands in pause and resume support.
        _binding(app, workspace_key)
        _get_eval_or_404(studio_store, app, workspace_key, eval_id)
        studio_store.update_eval_run_status(
            app_id=app.app_id, workspace_key=workspace_key, eval_id=eval_id, status="cancelled"
        )
        return {"workspaceKey": workspace_key, "evalId": eval_id, "status": "cancelled"}

    @api.get("/api/workspaces/{workspace_key}/evals/{eval_id}/stream")
    async def stream_eval(workspace_key: str, eval_id: str, request: Request) -> StreamingResponse:
        binding = _binding(app, workspace_key)
        row = _get_eval_or_404(studio_store, app, workspace_key, eval_id)
        request_model = _resolve_eval_request(
            studio_store, app, binding, workspace_key, row
        )
        runtime = _eval_runtime(binding)
        return StreamingResponse(
            _stream_eval(
                app=app,
                store=studio_store,
                runtime=runtime,
                request=request,
                workspace_key=workspace_key,
                eval_id=eval_id,
                eval_request=request_model,
            ),
            media_type="text/event-stream",
        )

    # ------------------------------------------------------------------
    # Runs (runs inspection)
    # ------------------------------------------------------------------

    @api.get("/api/workspaces/{workspace_key}/runs")
    async def list_runs(workspace_key: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        return {
            "workspaceKey": workspace_key,
            "runs": studio_store.list_runs(
                app_id=app.app_id, workspace_key=workspace_key
            ),
        }

    @api.get("/api/workspaces/{workspace_key}/runs/{run_id}")
    async def get_run(workspace_key: str, run_id: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        try:
            run = studio_store.get_run(
                app_id=app.app_id, workspace_key=workspace_key, run_id=run_id
            )
        except KeyError as exc:
            raise StudioApiError(
                status_code=404,
                code="run.not_found",
                message=f"Run {run_id!r} was not found.",
                details={"runId": run_id, "workspaceKey": workspace_key},
            ) from exc
        return {"workspaceKey": workspace_key, "run": run}

    @api.get("/api/workspaces/{workspace_key}/runs/{run_id}/events")
    async def list_run_events(
        workspace_key: str, run_id: str, since_seq: int | None = None
    ) -> dict[str, Any]:
        _binding(app, workspace_key)
        # 404 for an unknown run so reconnect never silently returns empty.
        try:
            studio_store.get_run(
                app_id=app.app_id, workspace_key=workspace_key, run_id=run_id
            )
        except KeyError as exc:
            raise StudioApiError(
                status_code=404,
                code="run.not_found",
                message=f"Run {run_id!r} was not found.",
                details={"runId": run_id, "workspaceKey": workspace_key},
            ) from exc
        return {
            "workspaceKey": workspace_key,
            "runId": run_id,
            "events": studio_store.list_run_events(
                app_id=app.app_id,
                workspace_key=workspace_key,
                run_id=run_id,
                since_seq=since_seq,
            ),
        }

    # ------------------------------------------------------------------
    # Traces (runtime/eval trace persistence)
    # ------------------------------------------------------------------

    @api.get("/api/workspaces/{workspace_key}/traces")
    async def list_traces(
        workspace_key: str,
        evalRunId: str | None = None,
        testCaseId: str | None = None,
        threadId: str | None = None,
    ) -> dict[str, Any]:
        _binding(app, workspace_key)
        return {
            "workspaceKey": workspace_key,
            "traces": studio_store.list_traces(
                app_id=app.app_id,
                workspace_key=workspace_key,
                eval_run_id=_string_or_none(evalRunId),
                test_case_id=_string_or_none(testCaseId),
                thread_id=_string_or_none(threadId),
            ),
        }

    @api.get("/api/workspaces/{workspace_key}/traces/{trace_id}")
    async def get_trace(workspace_key: str, trace_id: str) -> dict[str, Any]:
        binding = _binding(app, workspace_key)
        try:
            trace = studio_store.get_trace(
                app_id=app.app_id,
                workspace_key=workspace_key,
                trace_id=trace_id,
            )
        except KeyError:
            runtime = binding.get_runtime()
            trace_payload = _runtime_trace(runtime, trace_id)
            if trace_payload is None:
                raise StudioApiError(
                    status_code=404,
                    code="trace.not_found",
                    message=f"Trace {trace_id!r} was not found.",
                    details={"traceId": trace_id, "workspaceKey": workspace_key},
                )
            trace = studio_store.upsert_trace(
                app_id=app.app_id,
                workspace_key=workspace_key,
                trace=trace_payload,
            )
        return {"workspaceKey": workspace_key, "trace": trace}

    # ------------------------------------------------------------------
    # Approval inbox (approval inbox)
    # ------------------------------------------------------------------

    @api.get("/api/workspaces/{workspace_key}/approvals")
    async def list_approvals(workspace_key: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        return {
            "workspaceKey": workspace_key,
            "approvals": studio_store.list_approvals(
                app_id=app.app_id, workspace_key=workspace_key
            ),
        }

    @api.get("/api/workspaces/{workspace_key}/approvals/{approval_id}")
    async def get_approval(workspace_key: str, approval_id: str) -> dict[str, Any]:
        _binding(app, workspace_key)
        try:
            approval = studio_store.get_approval(
                app_id=app.app_id, workspace_key=workspace_key, approval_id=approval_id
            )
        except KeyError as exc:
            raise StudioApiError(
                status_code=404,
                code="approval.not_found",
                message=f"Approval {approval_id!r} was not found.",
                details={"approvalId": approval_id, "workspaceKey": workspace_key},
            ) from exc
        return {"workspaceKey": workspace_key, "approval": approval}

    @api.post(
        "/api/workspaces/{workspace_key}/approvals/{approval_id}/respond",
        operation_id="respondToApproval",
    )
    async def respond_to_approval(
        workspace_key: str,
        approval_id: str,
        request: Request,
    ) -> dict[str, Any]:
        binding = _binding(app, workspace_key)
        try:
            body = await request.json()
            if not isinstance(body, dict):
                raise ValueError("request body must be a JSON object")
        except ValueError as exc:
            raise StudioApiError(
                400,
                "approval.invalid_request",
                str(exc),
            ) from exc
        outcome = body.get("outcome") or body.get("decision")
        if outcome == "approved":
            outcome = "approve"
        elif outcome == "rejected":
            outcome = "reject"
        if outcome not in {"approve", "reject", "revise"}:
            raise StudioApiError(
                400,
                "approval.invalid_outcome",
                "approval outcome must be approve, reject, or revise",
            )
        runtime = binding.get_runtime()
        await runtime.respond_to_approval(
            approval_id,
            outcome,
            feedback=body.get("feedback") or body.get("reason"),
            partial=body.get("partial"),
        )
        studio_store.update_approval_ref(
            app_id=app.app_id,
            workspace_key=workspace_key,
            approval_id=approval_id,
            status=str(outcome),
        )
        return {
            "workspaceKey": workspace_key,
            "approvalId": approval_id,
            "status": outcome,
        }

    @api.get("/api/runtime")
    async def default_runtime() -> dict[str, Any]:
        return app.default_binding().runtime_summary()

    @api.get("/api/agents")
    async def default_agents() -> dict[str, Any]:
        return app.default_binding().agents_response()

    @api.api_route(
        "/api/{api_path:path}",
        methods=["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD"],
        include_in_schema=False,
    )
    async def api_not_found(api_path: str) -> JSONResponse:
        return _error_response(
            404,
            code="route.not_found",
            message=f"API route /api/{api_path} was not found.",
        )

    @api.get("/{studio_path:path}", include_in_schema=False)
    async def studio_spa_fallback(studio_path: str) -> Response:
        if studio_path.startswith("api/"):
            return _error_response(
                404,
                code="route.not_found",
                message=f"API route /{studio_path} was not found.",
            )
        if not serve_studio:
            return _studio_static_unavailable_response(serve_studio=False)
        if _studio_static_file_exists(studio_path):
            return _studio_static_file_response(
                studio_path,
                cache_control="public, max-age=31536000, immutable",
            )
        if Path(studio_path).suffix:
            return _studio_static_file_response(
                studio_path,
                cache_control="public, max-age=31536000, immutable",
            )
        return _studio_index_response()

    return api


def create_studio_server(
    app: FlowAIApp,
    *,
    serve_studio: bool = True,
    store: StudioStore | None = None,
    store_path: str | Path | None = ".flowai/studio.db",
) -> FastAPI:
    """Backward-compatible alias for the FastAPI app factory."""

    return create_studio_app(
        app,
        serve_studio=serve_studio,
        store=store,
        store_path=store_path,
    )


def run_studio_server(
    app: FlowAIApp,
    *,
    host: str = "127.0.0.1",
    port: int = 4111,
    serve_studio: bool = True,
) -> None:
    """Run the local Harness Studio FastAPI app with Uvicorn."""

    api = create_studio_app(app, serve_studio=serve_studio)
    if not serve_studio:
        print("Studio static UI serving disabled; API routes remain available.")
    uvicorn.run(api, host=host, port=port)


def _studio_static_dir() -> Path:
    return Path(__file__).resolve().parent / "static"


def _studio_index_response() -> Response:
    index = _studio_static_dir() / "index.html"
    if not index.exists():
        return _studio_static_unavailable_response(serve_studio=True)
    return FileResponse(
        index,
        media_type="text/html; charset=utf-8",
        headers={"Cache-Control": "no-store"},
    )


def _studio_static_unavailable_response(*, serve_studio: bool) -> JSONResponse:
    message = (
        "Studio static assets are not packaged in this local server."
        if serve_studio
        else "Studio static UI serving is disabled; API routes remain available."
    )
    return JSONResponse(
        {
            "message": message,
            "api": "/api/status",
            "studioApiVersion": STUDIO_API_VERSION,
        },
        status_code=503,
        headers={"Cache-Control": "no-store"},
    )


def _studio_static_file_response(relative_path: str, *, cache_control: str) -> Response:
    path = _safe_studio_static_path(relative_path)
    if path is None or not path.is_file():
        return _error_response(
            404,
            code="static.not_found",
            message=f"Studio static asset /{relative_path} was not found.",
        )
    return FileResponse(path, headers={"Cache-Control": cache_control})


def _studio_static_file_exists(relative_path: str) -> bool:
    path = _safe_studio_static_path(relative_path)
    return path is not None and path.is_file()


def _safe_studio_static_path(relative_path: str) -> Path | None:
    if "\x00" in relative_path:
        return None
    normalized = relative_path.strip("/")
    if not normalized:
        return None
    root = _studio_static_dir().resolve()
    candidate = (root / normalized).resolve()
    try:
        candidate.relative_to(root)
    except ValueError:
        return None
    return candidate


def _binding(app: FlowAIApp, workspace_key: str) -> Any:
    try:
        return app.workspace(workspace_key)
    except KeyError as exc:
        raise StudioApiError(
            status_code=404,
            code="workspace.not_found",
            message=f"Workspace {workspace_key!r} was not found.",
            details={"workspaceKey": workspace_key},
        ) from exc


def _agent(binding: Any, agent_id: str) -> Any:
    for agent in binding.runtime_spec.agents:
        if agent.name == agent_id:
            return agent
    raise StudioApiError(
        status_code=404,
        code="agent.not_found",
        message=f"Agent {agent_id!r} was not found.",
        details={"agentId": agent_id, "workspaceKey": binding.workspace_key},
    )


def _data_binding(app: FlowAIApp, workspace_key: str) -> Any:
    binding = _binding(app, workspace_key)
    if not getattr(binding, "has_data_environment", False):
        raise StudioApiError(
            status_code=409,
            code="data.environment_missing",
            message=(
                f"Workspace {workspace_key!r} has no data_environment. "
                "Pass data_environment to define_workspace_runtime or define_app."
            ),
            details={"workspaceKey": workspace_key},
        )
    return binding


def _workspace_runtime_source(binding: Any) -> dict[str, Any]:
    env = binding.data_environment or {}
    target = env.get("targetDatabase") or env.get("target_database") or {}
    legacy_url = env.get("targetDatabaseUrl") or env.get("target_database_url")
    legacy_schema = env.get("targetDatabaseSchema") or env.get("target_database_schema")
    kind = target.get("kind") if isinstance(target, dict) else None
    url = target.get("url") if isinstance(target, dict) else None
    schema = target.get("schema") if isinstance(target, dict) else None
    database_type = _database_type(kind=kind, url=url or legacy_url)
    now = "1970-01-01T00:00:00Z"
    return {
        "id": "workspace-runtime",
        "sourceId": "workspace-runtime",
        "name": "Workspace runtime",
        "kind": "workspace-runtime",
        "status": "ready",
        "databaseType": database_type,
        "host": "workspace-runtime",
        "port": 0,
        "databaseName": _database_name(url or legacy_url),
        "schemaName": schema or legacy_schema or ("main" if database_type == "sqlite" else "public"),
        "encryptedCredentials": None,
        "isActive": True,
        "createdAt": now,
        "updatedAt": now,
        "metadata": {
            "readOnly": True,
            "resourceId": binding.resource_id,
            "workspaceKey": binding.workspace_key,
        },
    }


def _database_type(*, kind: Any, url: Any) -> str:
    if kind == "sqlite" or (isinstance(url, str) and url.startswith("sqlite:")):
        return "sqlite"
    if kind == "postgres" or (
        isinstance(url, str) and (url.startswith("postgres://") or url.startswith("postgresql://"))
    ):
        return "postgresql"
    return "postgresql"


def _database_name(url: Any) -> str:
    if not isinstance(url, str) or url == "":
        return "workspace-runtime"
    if url.startswith("sqlite:"):
        return Path(url.removeprefix("sqlite:")).name or "sqlite"
    return url.rsplit("/", maxsplit=1)[-1].split("?", maxsplit=1)[0] or "database"


def _validate_source_id(source_id: str | None) -> None:
    if source_id in {None, "", "workspace-runtime", "default"}:
        return
    raise StudioApiError(
        status_code=404,
        code="data.source_not_found",
        message=f"Data source {source_id!r} was not found.",
        details={"sourceId": source_id},
    )


async def _json_body(request: Request) -> dict[str, Any]:
    try:
        body = await request.json()
    except Exception as exc:  # noqa: BLE001 - request boundary.
        raise StudioApiError(
            400,
            "request.invalid_json",
            "request body must be valid JSON",
        ) from exc
    if not isinstance(body, dict):
        raise StudioApiError(
            400,
            "request.invalid_body",
            "request body must be a JSON object",
        )
    return body


def _native_data_json(function: Any, *args: Any) -> dict[str, Any]:
    try:
        output = function(*args)
        decoded = json.loads(output)
    except StudioApiError:
        raise
    except Exception as exc:  # noqa: BLE001 - native boundary.
        raise StudioApiError(
            status_code=409,
            code="data.command_failed",
            message=str(exc),
        ) from exc
    if not isinstance(decoded, dict):
        raise StudioApiError(
            500,
            "data.invalid_native_response",
            "native data command returned a non-object response",
        )
    return decoded


def _required_body_string(body: dict[str, Any], key: str) -> str:
    value = body.get(key)
    if isinstance(value, str) and value:
        return value
    raise StudioApiError(
        400,
        "request.invalid_body",
        f"request body requires non-empty {key}",
    )


def _string_or_none(value: Any) -> str | None:
    return value if isinstance(value, str) and value else None


def _string_list(value: Any) -> list[str]:
    if value is None:
        return []
    if isinstance(value, str):
        return [value] if value else []
    if isinstance(value, list):
        return [entry for entry in value if isinstance(entry, str) and entry]
    return []


def _positive_int_or_none(value: Any) -> int | None:
    if isinstance(value, int) and value > 0:
        return value
    return None


def _events(result: dict[str, Any]) -> list[dict[str, Any]]:
    events = result.get("events")
    if not isinstance(events, list):
        return []
    return [event for event in events if isinstance(event, dict)]


async def _json_sse_events(events: list[dict[str, Any]]) -> Any:
    for event in events:
        yield f"data: {json.dumps(event, sort_keys=True)}\n\n"


def _persist_data_events(
    *,
    app: FlowAIApp,
    store: StudioStore,
    workspace_key: str,
    run_id: str,
    operation: str,
    events: list[dict[str, Any]],
) -> None:
    recorder = _RunEventRecorder(
        app=app,
        store=store,
        workspace_key=workspace_key,
        run_id=run_id,
        thread_id=f"data:{operation}",
        operation=operation,
        agent_id="flowai-runtime-data",
    )
    for event in events:
        recorder.record(kind=str(event.get("type") or operation), payload=event, raw=event)


class _RunEventRecorder:
    """Persist normalized run events for one streaming operation (run-event capture).

    Shared by chat (projected runtime events), eval (lifecycle envelopes), and
    data (profile/knowledge events). Every event is tagged with the run's
    ``operation`` + agent id, and ``approval.required`` events are captured into
    the approval inbox.
    """

    def __init__(
        self,
        *,
        app: FlowAIApp,
        store: StudioStore,
        workspace_key: str,
        run_id: str,
        thread_id: str,
        operation: str,
        agent_id: str = "",
    ) -> None:
        self._app = app
        self._store = store
        self._workspace_key = workspace_key
        self._run_id = run_id
        self._thread_id = thread_id
        self._operation = operation
        self._agent_id = agent_id
        self.seq = 0

    def record(
        self,
        *,
        kind: str,
        payload: dict[str, Any],
        raw: dict[str, Any] | None,
        agent_id: str | None = None,
    ) -> dict[str, Any]:
        self.seq += 1
        agent = agent_id if agent_id is not None else self._agent_id
        event = studio_event(
            workspace_key=self._workspace_key,
            run_id=self._run_id,
            thread_id=self._thread_id,
            agent_id=agent,
            seq=self.seq,
            kind=kind,
            payload=payload,
        )
        self._store.append_run_event(
            app_id=self._app.app_id,
            workspace_key=self._workspace_key,
            thread_id=self._thread_id,
            run_id=self._run_id,
            seq=self.seq,
            kind=kind,
            event=event,
            raw_event=raw or {},
            operation=self._operation,
            agent_id=agent,
        )
        if kind == "approval.required":
            approval_id = payload.get("approvalId")
            if isinstance(approval_id, str) and approval_id:
                self._store.upsert_approval_ref(
                    app_id=self._app.app_id,
                    workspace_key=self._workspace_key,
                    thread_id=self._thread_id,
                    run_id=self._run_id,
                    approval_id=approval_id,
                    payload=payload,
                )
        return event


def _project_knowledge_ingest_event(event: dict[str, Any]) -> dict[str, Any]:
    event_type = event.get("type")
    if event_type == "discovered":
        return {"type": "discovered", "totalFiles": int(event.get("total") or 0)}
    if event_type == "ingesting":
        return {
            "type": "ingesting",
            "current": int(event.get("current") or 0),
            "total": int(event.get("total") or 0),
            "fileName": str(event.get("name") or ""),
        }
    if event_type == "completed":
        return {
            "type": "completed",
            "documentsIngested": int(event.get("new") or 0),
            "documentsSkipped": int(event.get("skippedDuplicate") or 0),
            "knowledgeItemsExtracted": 0,
        }
    if event_type == "error":
        return {"type": "error", "message": str(event.get("message") or "Unknown error")}
    return event


async def _json_object_body(request: Request, *, code: str) -> dict[str, Any]:
    try:
        body = await request.json()
    except ValueError as exc:
        raise StudioApiError(
            status_code=400, code=code, message="request body must be valid JSON"
        ) from exc
    if not isinstance(body, dict):
        raise StudioApiError(
            status_code=400, code=code, message="request body must be a JSON object"
        )
    return body


def _validation_message(exc: Exception) -> str:
    if isinstance(exc, ValidationError):
        errors = exc.errors()
        if errors:
            first = errors[0]
            loc = ".".join(str(part) for part in first.get("loc", ()))
            msg = first.get("msg", "invalid value")
            return f"{loc}: {msg}" if loc else str(msg)
    return str(exc)


# --- tests --------------------------------------------------------


def _validate_test_case(body: dict[str, Any]) -> EvalTestCase:
    try:
        return EvalTestCase.model_validate(body)
    except (ValidationError, ValueError) as exc:
        raise StudioApiError(
            status_code=400,
            code="test.invalid",
            message=_validation_message(exc),
        ) from exc


def _get_test_or_404(
    store: StudioStore,
    app: FlowAIApp,
    workspace_key: str,
    test_id: str,
) -> dict[str, Any]:
    try:
        return store.get_test_case(
            app_id=app.app_id, workspace_key=workspace_key, test_case_id=test_id
        )
    except KeyError as exc:
        raise StudioApiError(
            status_code=404,
            code="test.not_found",
            message=f"Test case {test_id!r} was not found.",
            details={"testId": test_id, "workspaceKey": workspace_key},
        ) from exc


def _first_user_message(
    store: StudioStore, app: FlowAIApp, workspace_key: str, thread_id: str
) -> str | None:
    for message in store.list_messages(
        app_id=app.app_id, workspace_key=workspace_key, thread_id=thread_id
    ):
        if message.get("role") == "user":
            return message.get("content")
    return None


def _messages_with_rehydrated_tool_args(
    store: StudioStore,
    *,
    app_id: str,
    workspace_key: str,
    messages: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    events_by_run_id: dict[str, list[dict[str, Any]]] = {}
    hydrated: list[dict[str, Any]] = []
    for message in messages:
        metadata = message.get("metadata") if isinstance(message.get("metadata"), dict) else {}
        run_id = _string_or_none(metadata.get("runId"))
        if message.get("role") != "assistant" or not run_id:
            hydrated.append(message)
            continue
        if run_id not in events_by_run_id:
            events_by_run_id[run_id] = store.list_run_events(
                app_id=app_id,
                workspace_key=workspace_key,
                run_id=run_id,
            )
        hydrated.append(_message_with_rehydrated_tool_args(message, events_by_run_id[run_id]))
    return hydrated


def _messages_with_rehydrated_eval_trace_parts(
    store: StudioStore,
    *,
    app_id: str,
    workspace_key: str,
    messages: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    hydrated: list[dict[str, Any]] = []
    for message in messages:
        hydrated.append(
            _message_with_rehydrated_eval_trace_parts(
                store,
                app_id=app_id,
                workspace_key=workspace_key,
                message=message,
            )
        )
    return hydrated


def _message_with_rehydrated_tool_args(
    message: dict[str, Any],
    events: list[dict[str, Any]],
) -> dict[str, Any]:
    metadata = message.get("metadata") if isinstance(message.get("metadata"), dict) else {}
    parts = metadata.get("parts")
    if not isinstance(parts, list):
        return message
    arguments_by_call_id = _tool_arguments_by_call_id(events)
    if not arguments_by_call_id:
        return message

    changed = False
    next_parts: list[Any] = []
    for part in parts:
        if not isinstance(part, dict) or part.get("type") != "tool-invocation":
            next_parts.append(part)
            continue
        tool_call_id = _string_or_none(part.get("toolCallId"))
        if (
            tool_call_id
            and tool_call_id in arguments_by_call_id
            and _tool_args_missing_or_empty(part.get("args"))
        ):
            next_parts.append({**part, "args": arguments_by_call_id[tool_call_id]})
            changed = True
        else:
            next_parts.append(part)

    if not changed:
        return message
    return {**message, "metadata": {**metadata, "parts": next_parts}}


def _message_with_rehydrated_eval_trace_parts(
    store: StudioStore,
    *,
    app_id: str,
    workspace_key: str,
    message: dict[str, Any],
) -> dict[str, Any]:
    if message.get("role") != "assistant":
        return message
    metadata = message.get("metadata") if isinstance(message.get("metadata"), dict) else {}
    if metadata.get("source") != "eval":
        return message
    if _message_has_tool_parts(message):
        return message
    trace = _eval_trace_for_message(
        store,
        app_id=app_id,
        workspace_key=workspace_key,
        message=message,
        metadata=metadata,
    )
    if trace is None:
        return message
    return _message_with_eval_trace_parts(message, trace)


def _message_has_tool_parts(message: dict[str, Any]) -> bool:
    metadata = message.get("metadata") if isinstance(message.get("metadata"), dict) else {}
    parts = message.get("parts")
    if not isinstance(parts, list):
        parts = metadata.get("parts")
    if not isinstance(parts, list):
        return False
    return any(isinstance(part, dict) and part.get("type") == "tool-invocation" for part in parts)


def _eval_trace_for_message(
    store: StudioStore,
    *,
    app_id: str,
    workspace_key: str,
    message: dict[str, Any],
    metadata: dict[str, Any],
) -> dict[str, Any] | None:
    run_id = _string_or_none(metadata.get("runId"))
    test_case_id = _string_or_none(metadata.get("testCaseId"))
    thread_id = _string_or_none(message.get("threadId"))
    sample_index = metadata.get("sampleIndex")
    sample_index_value = sample_index if isinstance(sample_index, int) else None
    return _eval_trace_for_sample(
        store,
        app_id=app_id,
        workspace_key=workspace_key,
        run_id=run_id,
        test_case_id=test_case_id,
        thread_id=thread_id,
        sample_index=sample_index_value,
    )


def _eval_trace_for_sample(
    store: StudioStore,
    *,
    app_id: str,
    workspace_key: str,
    run_id: str | None,
    test_case_id: str | None,
    thread_id: str | None,
    sample_index: int | None,
) -> dict[str, Any] | None:
    if not run_id:
        return None
    traces = store.list_traces(
        app_id=app_id,
        workspace_key=workspace_key,
        eval_run_id=run_id,
        test_case_id=test_case_id,
        thread_id=thread_id,
    )
    for row in traces:
        if sample_index is not None and row.get("sampleIndex") != sample_index:
            continue
        trace = row.get("trace")
        if isinstance(trace, dict):
            return trace
    return None


def _metadata_with_eval_trace_parts(
    metadata: dict[str, Any], response_text: str | None, trace: dict[str, Any]
) -> dict[str, Any]:
    parts = _assistant_parts_from_eval_trace(trace, response_text)
    if not parts:
        return metadata
    next_metadata = {**metadata, "parts": parts}
    trace_id = _string_or_none(trace.get("traceId"))
    if trace_id:
        next_metadata["traceId"] = trace_id
    return next_metadata


def _message_with_eval_trace_parts(message: dict[str, Any], trace: dict[str, Any]) -> dict[str, Any]:
    metadata = message.get("metadata") if isinstance(message.get("metadata"), dict) else {}
    response_text = _string_or_none(message.get("content"))
    next_metadata = _metadata_with_eval_trace_parts(metadata, response_text, trace)
    if next_metadata is metadata:
        return message
    return {**message, "metadata": next_metadata}


def _entrypoint_lifecycle_agent_names(binding: Any) -> set[str]:
    names: set[str] = set()
    for agent in getattr(binding.runtime_spec, "agents", []):
        if getattr(agent, "entrypoint", False) or getattr(agent, "role", None) == "coordinator":
            names.add(str(agent.name))
    return names


def _is_hidden_agent_lifecycle_part(
    part: Any, hidden_lifecycle_agent_names: set[str]
) -> bool:
    return (
        isinstance(part, dict)
        and part.get("type") == "tool-agent"
        and part.get("agentName") in hidden_lifecycle_agent_names
    )


def _message_parts_without_hidden_lifecycle_agents(
    parts: list[Any], hidden_lifecycle_agent_names: set[str]
) -> list[Any]:
    if not hidden_lifecycle_agent_names:
        return parts
    return [
        part
        for part in parts
        if not _is_hidden_agent_lifecycle_part(part, hidden_lifecycle_agent_names)
    ]


def _message_with_exposed_parts(
    message: dict[str, Any],
    *,
    hidden_lifecycle_agent_names: set[str] | None = None,
) -> dict[str, Any]:
    hidden_names = hidden_lifecycle_agent_names or set()
    metadata = message.get("metadata") if isinstance(message.get("metadata"), dict) else {}
    parts = metadata.get("parts")
    next_message = message
    if isinstance(parts, list):
        visible_parts = _message_parts_without_hidden_lifecycle_agents(parts, hidden_names)
        if visible_parts is not parts and len(visible_parts) != len(parts):
            metadata = {**metadata, "parts": visible_parts}
            next_message = {**next_message, "metadata": metadata}
        if "parts" not in next_message:
            next_message = {**next_message, "parts": visible_parts}
    exposed_parts = next_message.get("parts")
    if isinstance(exposed_parts, list):
        visible_parts = _message_parts_without_hidden_lifecycle_agents(exposed_parts, hidden_names)
        if len(visible_parts) != len(exposed_parts):
            next_message = {**next_message, "parts": visible_parts}
    return next_message


def _tool_arguments_by_call_id(events: list[dict[str, Any]]) -> dict[str, Any]:
    arguments_by_call_id: dict[str, Any] = {}
    for event in events:
        if event.get("kind") != "tool.call.started":
            continue
        payload = _stored_event_payload(event)
        tool_call_id = _event_tool_call_id(payload)
        if tool_call_id:
            arguments_by_call_id[tool_call_id] = payload.get("arguments", {})
    return arguments_by_call_id


def _stored_event_payload(event: dict[str, Any]) -> dict[str, Any]:
    payload = event.get("payload")
    if isinstance(payload, dict):
        return payload
    envelope = event.get("event")
    if isinstance(envelope, dict) and isinstance(envelope.get("payload"), dict):
        return envelope["payload"]
    return {}


def _tool_args_missing_or_empty(value: Any) -> bool:
    return value is None or (isinstance(value, dict) and len(value) == 0)


def _assistant_parts_from_eval_trace(
    trace: dict[str, Any], response_text: str | None
) -> list[dict[str, Any]]:
    parts = _tool_invocation_parts_from_trace(trace)
    if response_text:
        parts.append({"type": "text", "text": response_text})
    return parts


def _tool_invocation_parts_from_trace(trace: dict[str, Any]) -> list[dict[str, Any]]:
    raw_steps = trace.get("steps")
    if not isinstance(raw_steps, list):
        return []
    indexed_steps = [
        (index, step) for index, step in enumerate(raw_steps) if isinstance(step, dict)
    ]
    indexed_steps.sort(
        key=lambda item: item[1].get("ordinal") if isinstance(item[1].get("ordinal"), int) else item[0]
    )
    parts: list[dict[str, Any]] = []
    for index, step in indexed_steps:
        tool_name = _string_or_none(step.get("toolName")) or _string_or_none(
            step.get("tool_name")
        )
        if not tool_name:
            continue
        tool_call_id = (
            _string_or_none(step.get("toolCallId"))
            or _string_or_none(step.get("tool_call_id"))
            or _string_or_none(step.get("correlationId"))
            or _string_or_none(step.get("correlation_id"))
            or f"{trace.get('traceId') or 'trace'}:{index}"
        )
        part: dict[str, Any] = {
            "type": "tool-invocation",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "args": _trace_payload_value(step.get("arguments")) or {},
            "state": "result",
        }
        if "result" in step and step.get("result") is not None:
            part["result"] = _trace_payload_value(step.get("result"))
        elif _string_or_none(step.get("error")):
            part["result"] = {"error": _string_or_none(step.get("error"))}
        parts.append(part)
    return parts


def _trace_payload_value(payload: Any) -> Any:
    if not isinstance(payload, dict):
        return payload
    kind = payload.get("kind")
    if kind == "inline":
        return payload.get("value")
    if kind == "omitted":
        return {"kind": "omitted", "reason": payload.get("reason")}
    if kind == "redacted":
        return {"kind": "redacted", "redaction": payload.get("redaction")}
    return payload


def _extract_trace(events: list[dict[str, Any]]) -> dict[str, Any]:
    """Project persisted run events into a test-builder trace (test-builder trace extraction).

    Tool trajectory and tool/sub-agent calls are reconstructed from the
    ``tool.call.*`` / ``sub_agent.call.*`` events captured during a run.
    """

    trajectory: list[str] = []
    seen_tool_calls: set[str] = set()
    tool_calls: dict[str, dict[str, Any]] = {}
    sub_agent_calls: list[dict[str, Any]] = []
    for event in events:
        kind = event.get("kind")
        payload = (event.get("event") or {}).get("payload") or {}
        if kind in {"tool.call.started", "tool.call.completed"}:
            tool_call_id = str(payload.get("toolCallId") or "")
            tool_name = str(payload.get("toolName") or "")
            if tool_name and (tool_call_id == "" or tool_call_id not in seen_tool_calls):
                trajectory.append(tool_name)
                if tool_call_id:
                    seen_tool_calls.add(tool_call_id)
            key = tool_call_id or f"_anon-{len(tool_calls)}"
            call = tool_calls.setdefault(
                key, {"toolCallId": tool_call_id, "toolName": tool_name}
            )
            if kind == "tool.call.started":
                call["arguments"] = payload.get("arguments")
            else:
                call["status"] = "completed"
                call["result"] = payload.get("result")
        elif kind in {"sub_agent.call.started", "sub_agent.call.completed"}:
            sub_agent_calls.append(
                {
                    "targetAgentId": payload.get("targetAgentId"),
                    "kind": kind,
                    "result": payload.get("result"),
                }
            )
    return {
        "trajectory": trajectory,
        "toolCalls": list(tool_calls.values()),
        "subAgentCalls": sub_agent_calls,
        "resolvedActions": [],
    }


def _flatten_action_seq(seq: Any) -> list[dict[str, Any]]:
    """Flatten the runtime's ActionSeq (``{head, tail}``) or a plain list into
    ``{"type", "payload"}`` expected-action mappings."""

    if isinstance(seq, Mapping) and seq.get("head") is not None:
        items: list[Any] = [seq["head"], *(seq.get("tail") or [])]
    elif isinstance(seq, list):
        items = seq
    else:
        return []
    actions: list[dict[str, Any]] = []
    for item in items:
        if not isinstance(item, Mapping) or not isinstance(item.get("kind"), str):
            continue
        payload = item.get("payload")
        actions.append(
            {
                "type": item["kind"],
                "payload": dict(payload) if isinstance(payload, Mapping) else {},
            }
        )
    return actions


def _extract_planned_actions(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Extract planned-action ground truth from a thread's stored plan.

    Prefers the validated plan returned by ``storePlan`` (``tool.call.completed``
    result), with the plan body carried by ``approval.required`` events as a
    fallback. When a thread stores several plans, the last one wins.
    """

    actions: list[dict[str, Any]] = []
    for event in events:
        kind = event.get("kind")
        payload = (event.get("event") or {}).get("payload") or {}
        candidate: Any = None
        if kind == "tool.call.completed" and payload.get("toolName") == "storePlan":
            result = payload.get("result")
            if isinstance(result, Mapping):
                candidate = result.get("actions")
        elif kind == "approval.required":
            raw = payload.get("raw")
            if isinstance(raw, Mapping) and raw.get("kind") == "plan":
                plan_body = raw.get("payload")
                if isinstance(plan_body, Mapping):
                    candidate = plan_body.get("actions")
        flattened = _flatten_action_seq(candidate)
        if flattened:
            actions = flattened
    return actions


# Role-default tool names mirrored from flowai-runtime ``role_default_handlers``
# (crates/flowai-runtime/src/toolkits.rs). These are composed at runtime from the
# agent's role and are intentionally not present in ``agent.toolkits``. Coordinator
# routing is surfaced separately via ``call_agent``.
_ROLE_DEFAULT_TOOL_NAMES: dict[str, tuple[str, ...]] = {
    "planner": ("storePlan", "getPlan"),
    "executor": ("getPlan", "executePlan", "resolveRef", "glimpseRef"),
}


def _agent_scoped_tools(binding: Any) -> list[dict[str, Any]]:
    """Aggregate the tool inventory across the binding's agents.

    Surfaces the tool names that can appear in a trajectory: each agent's
    explicit tools, the routing tool (``call_agent``) for agents that delegate
    to sub-agents, and toolkit-derived tools where the toolkit can describe them
    (best-effort; the runtime-generated ``agents`` toolkit is represented by the
    ``call_agent`` entry).
    """

    spec = binding.runtime_spec
    toolkit_by_id = {tk.id: tk for tk in getattr(spec, "toolkits", [])}
    tools: dict[str, dict[str, Any]] = {}

    def _add(name: str, agent_name: str) -> None:
        if not name:
            return
        entry = tools.setdefault(name, {"name": name, "agents": []})
        if agent_name not in entry["agents"]:
            entry["agents"].append(agent_name)

    for agent in spec.agents:
        for tool in agent.tools:
            _add(tool.binding_id or tool.name, agent.name)
        # Agents with routes invoke sub-agents through the generated call_agent.
        if getattr(agent, "routes", None):
            _add("call_agent", agent.name)
        # Role-default tools the runtime composes without serializing them into
        # ``toolkits`` (flowai-runtime ``role_default_handlers``). They appear in
        # trajectories, so the catalog must surface them or steps like
        # ``storePlan`` validate as Unknown.
        for name in _ROLE_DEFAULT_TOOL_NAMES.get(str(getattr(agent, "role", "")), ()):
            _add(name, agent.name)
        # Toolkit-derived tools (best-effort; skip toolkits that can't describe).
        for toolkit_id in getattr(agent, "toolkits", []):
            toolkit = toolkit_by_id.get(toolkit_id)
            if toolkit is None:
                continue
            try:
                definitions = json.loads(
                    _internal.describe_toolkit_tools(
                        json.dumps(toolkit.model_dump(by_alias=True, mode="json")),
                        json.dumps(agent.model_dump(by_alias=True, mode="json")),
                    )
                )
            except Exception:  # noqa: BLE001 - toolkit may not be describable.
                continue
            for definition in definitions:
                if isinstance(definition, dict) and definition.get("name"):
                    _add(str(definition["name"]), agent.name)

    return sorted(tools.values(), key=lambda entry: entry["name"])


# --- evals --------------------------------------------------------------


def _validate_eval_definition(body: dict[str, Any]) -> tuple[dict[str, Any], list[str]]:
    config_body = body.get("config")
    if not isinstance(config_body, dict):
        config_body = {}
    try:
        config = EvalConfig.model_validate(config_body)
    except (ValidationError, ValueError) as exc:
        raise StudioApiError(
            status_code=400,
            code="eval.invalid_config",
            message=_validation_message(exc),
        ) from exc
    definition = {
        "config": config.model_dump(by_alias=True, mode="json"),
        "scorerPreset": _string_or_none(body.get("scorerPreset")),
    }
    return definition, _string_list(body.get("testCaseIds"))


def _eval_response(
    row: dict[str, Any], latest_summary: dict[str, Any] | None = None
) -> dict[str, Any]:
    definition = row.get("config") or {}
    return {
        "id": row["id"],
        "config": definition.get("config", {}),
        "scorerPreset": definition.get("scorerPreset"),
        "testCaseIds": row.get("testCaseIds", []),
        "status": row.get("status"),
        "latestSummary": latest_summary,
        "createdAt": row.get("createdAt"),
        "updatedAt": row.get("updatedAt"),
    }


def _get_eval_or_404(
    store: StudioStore,
    app: FlowAIApp,
    workspace_key: str,
    eval_id: str,
) -> dict[str, Any]:
    try:
        return store.get_eval_run(
            app_id=app.app_id, workspace_key=workspace_key, eval_id=eval_id
        )
    except KeyError as exc:
        raise StudioApiError(
            status_code=404,
            code="eval.not_found",
            message=f"Eval {eval_id!r} was not found.",
            details={"evalId": eval_id, "workspaceKey": workspace_key},
        ) from exc


def _get_artifact_or_404(
    store: StudioStore,
    app: FlowAIApp,
    workspace_key: str,
    run_id: str,
) -> dict[str, Any]:
    try:
        return store.get_eval_artifact_by_run(
            app_id=app.app_id, workspace_key=workspace_key, run_id=run_id
        )
    except KeyError as exc:
        raise StudioApiError(
            status_code=404,
            code="eval.run_not_found",
            message=f"Eval run {run_id!r} was not found.",
            details={"runId": run_id, "workspaceKey": workspace_key},
        ) from exc


def _eval_runtime(binding: Any) -> Any:
    try:
        return binding.get_runtime()
    except Exception as exc:  # noqa: BLE001 - surface as a structured API error.
        raise StudioApiError(
            status_code=409,
            code="eval.no_runtime_binding",
            message=f"Workspace {binding.workspace_key!r} has no runnable runtime: {exc}",
            details={"workspaceKey": binding.workspace_key},
        ) from exc


def _resolve_eval_request(
    store: StudioStore,
    app: FlowAIApp,
    binding: Any,
    workspace_key: str,
    row: dict[str, Any],
) -> EvalRequest:
    definition = row.get("config") or {}
    config_wire = definition.get("config") or {}
    _validate_eval_target_agent(binding, config_wire)
    scorer_preset = definition.get("scorerPreset")
    test_cases: list[dict[str, Any]] = []
    for test_case_id in row.get("testCaseIds", []):
        try:
            stored = store.get_test_case(
                app_id=app.app_id, workspace_key=workspace_key, test_case_id=test_case_id
            )
        except KeyError as exc:
            raise StudioApiError(
                status_code=400,
                code="eval.unknown_test_case",
                message=f"Test case {test_case_id!r} is not registered.",
                details={"testCaseId": test_case_id},
            ) from exc
        test_cases.append(stored["testCase"])
    if not test_cases:
        raise StudioApiError(
            status_code=400,
            code="eval.no_test_cases",
            message="Eval has no test cases to run.",
        )
    payload: dict[str, Any] = {
        "tenantId": binding.resource_id,
        "workspaceId": workspace_key,
        "config": config_wire,
        "testCases": test_cases,
    }
    if scorer_preset:
        payload["scorerPreset"] = scorer_preset
    try:
        return EvalRequest.model_validate(payload)
    except (ValidationError, ValueError) as exc:
        raise StudioApiError(
            status_code=400,
            code="eval.invalid_config",
            message=_validation_message(exc),
        ) from exc


def _validate_eval_target_agent(binding: Any, config_wire: Mapping[str, Any]) -> None:
    mode = _string_or_none(config_wire.get("mode"))
    if mode != "specialist":
        return

    target_agent_id = _string_or_none(config_wire.get("targetAgentId"))
    if not target_agent_id:
        raise StudioApiError(
            status_code=400,
            code="eval.invalid_target_agent",
            message="Specialist eval mode requires targetAgentId.",
            details={"mode": mode, "workspaceKey": binding.workspace_key},
        )

    target_agent = next(
        (agent for agent in binding.runtime_spec.agents if agent.name == target_agent_id),
        None,
    )
    if target_agent is None or target_agent.role != "specialist":
        raise StudioApiError(
            status_code=400,
            code="eval.invalid_target_agent",
            message=f"Specialist eval target {target_agent_id!r} is not a registered specialist.",
            details={
                "mode": mode,
                "targetAgentId": target_agent_id,
                "workspaceKey": binding.workspace_key,
            },
        )


def _trace_ids_from_artifact(artifact: dict[str, Any]) -> list[str]:
    trace_ids: list[str] = []
    seen: set[str] = set()
    for case in artifact.get("testCases", []) or []:
        if not isinstance(case, dict):
            continue
        for sample in case.get("samples", []) or []:
            if not isinstance(sample, dict):
                continue
            trace = sample.get("trace")
            if not isinstance(trace, dict):
                continue
            trace_id = _string_or_none(trace.get("traceId"))
            if trace_id and trace_id not in seen:
                seen.add(trace_id)
                trace_ids.append(trace_id)
    return trace_ids


def _runtime_trace(runtime: Any, trace_id: str) -> dict[str, Any] | None:
    getter = getattr(runtime, "get_trace", None)
    if not callable(getter):
        return None
    try:
        trace = getter(trace_id)
    except Exception:  # noqa: BLE001 - trace persistence is best-effort here.
        return None
    return trace if isinstance(trace, dict) else None


def _persist_eval_artifact_traces(
    *,
    store: StudioStore,
    app: FlowAIApp,
    workspace_key: str,
    runtime: Any,
    artifact: dict[str, Any],
) -> None:
    for trace_id in _trace_ids_from_artifact(artifact):
        trace = _runtime_trace(runtime, trace_id)
        if trace is None:
            continue
        try:
            store.upsert_trace(app_id=app.app_id, workspace_key=workspace_key, trace=trace)
        except ValueError:
            continue


def _persist_eval_artifact_threads(
    *,
    store: StudioStore,
    app: FlowAIApp,
    workspace_key: str,
    artifact: dict[str, Any],
) -> None:
    run_id = _string_or_none(artifact.get("runId"))
    for case in artifact.get("testCases", []) or []:
        if not isinstance(case, dict):
            continue
        test_case_id = _string_or_none(case.get("testCaseId"))
        prompt = _string_or_none(case.get("input"))
        for sample in case.get("samples", []) or []:
            if not isinstance(sample, dict):
                continue
            thread_id = _string_or_none(sample.get("threadId"))
            response_text = _string_or_none(sample.get("responseText"))
            if not thread_id or not (prompt or response_text):
                continue
            sample_index = sample.get("sampleIndex")
            sample_index_value = sample_index if isinstance(sample_index, int) else None
            title = _thread_title(prompt or f"Eval sample {sample_index_value or 0}")
            store.upsert_thread(
                app_id=app.app_id,
                workspace_key=workspace_key,
                thread_id=thread_id,
                title=title,
            )
            if store.list_messages(
                app_id=app.app_id,
                workspace_key=workspace_key,
                thread_id=thread_id,
            ):
                continue
            metadata = {
                "source": "eval",
                "runId": run_id,
                "testCaseId": test_case_id,
                "sampleIndex": sample_index_value,
            }
            if prompt:
                store.append_message(
                    app_id=app.app_id,
                    workspace_key=workspace_key,
                    thread_id=thread_id,
                    role="user",
                    content=prompt,
                    metadata=metadata,
                )
            if response_text:
                trace = _eval_trace_for_sample(
                    store,
                    app_id=app.app_id,
                    workspace_key=workspace_key,
                    run_id=run_id,
                    test_case_id=test_case_id,
                    thread_id=thread_id,
                    sample_index=sample_index_value,
                )
                assistant_metadata = (
                    _metadata_with_eval_trace_parts(metadata, response_text, trace)
                    if trace is not None
                    else metadata
                )
                store.append_message(
                    app_id=app.app_id,
                    workspace_key=workspace_key,
                    thread_id=thread_id,
                    role="assistant",
                    content=response_text,
                    metadata=assistant_metadata,
                )


def _compare_artifacts(left: dict[str, Any], right: dict[str, Any]) -> dict[str, Any]:
    def _aggregates(artifact: dict[str, Any]) -> dict[str, float]:
        result: dict[str, float] = {}
        for case in artifact.get("testCases", []) or []:
            if isinstance(case, dict):
                result[str(case.get("testCaseId"))] = float(case.get("aggregateScore") or 0.0)
        return result

    left_artifact = left.get("artifact") or {}
    right_artifact = right.get("artifact") or {}
    left_scores = _aggregates(left_artifact)
    right_scores = _aggregates(right_artifact)
    test_cases = [
        {
            "testCaseId": test_case_id,
            "left": left_scores.get(test_case_id),
            "right": right_scores.get(test_case_id),
            "delta": (right_scores.get(test_case_id) or 0.0)
            - (left_scores.get(test_case_id) or 0.0),
        }
        for test_case_id in sorted(set(left_scores) | set(right_scores))
    ]
    return {
        "left": {
            "runId": left.get("runId"),
            "evalId": left.get("evalId"),
            "summary": left_artifact.get("summary"),
        },
        "right": {
            "runId": right.get("runId"),
            "evalId": right.get("evalId"),
            "summary": right_artifact.get("summary"),
        },
        "testCases": test_cases,
    }


def _lean_eval_event_payload(data: dict[str, Any]) -> dict[str, Any]:
    """Trim the full artifact out of a persisted eval run event.

    ``evalStarted`` / ``evalCompleted`` embed the whole artifact, which already
    lives in ``eval_artifacts``; keep only a run/summary reference so the run
    event timeline stays lean.
    """

    artifact = data.get("artifact")
    if not isinstance(artifact, dict):
        return data
    lean = {key: value for key, value in data.items() if key != "artifact"}
    lean["artifact"] = {"runId": artifact.get("runId"), "summary": artifact.get("summary")}
    return lean


def _eval_sse(envelope: dict[str, Any]) -> str:
    event_type = str(envelope.get("type") or "message")
    return f"event: {event_type}\ndata: {json.dumps(envelope, sort_keys=True)}\n\n"


async def _stream_eval(
    *,
    app: FlowAIApp,
    store: StudioStore,
    runtime: Any,
    request: Request,
    workspace_key: str,
    eval_id: str,
    eval_request: EvalRequest,
) -> Any:
    store.update_eval_run_status(
        app_id=app.app_id, workspace_key=workspace_key, eval_id=eval_id, status="running"
    )
    final_artifact: dict[str, Any] | None = None
    run_id: str | None = None
    recorder: _RunEventRecorder | None = None
    thread_id = f"eval:{eval_id}"
    try:
        async for envelope in runtime.stream_eval(eval_request):
            if await request.is_disconnected():
                break
            if isinstance(envelope, dict):
                run_id = str(envelope.get("runId") or run_id or f"eval-{eval_id}")
                if recorder is None:
                    recorder = _RunEventRecorder(
                        app=app,
                        store=store,
                        workspace_key=workspace_key,
                        run_id=run_id,
                        thread_id=thread_id,
                        operation="eval",
                    )
                event_type = str(envelope.get("type") or "message")
                data = envelope.get("data") or {}
                if event_type == "evalCompleted" and isinstance(data.get("artifact"), dict):
                    final_artifact = data["artifact"]
                # Persist a lean run event (the full artifact lives in eval_artifacts).
                recorder.record(
                    kind=event_type, payload=_lean_eval_event_payload(data), raw=None
                )
                yield _eval_sse(envelope)
    except asyncio.CancelledError:
        store.update_eval_run_status(
            app_id=app.app_id, workspace_key=workspace_key, eval_id=eval_id, status="cancelled"
        )
        raise
    except Exception as exc:  # noqa: BLE001 - stream boundary.
        store.update_eval_run_status(
            app_id=app.app_id, workspace_key=workspace_key, eval_id=eval_id, status="failed"
        )
        if recorder is not None:
            recorder.record(kind="evalFailed", payload={"error": str(exc)}, raw=None)
        yield _eval_sse(
            {"runId": run_id or eval_id, "sequence": -1, "type": "evalFailed", "data": {"error": str(exc)}}
        )
        return
    if final_artifact is not None:
        resolved_run_id = str(final_artifact.get("runId") or run_id or f"eval-{uuid4().hex}")
        store.append_eval_artifact(
            app_id=app.app_id,
            workspace_key=workspace_key,
            eval_id=eval_id,
            run_id=resolved_run_id,
            artifact=final_artifact,
        )
        _persist_eval_artifact_traces(
            store=store,
            app=app,
            workspace_key=workspace_key,
            runtime=runtime,
            artifact=final_artifact,
        )
        _persist_eval_artifact_threads(
            store=store,
            app=app,
            workspace_key=workspace_key,
            artifact=final_artifact,
        )
        store.update_eval_run_status(
            app_id=app.app_id, workspace_key=workspace_key, eval_id=eval_id, status="completed"
        )
    else:
        store.update_eval_run_status(
            app_id=app.app_id, workspace_key=workspace_key, eval_id=eval_id, status="cancelled"
        )


def _get_thread_or_404(
    store: StudioStore,
    *,
    app_id: str,
    workspace_key: str,
    thread_id: str,
) -> dict[str, Any]:
    try:
        return store.get_thread(
            app_id=app_id,
            workspace_key=workspace_key,
            thread_id=thread_id,
        )
    except KeyError as exc:
        raise StudioApiError(
            status_code=404,
            code="thread.not_found",
            message=f"Thread {thread_id!r} was not found.",
            details={"threadId": thread_id, "workspaceKey": workspace_key},
        ) from exc


async def _stream_chat(
    *,
    app: FlowAIApp,
    binding: Any,
    agent: Any,
    prompt: str,
    thread_id: str,
    run_id: str,
    request: Request,
    store: StudioStore,
    legacy_messages: bool,
    active_chat_runs: dict[tuple[str, str], _ActiveChatRun],
) -> Any:
    workspace_key = binding.workspace_key
    store.upsert_thread(
        app_id=app.app_id,
        workspace_key=workspace_key,
        thread_id=thread_id,
        title=_thread_title(prompt),
    )
    store.append_message(
        app_id=app.app_id,
        workspace_key=workspace_key,
        thread_id=thread_id,
        role="user",
        content=prompt,
        metadata={"legacyMessages": legacy_messages},
    )
    runtime = binding.get_runtime()
    stream = (
        runtime.query(prompt, thread_id=thread_id)
        if agent.role == "coordinator"
        else runtime.run_specialist(agent.name, prompt, thread_id=thread_id)
    )
    active_run = _ActiveChatRun(stream=stream, thread_id=thread_id, agent_id=agent.name)
    active_chat_runs[(workspace_key, run_id)] = active_run
    recorder = _RunEventRecorder(
        app=app,
        store=store,
        workspace_key=workspace_key,
        run_id=run_id,
        thread_id=thread_id,
        operation="chat",
        agent_id=agent.name,
    )
    visible_events: list[dict[str, Any]] = []
    active_nested_agents: set[str] = set()
    seen_lifecycle_events: set[tuple[str, str, str]] = set()
    disconnected = False
    stream_failed = False
    stream_cancelled = False
    last_finish_raw: dict[str, Any] | None = None
    assistant_message_persisted = False

    def persist_assistant_message(*, status: str | None = None) -> None:
        nonlocal assistant_message_persisted
        if assistant_message_persisted:
            return
        message_parts = _assistant_message_parts_from_events(
            visible_events,
            pending_state="cancelled" if status == "cancelled" else None,
        )
        message_text = _plain_text_from_message_parts(message_parts)
        if not message_parts:
            return
        metadata: dict[str, Any] = {"runId": run_id, "parts": message_parts}
        if status is not None:
            metadata["status"] = status
        store.append_message(
            app_id=app.app_id,
            workspace_key=workspace_key,
            thread_id=thread_id,
            role="assistant",
            content=message_text,
            metadata=metadata,
        )
        assistant_message_persisted = True

    try:
        async for raw in stream:
            if await request.is_disconnected():
                stream_cancelled = True
                disconnected = True
                _cancel_runtime_stream(stream)
                if not active_run.cancel_recorded:
                    _record_chat_cancelled(
                        app=app,
                        store=store,
                        workspace_key=workspace_key,
                        run_id=run_id,
                        active_run=active_run,
                    )
                break
            if not isinstance(raw, dict):
                raw = {"type": "runtime-event", "value": raw}
            if raw.get("type") == "finish":
                last_finish_raw = raw
            kind, payload = project_runtime_event(raw)
            dedupe_key = _event_dedupe_key(kind, payload)
            if dedupe_key is not None:
                if dedupe_key in seen_lifecycle_events:
                    continue
                seen_lifecycle_events.add(dedupe_key)
            hide_entrypoint_agent_lifecycle = (
                kind in {"sub_agent.call.started", "sub_agent.call.completed"}
                and payload.get("targetAgentId") == agent.name
            )
            if kind == "sub_agent.call.started":
                nested_id = _event_tool_call_id(payload) or _string_or_none(
                    payload.get("targetAgentId")
                )
                if nested_id and payload.get("targetAgentId") != agent.name:
                    active_nested_agents.add(nested_id)
            suppress_nested_text = kind == "message.delta" and bool(active_nested_agents)
            if suppress_nested_text:
                kind, payload = "runtime.event", {"raw": raw}
            if kind == "run.failed":
                if active_run.cancel_recorded or _is_cancelled_runtime_error(payload):
                    stream_cancelled = True
                    active_run.cancel_recorded = True
                    kind, payload = "run.cancelled", {"status": "cancelled"}
                else:
                    stream_failed = True
            event = recorder.record(kind=kind, payload=payload, raw=raw)
            if hide_entrypoint_agent_lifecycle:
                continue
            if kind != "runtime.finish":
                visible_events.append(event)
            yield encode_sse(event)
            if kind == "sub_agent.call.completed":
                nested_id = _event_tool_call_id(payload) or _string_or_none(
                    payload.get("targetAgentId")
                )
                if nested_id:
                    active_nested_agents.discard(nested_id)
            if stream_failed or stream_cancelled:
                return
        persist_assistant_message(
            status=(
                "cancelled"
                if disconnected or active_run.cancel_recorded or stream_cancelled
                else None
            )
        )
        if not disconnected and not active_run.cancel_recorded:
            event = recorder.record(
                kind="run.completed",
                payload={"status": "completed", "raw": last_finish_raw or {}},
                raw=last_finish_raw or {},
            )
            yield encode_sse(event)
    except asyncio.CancelledError:
        stream_cancelled = True
        persist_assistant_message(status="cancelled")
        if not active_run.cancel_recorded:
            _record_chat_cancelled(
                app=app,
                store=store,
                workspace_key=workspace_key,
                run_id=run_id,
                active_run=active_run,
            )
        raise
    except Exception as exc:  # noqa: BLE001 - stream boundary.
        stream_failed = True
        event = recorder.record(
            kind="run.failed",
            payload={
                "error": {
                    "code": "studio.stream_error",
                    "message": str(exc),
                    "retryable": False,
                    "details": {},
                }
            },
            raw={},
        )
        yield encode_sse(event)
    finally:
        should_persist_cancelled = (
            stream_cancelled or active_run.cancel_recorded or not stream_failed
        )
        if visible_events and not assistant_message_persisted and should_persist_cancelled:
            persist_assistant_message(status="cancelled")
            if not active_run.cancel_recorded:
                _record_chat_cancelled(
                    app=app,
                    store=store,
                    workspace_key=workspace_key,
                    run_id=run_id,
                    active_run=active_run,
                )
        active_chat_runs.pop((workspace_key, run_id), None)


def _thread_title(prompt: str) -> str:
    return prompt[:80] if len(prompt) <= 80 else f"{prompt[:77]}..."


def _cancel_runtime_stream(stream: Any) -> bool:
    cancel = getattr(stream, "cancel", None)
    if not callable(cancel):
        return False
    cancel()
    return True


def _is_cancelled_runtime_error(payload: dict[str, Any]) -> bool:
    error = payload.get("error") if isinstance(payload.get("error"), dict) else {}
    message = error.get("message")
    return isinstance(message, str) and message.strip().lower() in {
        "request cancelled",
        "request canceled",
    }


def _record_chat_cancelled(
    *,
    app: FlowAIApp,
    store: StudioStore,
    workspace_key: str,
    run_id: str,
    active_run: _ActiveChatRun,
) -> None:
    recorder = _RunEventRecorder(
        app=app,
        store=store,
        workspace_key=workspace_key,
        run_id=run_id,
        thread_id=active_run.thread_id,
        operation="chat",
        agent_id=active_run.agent_id,
    )
    recorder.record(kind="run.cancelled", payload={"status": "cancelled"}, raw={})
    active_run.cancel_recorded = True


def _event_tool_call_id(payload: dict[str, Any]) -> str | None:
    return _string_or_none(payload.get("toolCallId")) or _string_or_none(
        payload.get("toolInvocationId")
    )


def _event_dedupe_key(kind: str, payload: dict[str, Any]) -> tuple[str, str, str] | None:
    """Return a key for duplicate lifecycle events that should be idempotent."""

    if kind in {"tool.call.started", "tool.call.completed"}:
        tool_call_id = _event_tool_call_id(payload)
        tool_name = _string_or_none(payload.get("toolName")) or ""
        if tool_call_id:
            return (kind, tool_call_id, tool_name)
        return (kind, json.dumps(payload, sort_keys=True, default=str), tool_name)

    if kind in {"sub_agent.call.started", "sub_agent.call.completed"}:
        tool_call_id = _event_tool_call_id(payload)
        agent_id = _string_or_none(payload.get("targetAgentId")) or ""
        if tool_call_id:
            return (kind, tool_call_id, agent_id)
        return (kind, json.dumps(payload, sort_keys=True, default=str), agent_id)

    return None


def _assistant_message_parts_from_events(
    events: list[dict[str, Any]], *, pending_state: str | None = None
) -> list[dict[str, Any]]:
    parts: list[dict[str, Any]] = []
    text_buffer: list[str] = []
    started_tool_arguments: dict[str, Any] = {}
    started_tool_calls: dict[str, dict[str, Any]] = {}
    started_agent_calls: dict[str, dict[str, Any]] = {}
    completed_tool_call_ids: set[str] = set()
    completed_agent_call_ids: set[str] = set()
    pending_order: list[tuple[str, str]] = []
    seen_lifecycle_events: set[tuple[str, str, str]] = set()

    def flush_text() -> None:
        if text_buffer:
            parts.append({"type": "text", "text": "".join(text_buffer)})
            text_buffer.clear()

    for event in events:
        kind = event.get("kind")
        payload = event.get("payload") if isinstance(event.get("payload"), dict) else {}
        dedupe_key = _event_dedupe_key(str(kind), payload)
        if dedupe_key is not None:
            if dedupe_key in seen_lifecycle_events:
                continue
            seen_lifecycle_events.add(dedupe_key)

        if kind == "message.delta":
            text = payload.get("text")
            if isinstance(text, str) and text:
                text_buffer.append(text)
            continue

        if kind == "tool.call.started":
            flush_text()
            tool_call_id = _event_tool_call_id(payload)
            if tool_call_id:
                started_tool_arguments[tool_call_id] = payload.get("arguments", {})
                if tool_call_id not in started_tool_calls:
                    pending_order.append(("tool", tool_call_id))
                started_tool_calls[tool_call_id] = {
                    "toolCallId": tool_call_id,
                    "toolName": str(payload.get("toolName") or "tool"),
                    "args": payload.get("arguments", {}),
                }
            continue

        if kind == "tool.call.completed":
            flush_text()
            tool_call_id = _event_tool_call_id(payload) or ""
            if tool_call_id:
                completed_tool_call_ids.add(tool_call_id)
            arguments = (
                payload["arguments"]
                if "arguments" in payload
                else started_tool_arguments.get(tool_call_id, {})
            )
            parts.append(
                {
                    "type": "tool-invocation",
                    "toolCallId": tool_call_id,
                    "toolName": str(payload.get("toolName") or "tool"),
                    "args": arguments,
                    "state": "result",
                    "result": payload.get("result"),
                }
            )
            continue

        if kind == "sub_agent.call.started":
            flush_text()
            tool_call_id = _event_tool_call_id(payload)
            if tool_call_id:
                if tool_call_id not in started_agent_calls:
                    pending_order.append(("agent", tool_call_id))
                started_agent_calls[tool_call_id] = {
                    "toolCallId": tool_call_id,
                    "agentName": str(payload.get("targetAgentId") or "sub-agent"),
                }
            continue

        if kind == "sub_agent.call.completed":
            flush_text()
            tool_call_id = _event_tool_call_id(payload) or ""
            if tool_call_id:
                completed_agent_call_ids.add(tool_call_id)
            parts.append(
                {
                    "type": "tool-agent",
                    "toolCallId": tool_call_id,
                    "agentName": str(payload.get("targetAgentId") or "sub-agent"),
                    "state": "result",
                }
            )
            continue

    if pending_state == "cancelled":
        flush_text()
        for pending_kind, tool_call_id in pending_order:
            if pending_kind == "tool":
                if tool_call_id in completed_tool_call_ids:
                    continue
                started_tool = started_tool_calls.get(tool_call_id)
                if started_tool is None:
                    continue
                parts.append(
                    {
                        "type": "tool-invocation",
                        **started_tool,
                        "state": "cancelled",
                    }
                )
                continue

            if tool_call_id in completed_agent_call_ids:
                continue
            started_agent = started_agent_calls.get(tool_call_id)
            if started_agent is None:
                continue
            parts.append(
                {
                    "type": "tool-agent",
                    **started_agent,
                    "state": "cancelled",
                }
            )

    flush_text()
    return parts


def _plain_text_from_message_parts(parts: list[dict[str, Any]]) -> str:
    return "".join(
        str(part.get("text") or "")
        for part in parts
        if part.get("type") == "text"
    )


def _error_response(
    status_code: int,
    *,
    code: str,
    message: str,
    details: dict[str, Any] | None = None,
) -> JSONResponse:
    return JSONResponse(
        {
            "error": {
                "code": code,
                "message": message,
                "retryable": False,
                "details": details or {},
            }
        },
        status_code=status_code,
        headers={"Cache-Control": "no-store"},
    )
