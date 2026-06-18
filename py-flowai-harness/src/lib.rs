//! Private PyO3 bindings for the public `flowai-harness` Python package.
//!
//! The public Python facade validates and constructs Flow AI runtime specs.
//! This module exposes only low-level runtime artifacts needed by that facade:
//! a native runtime handle, async stream conversion, and callback adapters that
//! enter through the Rust runtime's normal dispatch points.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use agent_fw_agent::{
    ChatInterpreter, ChatProgram, ToolCallResult, ToolDefinition, ToolDispatcher, ToolHandler,
};
use agent_fw_algebra::testing::NullEventSink;
use agent_fw_algebra::CancellationToken;
use agent_fw_core::datasource::DatabaseType;
use agent_fw_core::stream_part::FinishReason;
use agent_fw_core::tenant::TenantContext;
use agent_fw_core::{StreamPart, TenantId, ThreadId};
use agent_fw_eval::{EvalMode, EvalTestCase, RawSampleOutput, ScoreWeights};
use agent_fw_ingest::introspection::IntrospectionService;
use agent_fw_interpreter::{
    DashMapKVStore, MockChatInterpreter, MockEnricher, RigAnthropicChatInterpreter,
};
use agent_fw_plan::{ActionDispatcher, ActionSeq, ExecutionResult};
use agent_fw_tool::ToolEnvironment;
use async_trait::async_trait;
use flowai_runtime::storage::DataEnvironmentConfig;
use flowai_runtime::RecordingTraceSink;
use flowai_runtime::{
    ApprovalDecision, ApprovalOutcome, ApprovalRule, ArtifactRef, CancellableRuntimeEventStream,
    EvalEventStream, HarnessAction, HarnessActionContext, HarnessActionError, HostToolBinding,
    QueryRequest, ReferenceRegistry, Runtime, RuntimeDeps, RuntimeError, RuntimeEventStream,
    RuntimeMcpConfig, RuntimeMcpError, RuntimeSpec, SpecialistRequest, TraceListFilter, TraceSink,
};
use futures::{stream, Stream, StreamExt};
use pyo3::exceptions::{
    PyAttributeError, PyKeyError, PyRuntimeError, PyStopAsyncIteration, PyValueError,
};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tokio::sync::Mutex as TokioMutex;

const NATIVE_API_VERSION: u32 = 4;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostToolSpec {
    binding_id: String,
    name: String,
    description: String,
    input_schema: JsonValue,
    approval: Option<ApprovalRule>,
}

#[derive(Debug, Deserialize)]
struct ScriptedToolCall {
    tool: String,
    #[serde(default)]
    args: JsonValue,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ScriptedPrompt {
    ToolCall(ScriptedToolCall),
    Script { script: Vec<ScriptedToolCall> },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum PyKnowledgeSourceSpec {
    LocalDirectory {
        path: String,
        #[serde(default)]
        extensions: Vec<String>,
    },
    S3Bucket {
        bucket: String,
        #[serde(default)]
        prefix: Option<String>,
        #[serde(default)]
        region: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PySearchCatalogRequest {
    query: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PyListMetricsRequest {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

impl ScriptedPrompt {
    fn into_calls(self) -> Vec<ScriptedToolCall> {
        match self {
            Self::ToolCall(call) => vec![call],
            Self::Script { script } => script,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScoreEvalSampleOptions {
    mode: Option<EvalMode>,
    scorer_preset: Option<String>,
    score_weights: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    scorer_config: Option<JsonValue>,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSampleOutputWire {
    #[serde(default)]
    actual_trajectory: Vec<String>,
    #[serde(default)]
    response_text: Option<String>,
    #[serde(default)]
    extra: Option<JsonValue>,
}

impl From<RawSampleOutputWire> for RawSampleOutput {
    fn from(value: RawSampleOutputWire) -> Self {
        let output = match value.extra {
            Some(extra) => RawSampleOutput::with_extra(value.actual_trajectory, extra),
            None => RawSampleOutput::new(value.actual_trajectory),
        };
        match value.response_text {
            Some(response_text) => output.with_response_text(response_text),
            None => output,
        }
    }
}

/// Native Flow AI runtime handle returned by `create_runtime(...)`.
///
/// Owns agent orchestration, provider routing, approval gates, plan
/// lifecycle, reference storage, eval execution, and MCP tool serving.
/// Python supplies callback adapters only. Exported as
/// `flowai_harness.Runtime`.
#[pyclass(name = "PyRuntime")]
pub struct PyRuntime {
    inner: Arc<Runtime>,
    resource_id: String,
    event_hooks: Arc<Vec<Py<PyAny>>>,
    python_loop: Arc<StdMutex<Option<Py<PyAny>>>>,
    trace_sink: Arc<RecordingTraceSink>,
}

#[pymethods]
impl PyRuntime {
    /// Tenant resource id this runtime executes under.
    #[getter]
    fn resource_id(&self) -> String {
        self.resource_id.clone()
    }

    /// Run a coordinator turn and return an async-iterable event stream.
    ///
    /// Args:
    ///     prompt: User prompt for this turn.
    ///     thread_id: Conversation thread identifier.
    ///     resume: Optional resume token continuing an interrupted run.
    ///
    /// Returns:
    ///     An async-iterable stream yielding runtime event dicts.
    #[pyo3(signature = (prompt, thread_id, resume=None))]
    fn query(
        &self,
        prompt: String,
        thread_id: String,
        resume: Option<String>,
    ) -> PyRuntimeEventStream {
        let runtime = self.inner.clone();
        let resource_id = TenantId::new_unchecked(self.resource_id.clone());
        PyRuntimeEventStream::new(
            move || {
                runtime.query_cancellable(QueryRequest {
                    prompt,
                    resource_id,
                    thread_id: ThreadId::new_unchecked(thread_id),
                    resume,
                })
            },
            self.event_hooks.clone(),
            self.python_loop.clone(),
        )
    }

    /// Dispatch a specialist agent directly, bypassing the coordinator.
    ///
    /// Args:
    ///     specialist: Name of the registered specialist agent.
    ///     prompt: User prompt for the specialist.
    ///     thread_id: Optional conversation thread identifier.
    ///
    /// Returns:
    ///     An async-iterable stream yielding runtime event dicts.
    #[pyo3(signature = (specialist, prompt, thread_id=None))]
    fn run_specialist(
        &self,
        specialist: String,
        prompt: String,
        thread_id: Option<String>,
    ) -> PyRuntimeEventStream {
        let runtime = self.inner.clone();
        let resource_id = TenantId::new_unchecked(self.resource_id.clone());
        PyRuntimeEventStream::new(
            move || {
                runtime.run_specialist_cancellable(SpecialistRequest {
                    specialist,
                    prompt,
                    resource_id,
                    thread_id: thread_id.map(ThreadId::new_unchecked),
                })
            },
            self.event_hooks.clone(),
            self.python_loop.clone(),
        )
    }

    /// Run an eval to completion and return the eval artifact.
    ///
    /// Args:
    ///     eval_request: `EvalRequest` model, mapping, or JSON string.
    ///
    /// Returns:
    ///     Awaitable resolving to the eval artifact dict (validate with
    ///     `EvalArtifact`).
    ///
    /// Raises:
    ///     ValueError: If the request cannot be parsed.
    ///     RuntimeError: If the eval run fails.
    #[pyo3(signature = (eval_request))]
    fn run_eval<'py>(
        &self,
        py: Python<'py>,
        eval_request: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        // Capture the caller's asyncio loop so async Python tool callbacks can be
        // driven via run_coroutine_threadsafe during sample execution. Unlike the
        // streaming paths (query/stream_eval, which capture lazily on first
        // __anext__), run_eval resolves a single future and never iterates a
        // stream, so without this the loop stays None and every `async def` tool
        // callback fails with "async Python callback requires the runtime stream
        // to be iterated from asyncio" (the coroutine is dropped, never awaited).
        capture_python_loop(py, &self.python_loop);
        let request_json = py_model_or_json_to_string(py, eval_request.bind(py))?;
        let request: flowai_runtime::EvalRequest =
            serde_json::from_str(&request_json).map_err(py_value_error)?;
        let runtime = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let artifact = runtime
                .run_eval(request)
                .await
                .map_err(eval_runner_error_to_py)?;
            Python::with_gil(|py| {
                json_value_to_py(
                    py,
                    &serde_json::to_value(&artifact).map_err(py_runtime_error)?,
                )
            })
        })
    }

    /// Run an eval and stream progress event envelopes.
    ///
    /// Args:
    ///     eval_request: `EvalRequest` model, mapping, or JSON string.
    ///
    /// Returns:
    ///     An async-iterable stream yielding eval event envelope dicts
    ///     (validate with `HarnessEvalEventEnvelope`).
    ///
    /// Raises:
    ///     ValueError: If the request cannot be parsed.
    #[pyo3(signature = (eval_request))]
    fn stream_eval(&self, py: Python<'_>, eval_request: Py<PyAny>) -> PyResult<PyEvalEventStream> {
        let request_json = py_model_or_json_to_string(py, eval_request.bind(py))?;
        let request: flowai_runtime::EvalRequest =
            serde_json::from_str(&request_json).map_err(py_value_error)?;
        let runtime = self.inner.clone();
        Ok(PyEvalEventStream::new(
            move || runtime.stream_eval(request),
            self.python_loop.clone(),
        ))
    }

    /// Return one recorded trace by id, or None when not found.
    ///
    /// Args:
    ///     trace_id: Trace identifier from an eval artifact or event.
    ///
    /// Returns:
    ///     The trace dict, or None when no trace has that id.
    #[pyo3(signature = (trace_id))]
    fn get_trace(&self, py: Python<'_>, trace_id: String) -> PyResult<Option<Py<PyAny>>> {
        let trace = pyo3_async_runtimes::tokio::get_runtime()
            .block_on(self.trace_sink.get_trace(&trace_id))
            .map_err(py_runtime_error)?;
        match trace {
            Some(trace) => {
                let value = serde_json::to_value(trace).map_err(py_runtime_error)?;
                Ok(Some(json_value_to_py(py, &value)?))
            }
            None => Ok(None),
        }
    }

    /// List recorded traces, optionally filtered.
    ///
    /// Args:
    ///     eval_run_id: Only traces recorded for this eval run.
    ///     test_case_id: Only traces recorded for this test case.
    ///     thread_id: Only traces recorded for this thread.
    ///
    /// Returns:
    ///     A list of trace dicts matching every supplied filter.
    #[pyo3(signature = (eval_run_id=None, test_case_id=None, thread_id=None))]
    fn list_traces(
        &self,
        py: Python<'_>,
        eval_run_id: Option<String>,
        test_case_id: Option<String>,
        thread_id: Option<String>,
    ) -> PyResult<Py<PyAny>> {
        let traces = pyo3_async_runtimes::tokio::get_runtime()
            .block_on(self.trace_sink.list_traces(TraceListFilter {
                eval_run_id,
                test_case_id,
                thread_id,
            }))
            .map_err(py_runtime_error)?;
        let value = serde_json::to_value(traces).map_err(py_runtime_error)?;
        json_value_to_py(py, &value)
    }

    /// Store a value and return its typed reference envelope.
    ///
    /// Args:
    ///     reference: `ReferenceSpec` or reference kind name. A spec's
    ///         Python `glimpse` callback runs once before storing.
    ///     value: JSON-serializable payload to store.
    ///     glimpse: Explicit glimpse value; defaults to `{}` when neither a
    ///         callback nor a value is supplied.
    ///
    /// Returns:
    ///     Awaitable resolving to the reference envelope dict.
    #[pyo3(signature = (reference, value, glimpse=None))]
    fn create_reference<'py>(
        &self,
        py: Python<'py>,
        reference: Py<PyAny>,
        value: Py<PyAny>,
        glimpse: Option<Py<PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        PyReferenceClient::new(
            self.inner.references().clone(),
            TenantId::new_unchecked(self.resource_id.clone()),
        )
        .create(py, reference, value, glimpse)
    }

    /// Resolve a reference envelope to its full stored payload.
    ///
    /// Args:
    ///     reference: Reference envelope previously returned by
    ///         `create_reference(...)` or emitted by the runtime.
    ///
    /// Returns:
    ///     Awaitable resolving to the stored payload.
    #[pyo3(signature = (reference))]
    fn resolve_reference<'py>(
        &self,
        py: Python<'py>,
        reference: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        PyReferenceClient::new(
            self.inner.references().clone(),
            TenantId::new_unchecked(self.resource_id.clone()),
        )
        .resolve(py, reference)
    }

    /// Return the cached glimpse for a reference without resolving it.
    ///
    /// Args:
    ///     reference: Reference envelope previously returned by
    ///         `create_reference(...)` or emitted by the runtime.
    ///
    /// Returns:
    ///     Awaitable resolving to the cached glimpse value.
    #[pyo3(signature = (reference))]
    fn reference_glimpse<'py>(
        &self,
        py: Python<'py>,
        reference: Py<PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        PyReferenceClient::new(
            self.inner.references().clone(),
            TenantId::new_unchecked(self.resource_id.clone()),
        )
        .glimpse(py, reference)
    }

    /// Resolve a pending approval gate.
    ///
    /// Args:
    ///     approval_id: Approval id from the approval event.
    ///     outcome: `"approve"`, `"reject"`, or `"revise"`.
    ///     feedback: Optional reviewer feedback forwarded to the agent.
    ///     partial: Optional partial revision payload.
    ///
    /// Raises:
    ///     ValueError: If `outcome` is not a supported value.
    ///     RuntimeError: If the approval id is unknown or the gate cannot
    ///         be resolved.
    #[pyo3(signature = (approval_id, outcome, feedback=None, partial=None))]
    fn respond_to_approval<'py>(
        &self,
        py: Python<'py>,
        approval_id: String,
        outcome: String,
        feedback: Option<String>,
        partial: Option<Py<PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let outcome = match outcome.as_str() {
            "approve" => ApprovalOutcome::Approve,
            "reject" => ApprovalOutcome::Reject,
            "revise" => ApprovalOutcome::Revise,
            other => {
                return Err(PyValueError::new_err(format!(
                    "approval outcome must be approve, reject, or revise; got {other}"
                )));
            }
        };
        let partial = match partial {
            Some(value) => Some(py_any_to_json_value(py, value.bind(py))?),
            None => None,
        };
        let runtime = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            runtime
                .respond_to_approval(ApprovalDecision {
                    approval_id,
                    outcome,
                    feedback,
                    partial,
                })
                .await
                .map_err(runtime_error_to_py)?;
            Ok(())
        })
    }

    /// Return MCP tool descriptors for one runtime agent.
    ///
    /// Args:
    ///     agent: Runtime agent whose direct tools are listed.
    ///     expose_agent_tools: Reserved for future recursive agent tools;
    ///         the runtime-generated `agents` toolkit is not supported in
    ///         this mode.
    ///
    /// Returns:
    ///     A list of MCP tool descriptor dicts.
    #[pyo3(signature = (agent, *, expose_agent_tools=false))]
    fn list_mcp_tools(
        &self,
        py: Python<'_>,
        agent: String,
        expose_agent_tools: bool,
    ) -> PyResult<Py<PyAny>> {
        let server = self
            .inner
            .clone()
            .mcp_tool_server(RuntimeMcpConfig {
                agent,
                thread_id: None,
                call_timeout: std::time::Duration::from_secs(30),
                expose_agent_tools,
            })
            .map_err(runtime_mcp_error_to_py)?;
        let tools = serde_json::to_value(server.list_mcp_tools()).map_err(py_runtime_error)?;
        json_value_to_py(py, &tools)
    }

    /// Serve one agent's tools over MCP stdio until the client disconnects.
    ///
    /// Python tool callbacks execute in the calling Python process.
    ///
    /// Args:
    ///     agent: Runtime agent whose tools are exposed.
    ///     thread_id: Optional fixed thread id for runtime tool dispatch.
    ///     call_timeout_secs: Per tool-call timeout in seconds.
    ///     expose_agent_tools: Reserved for future recursive agent tools.
    ///
    /// Returns:
    ///     Awaitable resolving when the MCP client disconnects.
    #[pyo3(signature = (
        agent,
        *,
        thread_id=None,
        call_timeout_secs=30.0,
        expose_agent_tools=false
    ))]
    fn serve_mcp_stdio<'py>(
        &self,
        py: Python<'py>,
        agent: String,
        thread_id: Option<String>,
        call_timeout_secs: f64,
        expose_agent_tools: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        capture_python_loop(py, &self.python_loop);
        let server = self.mcp_server(agent, thread_id, call_timeout_secs, expose_agent_tools)?;
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            server
                .serve_stdio()
                .await
                .map_err(|err| PyRuntimeError::new_err(err.to_string()))?;
            Ok(())
        })
    }

    /// Serve one agent's tools over MCP Streamable HTTP.
    ///
    /// Binds to loopback by default and validates browser `Origin` headers
    /// unless `require_origin=False` is supplied.
    ///
    /// Args:
    ///     agent: Runtime agent whose tools are exposed.
    ///     host: Host to bind.
    ///     port: Port to bind.
    ///     path: HTTP endpoint path.
    ///     transport: Only `"streamable-http"` is supported.
    ///     thread_id: Optional fixed thread id for runtime tool dispatch.
    ///     call_timeout_secs: Per tool-call timeout in seconds.
    ///     expose_agent_tools: Reserved for future recursive agent tools.
    ///     allowed_origins: Additional `Origin` header values to accept.
    ///     require_origin: Validate browser `Origin` headers.
    ///
    /// Returns:
    ///     Awaitable resolving when the server stops.
    ///
    /// Raises:
    ///     ValueError: If `transport` is not `"streamable-http"`.
    ///     RuntimeError: If the server fails to bind or serving fails.
    #[pyo3(signature = (
        agent,
        *,
        host="127.0.0.1".to_string(),
        port=8765,
        path="/mcp".to_string(),
        transport="streamable-http".to_string(),
        thread_id=None,
        call_timeout_secs=30.0,
        expose_agent_tools=false,
        allowed_origins=None,
        require_origin=true
    ))]
    fn serve_mcp_http<'py>(
        &self,
        py: Python<'py>,
        agent: String,
        host: String,
        port: u16,
        path: String,
        transport: String,
        thread_id: Option<String>,
        call_timeout_secs: f64,
        expose_agent_tools: bool,
        allowed_origins: Option<Vec<String>>,
        require_origin: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        if transport != "streamable-http" {
            return Err(PyValueError::new_err(format!(
                "transport must be 'streamable-http'; got {transport}"
            )));
        }
        capture_python_loop(py, &self.python_loop);
        let server = self.mcp_server(agent, thread_id, call_timeout_secs, expose_agent_tools)?;
        let bind_addr = parse_bind_addr(&host, port)?;
        let http_config = agent_fw_mcp::McpHttpServerConfig {
            bind_addr,
            endpoint_path: path.clone(),
            allowed_origins: allowed_origins.unwrap_or_default(),
            require_origin,
        };
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let bound = server
                .bind_streamable_http(http_config)
                .await
                .map_err(|err| {
                    PyRuntimeError::new_err(format!(
                        "failed to bind MCP HTTP server on {host}:{port}{path}: {err}"
                    ))
                })?;
            eprintln!(
                "flowai-harness MCP Streamable HTTP listening on {}",
                bound.endpoint_url()
            );
            bound
                .serve()
                .await
                .map_err(|err| PyRuntimeError::new_err(err.to_string()))?;
            Ok(())
        })
    }
}

impl PyRuntime {
    fn mcp_server(
        &self,
        agent: String,
        thread_id: Option<String>,
        call_timeout_secs: f64,
        expose_agent_tools: bool,
    ) -> PyResult<agent_fw_mcp::McpToolServer> {
        Ok(self
            .inner
            .clone()
            .mcp_tool_server(RuntimeMcpConfig {
                agent,
                thread_id: thread_id.map(ThreadId::new_unchecked),
                call_timeout: duration_from_secs(call_timeout_secs)?,
                expose_agent_tools,
            })
            .map_err(runtime_mcp_error_to_py)?)
    }
}

type StreamStarter = Box<dyn FnOnce() -> CancellableRuntimeEventStream + Send + 'static>;

#[pyclass(name = "PyRuntimeEventStream")]
pub struct PyRuntimeEventStream {
    stream: Arc<TokioMutex<Option<RuntimeEventStream>>>,
    starter: Arc<StdMutex<Option<StreamStarter>>>,
    cancel: Arc<StdMutex<Option<CancellationToken>>>,
    cancel_requested: Arc<AtomicBool>,
    event_hooks: Arc<Vec<Py<PyAny>>>,
    python_loop: Arc<StdMutex<Option<Py<PyAny>>>>,
}

type EvalStreamStarter = Box<dyn FnOnce() -> EvalEventStream + Send + 'static>;

#[pyclass(name = "PyEvalEventStream")]
pub struct PyEvalEventStream {
    stream: Arc<TokioMutex<Option<EvalEventStream>>>,
    starter: Arc<StdMutex<Option<EvalStreamStarter>>>,
    python_loop: Arc<StdMutex<Option<Py<PyAny>>>>,
}

impl PyEvalEventStream {
    fn new(
        start: impl FnOnce() -> EvalEventStream + Send + 'static,
        python_loop: Arc<StdMutex<Option<Py<PyAny>>>>,
    ) -> Self {
        Self {
            stream: Arc::new(TokioMutex::new(None)),
            starter: Arc::new(StdMutex::new(Some(Box::new(start)))),
            python_loop,
        }
    }
}

#[pymethods]
impl PyEvalEventStream {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let stream = self.stream.clone();
        let starter = self.starter.clone();
        let python_loop = self.python_loop.clone();
        capture_python_loop(py, &python_loop);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let event = {
                let mut guard = stream.lock().await;
                if guard.is_none() {
                    let start = {
                        let mut starter = starter.lock().map_err(|_| {
                            PyRuntimeError::new_err("eval stream starter lock poisoned")
                        })?;
                        starter.take().ok_or_else(|| {
                            PyRuntimeError::new_err("eval stream was already consumed")
                        })?
                    };
                    *guard = Some(start());
                }
                guard.as_mut().expect("stream initialized").next().await
            };
            let Some(event) = event else {
                return Err(PyStopAsyncIteration::new_err(()));
            };
            Python::with_gil(|py| {
                json_value_to_py(py, &serde_json::to_value(&event).map_err(py_runtime_error)?)
            })
        })
    }
}

impl PyRuntimeEventStream {
    fn new(
        start: impl FnOnce() -> CancellableRuntimeEventStream + Send + 'static,
        event_hooks: Arc<Vec<Py<PyAny>>>,
        python_loop: Arc<StdMutex<Option<Py<PyAny>>>>,
    ) -> Self {
        Self {
            stream: Arc::new(TokioMutex::new(None)),
            starter: Arc::new(StdMutex::new(Some(Box::new(start)))),
            cancel: Arc::new(StdMutex::new(None)),
            cancel_requested: Arc::new(AtomicBool::new(false)),
            event_hooks,
            python_loop,
        }
    }
}

#[pymethods]
impl PyRuntimeEventStream {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let stream = self.stream.clone();
        let starter = self.starter.clone();
        let cancel = self.cancel.clone();
        let cancel_requested = self.cancel_requested.clone();
        let hooks = self.event_hooks.clone();
        let python_loop = self.python_loop.clone();
        capture_python_loop(py, &python_loop);
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let part = {
                let mut guard = stream.lock().await;
                if guard.is_none() {
                    let start = {
                        let mut starter = starter.lock().map_err(|_| {
                            PyRuntimeError::new_err("runtime stream starter lock poisoned")
                        })?;
                        starter.take().ok_or_else(|| {
                            PyRuntimeError::new_err("runtime stream was already consumed")
                        })?
                    };
                    let cancellable = start();
                    let (runtime_stream, request_cancel) = cancellable.into_parts();
                    if cancel_requested.load(Ordering::SeqCst) {
                        request_cancel.cancel();
                    }
                    {
                        let mut cancel_guard = cancel.lock().map_err(|_| {
                            PyRuntimeError::new_err("runtime stream cancel lock poisoned")
                        })?;
                        *cancel_guard = Some(request_cancel);
                    }
                    *guard = Some(runtime_stream);
                }
                guard.as_mut().expect("stream initialized").next().await
            };
            let Some(part) = part else {
                return Err(PyStopAsyncIteration::new_err(()));
            };
            Python::with_gil(|py| {
                let event =
                    json_value_to_py(py, &serde_json::to_value(&part).map_err(py_runtime_error)?)?;
                for hook in hooks.iter() {
                    hook.call1(py, (event.clone_ref(py),))?;
                }
                Ok(event)
            })
        })
    }

    fn cancel(&self) -> PyResult<()> {
        self.cancel_requested.store(true, Ordering::SeqCst);
        let guard = self
            .cancel
            .lock()
            .map_err(|_| PyRuntimeError::new_err("runtime stream cancel lock poisoned"))?;
        if let Some(cancel) = guard.as_ref() {
            cancel.cancel();
        }
        Ok(())
    }
}

#[pyfunction]
#[pyo3(signature = (argv=None))]
fn run_cli(argv: Option<Vec<String>>) -> PyResult<i32> {
    let args = argv.unwrap_or_else(|| vec!["flowai-harness".to_string()]);
    match pyo3_async_runtimes::tokio::get_runtime().block_on(flowai_harness_cli::run(args)) {
        Ok(()) => Ok(0),
        Err(_) => Ok(1),
    }
}

#[pyfunction]
#[pyo3(signature = (toolkit_json, agent_json))]
fn describe_toolkit_tools(toolkit_json: String, agent_json: String) -> PyResult<String> {
    let toolkit: flowai_runtime::ToolkitSpec =
        serde_json::from_str(&toolkit_json).map_err(py_value_error)?;
    let agent: flowai_runtime::AgentSpec =
        serde_json::from_str(&agent_json).map_err(py_value_error)?;
    let definitions = flowai_runtime::toolkits::describe_toolkit_tools(&toolkit, &agent)
        .map_err(py_value_error)?;
    serde_json::to_string(&definitions).map_err(py_runtime_error)
}

#[pyfunction]
#[pyo3(signature = (test_case_json, output_json, options_json))]
fn score_eval_sample(
    py: Python<'_>,
    test_case_json: String,
    output_json: String,
    options_json: String,
) -> PyResult<Py<PyAny>> {
    let test_case: EvalTestCase = serde_json::from_str(&test_case_json).map_err(py_value_error)?;
    let output: RawSampleOutput = serde_json::from_str::<RawSampleOutputWire>(&output_json)
        .map_err(py_value_error)?
        .into();
    let options: ScoreEvalSampleOptions =
        serde_json::from_str(&options_json).map_err(py_value_error)?;
    let _scope_echo = (&options.tenant_id, &options.workspace_id);
    let weights = options
        .score_weights
        .map(|weights| ScoreWeights::new(weights.into_iter().collect()))
        .transpose()
        .map_err(py_value_error)?;

    let preset = match (options.mode, options.scorer_preset.as_deref()) {
        (None, None) => flowai_runtime::PRESET_SEQUENTIAL,
        (None, Some(preset)) => preset,
        (Some(mode), None) => preset_for_mode(mode),
        (Some(_), Some(flowai_runtime::PRESET_TRAJECTORY_ONLY)) => {
            return Err(PyValueError::new_err(
                "scorerPreset='trajectory_only' is valid only when mode is omitted",
            ));
        }
        (Some(mode), Some(preset)) if preset_matches_mode(preset, mode) => preset,
        (Some(mode), Some(preset)) => {
            return Err(PyValueError::new_err(format!(
                "mode '{}' conflicts with scorerPreset '{preset}'",
                eval_mode_wire_name(mode)
            )));
        }
    };
    let weights = match weights {
        Some(weights) => {
            if preset == flowai_runtime::PRESET_SPECIALIST {
                flowai_runtime::eval_validate_specialist_explicit_score_weights(
                    &weights,
                    std::slice::from_ref(&test_case),
                )
                .map_err(py_value_error)?;
            }
            weights
        }
        None => {
            let weights = flowai_runtime::eval_default_score_weights_for_preset_and_test_cases(
                preset,
                std::slice::from_ref(&test_case),
            )
            .map_err(py_value_error)?
            .ok_or_else(|| {
                PyValueError::new_err(format!("preset '{preset}' has no default weights"))
            })?;
            if test_case.final_response.is_some() {
                flowai_runtime::eval_add_default_final_response_weight(weights)
                    .map_err(py_value_error)?
            } else {
                weights
            }
        }
    };
    let scorer = flowai_runtime::eval_scorer_for_eval_test_case_with_config(
        preset,
        weights,
        &test_case,
        options.scorer_config.as_ref(),
        true,
    )
    .map_err(py_value_error)?;

    let scored = scorer.score(&test_case, &output);
    json_value_to_py(py, &serde_json::to_value(scored).map_err(py_runtime_error)?)
}

#[pyfunction]
#[pyo3(signature = (
    spec_json,
    resource_id,
    agent_tools_json,
    tool_specs_json,
    tool_callbacks,
    approval_callbacks,
    action_dispatcher,
    event_hooks,
    interpreter,
    mock_response,
    services,
    data_environment_json
))]
fn create_runtime(
    py: Python<'_>,
    spec_json: String,
    resource_id: String,
    agent_tools_json: String,
    tool_specs_json: String,
    tool_callbacks: Bound<'_, PyDict>,
    approval_callbacks: Bound<'_, PyDict>,
    action_dispatcher: Option<Py<PyAny>>,
    event_hooks: Vec<Py<PyAny>>,
    interpreter: String,
    mock_response: Option<String>,
    services: Py<PyAny>,
    data_environment_json: Option<String>,
) -> PyResult<PyRuntime> {
    let spec: RuntimeSpec = serde_json::from_str(&spec_json).map_err(py_value_error)?;
    let agent_tools: BTreeMap<String, Vec<String>> =
        serde_json::from_str(&agent_tools_json).map_err(py_value_error)?;
    let tool_specs: Vec<HostToolSpec> =
        serde_json::from_str(&tool_specs_json).map_err(py_value_error)?;
    let tool_specs = tool_specs
        .into_iter()
        .map(|spec| (spec.binding_id.clone(), spec))
        .collect::<BTreeMap<_, _>>();
    let tool_callbacks = py_callable_map(&tool_callbacks)?;
    let approval_callbacks = py_callable_map(&approval_callbacks)?;
    let python_loop = Arc::new(StdMutex::new(None));

    let judge_capable = interpreter == "anthropic" && mock_response.is_none();
    let interpreter = interpreter_for(&interpreter, &spec, mock_response)?;
    let trace_sink = Arc::new(RecordingTraceSink::new());
    let mut deps = RuntimeDeps::new(
        interpreter,
        Arc::new(NullEventSink),
        TenantContext::new(TenantId::new_unchecked(resource_id.clone())),
        Arc::new(DashMapKVStore::new()),
    )
    .with_trace_sink(trace_sink.clone())
    .with_judge_capable_interpreter_provider(flowai_runtime::DEFAULT_PROVIDER_KEY, judge_capable);
    deps = apply_data_environment(deps, data_environment_json)?;

    for (name, callback) in approval_callbacks {
        deps = deps.with_approval_predicate(name, py_approval_predicate(callback));
    }

    for (agent, binding_ids) in agent_tools {
        for binding_id in binding_ids {
            let tool_spec = tool_specs.get(&binding_id).ok_or_else(|| {
                PyValueError::new_err(format!("tool binding '{binding_id}' has no spec"))
            })?;
            let callback = tool_callbacks
                .get(&binding_id)
                .or_else(|| tool_callbacks.get(&tool_spec.name))
                .map(|callback| callback.clone_ref(py))
                .ok_or_else(|| {
                    PyValueError::new_err(format!(
                        "tool binding '{}' for tool '{}' has no Python handler",
                        binding_id, tool_spec.name
                    ))
                })?;
            let handler = Arc::new(PyToolHandler {
                name: tool_spec.name.clone(),
                description: tool_spec.description.clone(),
                input_schema: tool_spec.input_schema.clone(),
                callback,
                services: services.clone_ref(py),
                python_loop: python_loop.clone(),
            });
            let mut binding = HostToolBinding::new(handler);
            if let Some(approval) = tool_spec.approval.clone() {
                binding = binding.with_approval(approval);
            }
            deps = deps.with_host_tool(agent.clone(), binding);
        }
    }

    if let Some(callback) = action_dispatcher {
        deps = deps.with_action_dispatcher(Arc::new(PyActionDispatcher {
            callback,
            python_loop: python_loop.clone(),
        }));
    }

    let runtime = Runtime::new(spec, deps).map_err(runtime_error_to_py)?;
    let _ = py;
    Ok(PyRuntime {
        inner: Arc::new(runtime),
        resource_id,
        event_hooks: Arc::new(event_hooks),
        python_loop,
        trace_sink,
    })
}

#[pyfunction]
#[pyo3(signature = (data_environment_json))]
fn data_list_schemas(data_environment_json: String) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let schemas = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        let target = flowai_runtime::storage::build_target_database_from_environment(&env)
            .await
            .map_err(py_value_error)?;
        IntrospectionService::new(target)
            .list_schemas()
            .await
            .map_err(py_value_error)
    })?;
    json_string(&serde_json::json!({ "schemas": schemas }))
}

#[pyfunction]
#[pyo3(signature = (data_environment_json, schema_name=None))]
fn data_list_tables(
    data_environment_json: String,
    schema_name: Option<String>,
) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let tables = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        let target = flowai_runtime::storage::build_target_database_from_environment(&env)
            .await
            .map_err(py_value_error)?;
        let schema =
            schema_name.unwrap_or_else(|| default_schema_for_database(target.database_type()));
        IntrospectionService::new(target)
            .list_tables(&schema)
            .await
            .map_err(py_value_error)
    })?;
    json_string(&serde_json::json!({ "tables": tables }))
}

#[pyfunction]
#[pyo3(signature = (data_environment_json, table_name, schema_name=None))]
fn data_get_table_detail(
    data_environment_json: String,
    table_name: String,
    schema_name: Option<String>,
) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let table = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        let target = flowai_runtime::storage::build_target_database_from_environment(&env)
            .await
            .map_err(py_value_error)?;
        let schema =
            schema_name.unwrap_or_else(|| default_schema_for_database(target.database_type()));
        IntrospectionService::new(target)
            .introspect_table(&schema, &table_name)
            .await
            .map_err(py_value_error)
    })?;
    json_string(&serde_json::json!({ "table": table }))
}

#[pyfunction]
#[pyo3(signature = (data_environment_json, table_name, schema_name=None, limit=None))]
fn data_sample_table(
    data_environment_json: String,
    table_name: String,
    schema_name: Option<String>,
    limit: Option<usize>,
) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let rows = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        let target = flowai_runtime::storage::build_target_database_from_environment(&env)
            .await
            .map_err(py_value_error)?;
        let schema =
            schema_name.unwrap_or_else(|| default_schema_for_database(target.database_type()));
        IntrospectionService::new(target)
            .sample_rows(&schema, &table_name, limit.unwrap_or(25))
            .await
            .map_err(py_value_error)
    })?;
    json_string(&serde_json::json!({ "rows": rows }))
}

#[pyfunction]
#[pyo3(signature = (
    data_environment_json,
    database_id,
    schema_name=None,
    tables_json=None,
    model_id=None,
    sample_size=None
))]
fn data_profile_estimate(
    data_environment_json: String,
    database_id: String,
    schema_name: Option<String>,
    tables_json: Option<String>,
    model_id: Option<String>,
    sample_size: Option<usize>,
) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let tables = parse_tables_json(tables_json)?;
    let result = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        let deps = flowai_runtime::data::ProfilingCommandDeps::new(Arc::new(MockEnricher::new()));
        let tenant_id = env.tenant_id.clone();
        let workspace_id = env.workspace_id.clone();
        flowai_runtime::data::estimate_profiling(
            &flowai_runtime::data::ProfilingEstimateCommand {
                data_environment: env,
                tenant_id,
                workspace_id,
                database_id,
                schema_name,
                tables,
                model_id,
                sample_size,
            },
            &deps,
        )
        .await
        .map_err(py_value_error)
    })?;
    json_string(&serde_json::json!({ "estimate": result }))
}

#[pyfunction]
#[pyo3(signature = (
    data_environment_json,
    database_id,
    table_name,
    schema_name=None,
    model_id=None,
    sample_size=None
))]
fn data_profile_table(
    data_environment_json: String,
    database_id: String,
    table_name: String,
    schema_name: Option<String>,
    model_id: Option<String>,
    sample_size: Option<usize>,
) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let output = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        let deps = flowai_runtime::data::ProfilingCommandDeps::new(Arc::new(MockEnricher::new()));
        let tenant_id = env.tenant_id.clone();
        let workspace_id = env.workspace_id.clone();
        let handle = flowai_runtime::data::profile_table(
            flowai_runtime::data::ProfileTableCommand {
                data_environment: env,
                tenant_id,
                workspace_id,
                database_id,
                schema_name,
                table_name,
                model_id,
                sample_size,
            },
            deps,
        )
        .await
        .map_err(py_value_error)?;
        let job_id = handle.job_id.clone();
        let events = drain_serialized_events(handle.events).await?;
        Ok::<JsonValue, PyErr>(serde_json::json!({ "jobId": job_id, "events": events }))
    })?;
    json_string(&output)
}

#[pyfunction]
#[pyo3(signature = (
    data_environment_json,
    database_id,
    schema_name=None,
    tables_json=None,
    model_id=None,
    sample_size=None
))]
fn data_profile_database(
    data_environment_json: String,
    database_id: String,
    schema_name: Option<String>,
    tables_json: Option<String>,
    model_id: Option<String>,
    sample_size: Option<usize>,
) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let tables = parse_tables_json(tables_json)?;
    let output = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        let deps = flowai_runtime::data::ProfilingCommandDeps::new(Arc::new(MockEnricher::new()));
        let tenant_id = env.tenant_id.clone();
        let workspace_id = env.workspace_id.clone();
        let handle = flowai_runtime::data::profile_database(
            flowai_runtime::data::ProfileDatabaseCommand {
                data_environment: env,
                tenant_id,
                workspace_id,
                database_id,
                schema_name,
                tables,
                model_id,
                sample_size,
            },
            deps,
        )
        .await
        .map_err(py_value_error)?;
        let job_id = handle.job_id.clone();
        let events = drain_serialized_events(handle.events).await?;
        Ok::<JsonValue, PyErr>(serde_json::json!({ "jobId": job_id, "events": events }))
    })?;
    json_string(&output)
}

#[pyfunction]
#[pyo3(signature = (data_environment_json, tenant_id, database_id, source_json, extract_knowledge=false))]
fn data_ingest_knowledge(
    data_environment_json: String,
    tenant_id: String,
    database_id: String,
    source_json: String,
    extract_knowledge: bool,
) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let source = parse_knowledge_source(&source_json)?;
    let output = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        let workspace_id = env.workspace_id.clone();
        let deps = if extract_knowledge {
            flowai_runtime::data::KnowledgeCommandDeps::new()
                .with_enricher(Arc::new(MockEnricher::new()))
        } else {
            flowai_runtime::data::KnowledgeCommandDeps::new()
        };
        let handle = flowai_runtime::data::ingest_knowledge(
            flowai_runtime::data::IngestKnowledgeCommand {
                data_environment: env,
                tenant_id,
                workspace_id,
                database_id,
                source,
                extract_knowledge,
            },
            deps,
        )
        .await
        .map_err(py_value_error)?;
        let job_id = handle.job_id.clone();
        let events = drain_serialized_events(handle.events).await?;
        Ok::<JsonValue, PyErr>(serde_json::json!({ "jobId": job_id, "events": events }))
    })?;
    json_string(&output)
}

#[pyfunction]
#[pyo3(signature = (data_environment_json, tenant_id))]
fn data_list_knowledge_documents(
    data_environment_json: String,
    tenant_id: String,
) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let documents = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        let kv = flowai_runtime::storage::build_kv_store_from_environment(&env)
            .await
            .map_err(py_value_error)?;
        let mut keys = kv
            .list_keys(&tenant_id, "knowledge:doc:")
            .await
            .map_err(py_value_error)?;
        keys.sort();
        let values = kv
            .get_many_json(&tenant_id, &keys)
            .await
            .map_err(py_value_error)?;
        Ok::<Vec<JsonValue>, PyErr>(
            keys.iter()
                .filter_map(|key| values.get(key).cloned())
                .collect(),
        )
    })?;
    json_string(&serde_json::json!({ "documents": documents }))
}

#[pyfunction]
#[pyo3(signature = (data_environment_json, tenant_id))]
fn data_list_knowledge_items(data_environment_json: String, tenant_id: String) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let items = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        let kv = flowai_runtime::storage::build_kv_store_from_environment(&env)
            .await
            .map_err(py_value_error)?;
        let mut keys = kv
            .list_keys(&tenant_id, "knowledge:")
            .await
            .map_err(py_value_error)?;
        keys.retain(|key| {
            key != "knowledge:content_hashes"
                && !key.starts_with("knowledge:doc:")
                && !key.ends_with(":knowledge_ids")
        });
        keys.sort();
        let values = kv
            .get_many_json(&tenant_id, &keys)
            .await
            .map_err(py_value_error)?;
        Ok::<Vec<JsonValue>, PyErr>(
            keys.iter()
                .filter_map(|key| values.get(key).cloned())
                .collect(),
        )
    })?;
    json_string(&serde_json::json!({ "items": items }))
}

#[pyfunction]
#[pyo3(signature = (data_environment_json, query_json))]
fn data_search_catalog(data_environment_json: String, query_json: String) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let request: PySearchCatalogRequest =
        serde_json::from_str(&query_json).map_err(py_value_error)?;
    let result = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        flowai_runtime::data::search_catalog(flowai_runtime::data::SearchCatalogCommand {
            data_environment: env,
            query: request.query,
            mode: request.mode,
            limit: request.limit,
        })
        .await
        .map_err(py_value_error)
    })?;
    json_string(&serde_json::json!({ "search": result }))
}

#[pyfunction]
#[pyo3(signature = (data_environment_json))]
fn data_list_tools(data_environment_json: String) -> PyResult<String> {
    let _ = parse_data_environment(&data_environment_json)?;
    let tools = flowai_runtime::data::list_catalog_tools();
    json_string(&serde_json::json!({ "tools": tools.tools }))
}

#[pyfunction]
#[pyo3(signature = (data_environment_json, tool_id, input_json))]
fn data_execute_tool(
    data_environment_json: String,
    tool_id: String,
    input_json: String,
) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let input: JsonValue = serde_json::from_str(&input_json).map_err(py_value_error)?;
    let result = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        flowai_runtime::data::execute_catalog_tool(
            flowai_runtime::data::ExecuteCatalogToolCommand {
                data_environment: env,
                tool_id,
                input,
            },
        )
        .await
        .map_err(py_value_error)
    })?;
    json_string(&serde_json::json!({ "result": result }))
}

#[pyfunction]
#[pyo3(signature = (data_environment_json, query_json=None))]
fn data_list_metrics(
    data_environment_json: String,
    query_json: Option<String>,
) -> PyResult<String> {
    let env = parse_data_environment(&data_environment_json)?;
    let request = match query_json {
        Some(query_json) => {
            serde_json::from_str::<PyListMetricsRequest>(&query_json).map_err(py_value_error)?
        }
        None => PyListMetricsRequest::default(),
    };
    let result = pyo3_async_runtimes::tokio::get_runtime().block_on(async move {
        flowai_runtime::data::list_metrics(flowai_runtime::data::ListMetricsCommand {
            data_environment: env,
            query: request.query,
            limit: request.limit,
        })
        .await
        .map_err(py_value_error)
    })?;
    json_string(&serde_json::json!({
        "metrics": result.metrics,
        "totalCount": result.total_count
    }))
}

fn parse_data_environment(data_environment_json: &str) -> PyResult<DataEnvironmentConfig> {
    serde_json::from_str(data_environment_json)
        .map_err(|err| PyValueError::new_err(format!("invalid data_environment: {err}")))
}

fn parse_tables_json(tables_json: Option<String>) -> PyResult<Vec<String>> {
    match tables_json {
        Some(value) => serde_json::from_str::<Vec<String>>(&value).map_err(py_value_error),
        None => Ok(Vec::new()),
    }
}

fn parse_knowledge_source(
    source_json: &str,
) -> PyResult<flowai_runtime::data::KnowledgeSourceSpec> {
    match serde_json::from_str::<PyKnowledgeSourceSpec>(source_json).map_err(py_value_error)? {
        PyKnowledgeSourceSpec::LocalDirectory { path, extensions } => {
            Ok(flowai_runtime::data::KnowledgeSourceSpec::LocalDirectory {
                path: PathBuf::from(path),
                extensions,
            })
        }
        PyKnowledgeSourceSpec::S3Bucket {
            bucket,
            prefix,
            region,
        } => Err(PyValueError::new_err(format!(
            "knowledge source s3Bucket is not supported by flowai-runtime yet: bucket={bucket}, prefix={}, region={}",
            prefix.unwrap_or_default(),
            region.unwrap_or_default()
        ))),
    }
}

async fn drain_serialized_events<T: Serialize>(
    mut events: tokio::sync::mpsc::Receiver<T>,
) -> PyResult<Vec<JsonValue>> {
    let mut collected = Vec::new();
    while let Some(event) = events.recv().await {
        collected.push(serde_json::to_value(event).map_err(py_runtime_error)?);
    }
    Ok(collected)
}

fn default_schema_for_database(database_type: DatabaseType) -> String {
    match database_type {
        DatabaseType::SQLite => "main".to_string(),
        DatabaseType::PostgreSQL | DatabaseType::MySQL => "public".to_string(),
    }
}

fn json_string(value: &JsonValue) -> PyResult<String> {
    serde_json::to_string(value).map_err(py_runtime_error)
}

fn apply_data_environment(
    deps: RuntimeDeps,
    data_environment_json: Option<String>,
) -> PyResult<RuntimeDeps> {
    let Some(data_environment_json) = data_environment_json else {
        return Ok(deps);
    };
    let spec: DataEnvironmentConfig = serde_json::from_str(&data_environment_json)
        .map_err(|err| PyValueError::new_err(format!("invalid data_environment: {err}")))?;
    pyo3_async_runtimes::tokio::get_runtime()
        .block_on(flowai_runtime::storage::apply_to_runtime_deps(deps, spec))
        .map_err(|err| PyValueError::new_err(format!("invalid data_environment: {err}")))
}

#[pyclass(name = "ReferenceClient")]
struct PyReferenceClient {
    registry: Arc<ReferenceRegistry>,
    tenant: TenantId,
}

impl PyReferenceClient {
    fn new(registry: Arc<ReferenceRegistry>, tenant: TenantId) -> Self {
        Self { registry, tenant }
    }
}

#[pymethods]
impl PyReferenceClient {
    #[pyo3(signature = (reference, value, glimpse=None))]
    fn create<'py>(
        &self,
        py: Python<'py>,
        reference: Py<PyAny>,
        value: Py<PyAny>,
        glimpse: Option<Py<PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reference_bound = reference.bind(py);
        let value_bound = value.bind(py);
        let (kind, value_json, glimpse_json) =
            reference_create_parts(py, reference_bound, value_bound, glimpse.as_ref())?;
        let registry = self.registry.clone();
        let tenant = self.tenant.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let artifact = registry
                .create(&kind, value_json, glimpse_json.clone(), &tenant)
                .await
                .map_err(py_value_error)?;
            Python::with_gil(|py| {
                json_value_to_py(
                    py,
                    &serde_json::json!({
                        "kind": artifact.kind,
                        "id": artifact.id,
                        "glimpse": glimpse_json,
                    }),
                )
            })
        })
    }

    #[pyo3(signature = (reference))]
    fn resolve<'py>(&self, py: Python<'py>, reference: Py<PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let artifact = artifact_ref_from_py(reference.bind(py))?;
        let registry = self.registry.clone();
        let tenant = self.tenant.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let body = registry
                .resolve(&artifact, &tenant)
                .await
                .map_err(py_value_error)?;
            Python::with_gil(|py| json_value_to_py(py, &body.value))
        })
    }

    #[pyo3(signature = (reference))]
    fn glimpse<'py>(&self, py: Python<'py>, reference: Py<PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let artifact = artifact_ref_from_py(reference.bind(py))?;
        let registry = self.registry.clone();
        let tenant = self.tenant.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let glimpse = registry
                .glimpse(&artifact, &tenant)
                .await
                .map_err(py_value_error)?;
            Python::with_gil(|py| json_value_to_py(py, &glimpse))
        })
    }
}

#[pyclass(name = "ToolContext")]
struct PyToolContext {
    values: Py<PyDict>,
}

#[pymethods]
impl PyToolContext {
    fn __getitem__(&self, py: Python<'_>, key: &str) -> PyResult<Py<PyAny>> {
        self.values
            .bind(py)
            .get_item(key)?
            .map(|value| value.unbind())
            .ok_or_else(|| PyKeyError::new_err(key.to_string()))
    }

    fn __getattr__(&self, py: Python<'_>, key: &str) -> PyResult<Py<PyAny>> {
        self.values
            .bind(py)
            .get_item(key)?
            .map(|value| value.unbind())
            .ok_or_else(|| PyAttributeError::new_err(key.to_string()))
    }

    #[pyo3(signature = (key, default=None))]
    fn get(&self, py: Python<'_>, key: &str, default: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        if let Some(value) = self.values.bind(py).get_item(key)? {
            return Ok(value.unbind());
        }
        Ok(default.unwrap_or_else(|| py.None()))
    }

    fn __contains__(&self, py: Python<'_>, key: &str) -> PyResult<bool> {
        self.values.bind(py).contains(key)
    }

    fn keys(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(self.values.bind(py).call_method0("keys")?.unbind())
    }

    fn as_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(self.values.bind(py).copy()?.unbind().into_any())
    }
}

struct PyToolHandler {
    name: String,
    description: String,
    input_schema: JsonValue,
    callback: Py<PyAny>,
    services: Py<PyAny>,
    python_loop: Arc<StdMutex<Option<Py<PyAny>>>>,
}

#[async_trait]
impl ToolHandler for PyToolHandler {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: self.description.clone(),
            input_schema: self.input_schema.clone(),
        }
    }

    async fn handle(
        &self,
        tool_use_id: &str,
        input: JsonValue,
        env: &ToolEnvironment,
    ) -> ToolCallResult {
        let registry = env.maybe_ext::<ReferenceRegistry>().cloned();
        let tenant = env.resource_id().clone();
        let ctx = match Python::with_gil(|py| {
            build_tool_context(py, tool_use_id, &self.services, registry, tenant)
        }) {
            Ok(ctx) => ctx,
            Err(err) => return ToolCallResult::error(tool_use_id, err.to_string()),
        };
        let callback = Python::with_gil(|py| self.callback.clone_ref(py));
        match call_python_callback(callback, input, ctx, self.python_loop.clone()).await {
            Ok(content) => tool_success_with_ui_channels(tool_use_id, content),
            Err(message) => ToolCallResult::error(tool_use_id, message),
        }
    }
}

fn build_tool_context(
    py: Python<'_>,
    tool_use_id: &str,
    services: &Py<PyAny>,
    references: Option<Arc<ReferenceRegistry>>,
    tenant: TenantId,
) -> PyResult<Py<PyAny>> {
    let values = PyDict::new(py);
    values.set_item("tool_use_id", tool_use_id)?;
    values.set_item("services", services.bind(py))?;
    if let Some(registry) = references {
        values.set_item(
            "references",
            Py::new(py, PyReferenceClient::new(registry, tenant))?,
        )?;
    }
    if let Ok(service_map) = services.bind(py).downcast::<PyDict>() {
        for (name, service) in service_map.iter() {
            values.set_item(name, service)?;
        }
    }
    Ok(Py::new(
        py,
        PyToolContext {
            values: values.unbind(),
        },
    )?
    .into_any())
}

struct PyActionDispatcher {
    callback: Py<PyAny>,
    python_loop: Arc<StdMutex<Option<Py<PyAny>>>>,
}

#[async_trait]
impl ActionDispatcher for PyActionDispatcher {
    type Action = HarnessAction;
    type Context = HarnessActionContext;
    type Error = HarnessActionError;

    async fn dispatch(
        &self,
        actions: &ActionSeq<HarnessAction>,
        ctx: &HarnessActionContext,
    ) -> Result<ExecutionResult, HarnessActionError> {
        let actions = serde_json::to_value(actions.iter().collect::<Vec<_>>())
            .map_err(|err| HarnessActionError::new(err.to_string()))?;
        let ctx = action_context_to_json(ctx);
        let callback = Python::with_gil(|py| self.callback.clone_ref(py));
        let result = call_python_json_callback(callback, actions, ctx, self.python_loop.clone())
            .await
            .map_err(HarnessActionError::new)?;
        if result.is_null() {
            return Ok(ExecutionResult::default());
        }
        validate_action_dispatcher_result(&result)?;
        serde_json::from_value(result).map_err(|err| HarnessActionError::new(err.to_string()))
    }
}

fn action_context_to_json(ctx: &HarnessActionContext) -> JsonValue {
    let mut object = serde_json::Map::new();
    object.insert(
        "resolved_refs".to_string(),
        resolved_refs_by_kind_and_id(ctx),
    );
    JsonValue::Object(object)
}

fn resolved_refs_by_kind_and_id(ctx: &HarnessActionContext) -> JsonValue {
    let mut by_kind = serde_json::Map::new();

    for (artifact_ref, value) in &ctx.resolved_refs {
        let entry = by_kind
            .entry(artifact_ref.kind.clone())
            .or_insert_with(|| JsonValue::Object(serde_json::Map::new()));
        let JsonValue::Object(by_id) = entry else {
            unreachable!("resolved_refs entries are initialized as JSON objects");
        };
        by_id.insert(artifact_ref.id.clone(), value.clone());
    }

    JsonValue::Object(by_kind)
}

fn validate_action_dispatcher_result(result: &JsonValue) -> Result<(), HarnessActionError> {
    let JsonValue::Object(object) = result else {
        return Err(HarnessActionError::new(
            "action_dispatcher must return None or an object with required field \
             `entitiesAffected`, optional `summary`, and optional `details`",
        ));
    };

    for key in object.keys() {
        if !matches!(key.as_str(), "entitiesAffected" | "summary" | "details") {
            return Err(HarnessActionError::new(format!(
                "action_dispatcher returned unexpected field `{key}`; put domain-specific data \
                 under `details`"
            )));
        }
    }

    let Some(entities_affected) = object.get("entitiesAffected") else {
        return Err(HarnessActionError::new(
            "action_dispatcher must return None or an object with required field \
             `entitiesAffected` as a non-negative integer",
        ));
    };
    if entities_affected.as_u64().is_none() {
        return Err(HarnessActionError::new(
            "action_dispatcher field `entitiesAffected` must be a non-negative integer",
        ));
    }

    if let Some(summary) = object.get("summary") {
        if !(summary.is_null() || summary.is_string()) {
            return Err(HarnessActionError::new(
                "action_dispatcher field `summary` must be a string or null",
            ));
        }
    }

    Ok(())
}

#[derive(Clone)]
struct TestInterpreter {
    scripted: bool,
    dispatcher: Option<Arc<dyn ToolDispatcher>>,
}

struct TestInterpreterStreamState {
    pending: VecDeque<StreamPart>,
    phase: TestInterpreterStreamPhase,
}

enum TestInterpreterStreamPhase {
    Start {
        scripted: bool,
        dispatcher: Option<Arc<dyn ToolDispatcher>>,
        prompt: String,
    },
    Calls {
        dispatcher: Arc<dyn ToolDispatcher>,
        calls: VecDeque<(usize, ScriptedToolCall)>,
        multiple: bool,
    },
    Dispatch {
        dispatcher: Arc<dyn ToolDispatcher>,
        calls: VecDeque<(usize, ScriptedToolCall)>,
        multiple: bool,
        tool_use_id: String,
        tool_name: String,
        args: JsonValue,
    },
    Finish,
    Done,
}

impl TestInterpreter {
    fn new(scripted: bool) -> Self {
        Self {
            scripted,
            dispatcher: None,
        }
    }
}

impl ChatInterpreter for TestInterpreter {
    fn interpret(
        &self,
        program: ChatProgram,
        _cancel: CancellationToken,
    ) -> Pin<Box<dyn Stream<Item = StreamPart> + Send>> {
        let state = TestInterpreterStreamState {
            pending: VecDeque::new(),
            phase: TestInterpreterStreamPhase::Start {
                scripted: self.scripted,
                dispatcher: self.dispatcher.clone(),
                prompt: program.conversation().prompt().as_str().to_string(),
            },
        };

        Box::pin(stream::unfold(state, |mut state| async move {
            loop {
                if let Some(part) = state.pending.pop_front() {
                    return Some((part, state));
                }

                let phase = std::mem::replace(&mut state.phase, TestInterpreterStreamPhase::Done);
                match phase {
                    TestInterpreterStreamPhase::Start {
                        scripted,
                        dispatcher,
                        prompt,
                    } => {
                        state.pending.push_back(StreamPart::StepStart);
                        if !scripted {
                            state.phase = TestInterpreterStreamPhase::Finish;
                            continue;
                        }

                        match serde_json::from_str::<ScriptedPrompt>(&prompt) {
                            Ok(script) => {
                                if let Some(dispatcher) = dispatcher {
                                    let calls = script
                                        .into_calls()
                                        .into_iter()
                                        .enumerate()
                                        .collect::<VecDeque<_>>();
                                    let multiple = calls.len() > 1;
                                    state.phase = TestInterpreterStreamPhase::Calls {
                                        dispatcher,
                                        calls,
                                        multiple,
                                    };
                                } else {
                                    state.pending.push_back(StreamPart::error(
                                        "scripted interpreter has no dispatcher",
                                    ));
                                    state.phase = TestInterpreterStreamPhase::Finish;
                                }
                            }
                            Err(_) => {
                                state.pending.push_back(StreamPart::text(prompt));
                                state.phase = TestInterpreterStreamPhase::Finish;
                            }
                        }
                    }
                    TestInterpreterStreamPhase::Calls {
                        dispatcher,
                        mut calls,
                        multiple,
                    } => {
                        let Some((index, call)) = calls.pop_front() else {
                            state.phase = TestInterpreterStreamPhase::Finish;
                            continue;
                        };

                        let tool_use_id = if multiple {
                            format!("scripted-tool-{}", index + 1)
                        } else {
                            "scripted-tool-1".to_string()
                        };
                        let tool_name = call.tool;
                        let args = call.args;
                        state.phase = TestInterpreterStreamPhase::Dispatch {
                            dispatcher,
                            calls,
                            multiple,
                            tool_use_id: tool_use_id.clone(),
                            tool_name: tool_name.clone(),
                            args: args.clone(),
                        };
                        return Some((StreamPart::tool_call(tool_use_id, tool_name, args), state));
                    }
                    TestInterpreterStreamPhase::Dispatch {
                        dispatcher,
                        calls,
                        multiple,
                        tool_use_id,
                        tool_name,
                        args,
                    } => {
                        let result = dispatcher
                            .dispatch(&tool_name, &tool_use_id, args.clone())
                            .await;
                        let content = result.content.clone();
                        if !result.is_error {
                            state
                                .pending
                                .push_back(StreamPart::text(content.to_string()));
                        }
                        state.phase = TestInterpreterStreamPhase::Calls {
                            dispatcher,
                            calls,
                            multiple,
                        };
                        return Some((
                            StreamPart::tool_result(tool_use_id, tool_name, args, content),
                            state,
                        ));
                    }
                    TestInterpreterStreamPhase::Finish => {
                        state.phase = TestInterpreterStreamPhase::Done;
                        return Some((
                            StreamPart::finish(FinishReason::Stop, Default::default()),
                            state,
                        ));
                    }
                    TestInterpreterStreamPhase::Done => return None,
                }
            }
        }))
    }

    fn with_tool_dispatcher(
        self: Arc<Self>,
        dispatcher: Arc<dyn ToolDispatcher>,
    ) -> Option<Arc<dyn ChatInterpreter>> {
        Some(Arc::new(Self {
            scripted: self.scripted,
            dispatcher: Some(dispatcher),
        }))
    }
}

fn interpreter_for(
    kind: &str,
    spec: &RuntimeSpec,
    mock_response: Option<String>,
) -> PyResult<Arc<dyn ChatInterpreter>> {
    if let Some(response) = mock_response {
        if kind != "noop" {
            return Err(PyValueError::new_err(
                "mock_response requires interpreter='noop'",
            ));
        }
        return Ok(Arc::new(MockChatInterpreter::new(response).with_latency(0)));
    }

    match kind {
        "noop" => Ok(Arc::new(TestInterpreter::new(false))),
        "scripted" => Ok(Arc::new(TestInterpreter::new(true))),
        "anthropic" => {
            let provider = spec.providers.get("anthropic").ok_or_else(|| {
                PyValueError::new_err("interpreter='anthropic' requires an anthropic provider")
            })?;
            let api_key = provider
                .config
                .get("apiKey")
                .and_then(JsonValue::as_str)
                .map(str::to_string)
                .or_else(|| {
                    provider
                        .config
                        .get("apiKeyEnv")
                        .and_then(JsonValue::as_str)
                        .and_then(|name| std::env::var(name).ok())
                })
                .ok_or_else(|| {
                    PyValueError::new_err(
                        "anthropic provider requires apiKey or apiKeyEnv resolving to an environment variable",
                    )
                })?;
            let interpreter = RigAnthropicChatInterpreter::new(api_key)
                .map_err(|err| PyRuntimeError::new_err(err.to_string()))?;
            Ok(Arc::new(interpreter))
        }
        other => Err(PyValueError::new_err(format!(
            "unknown interpreter '{other}'; expected noop, scripted, or anthropic"
        ))),
    }
}

fn py_approval_predicate(callback: Py<PyAny>) -> agent_fw_agent::approval::ApprovalPredicate {
    Arc::new(move |ctx| {
        let input = ctx.input.clone();
        let context = serde_json::json!({
            "kind": ctx.kind.to_string(),
            "target": ctx.target,
            "tenant": ctx.tenant.as_str(),
        });
        Python::with_gil(|py| -> PyResult<bool> {
            let args = json_value_to_py(py, &input)?;
            let ctx_obj = json_value_to_py(py, &context)?;
            callback.call1(py, (args, ctx_obj))?.extract(py)
        })
        .unwrap_or_else(|err| {
            Python::with_gil(|py| err.print(py));
            true
        })
    })
}

enum CallbackReturn {
    Value(Py<PyAny>),
    ThreadsafeFuture(Py<PyAny>),
}

async fn call_python_json_callback(
    callback: Py<PyAny>,
    args: JsonValue,
    ctx: JsonValue,
    python_loop: Arc<StdMutex<Option<Py<PyAny>>>>,
) -> Result<JsonValue, String> {
    let ctx = Python::with_gil(|py| json_value_to_py(py, &ctx)).map_err(|err| err.to_string())?;
    call_python_callback(callback, args, ctx, python_loop).await
}

async fn call_python_callback(
    callback: Py<PyAny>,
    args: JsonValue,
    ctx: Py<PyAny>,
    python_loop: Arc<StdMutex<Option<Py<PyAny>>>>,
) -> Result<JsonValue, String> {
    let returned = Python::with_gil(|py| -> PyResult<CallbackReturn> {
        let args = json_value_to_py(py, &args)?;
        let result = callback.call1(py, (args, ctx))?;
        let is_awaitable = result.bind(py).hasattr("__await__")?;
        if is_awaitable {
            let loop_obj = {
                let guard = python_loop
                    .lock()
                    .map_err(|_| PyRuntimeError::new_err("python loop lock poisoned"))?;
                guard.as_ref().map(|loop_obj| loop_obj.clone_ref(py))
            }
            .ok_or_else(|| {
                PyRuntimeError::new_err(
                    "async Python callback requires the runtime stream to be iterated from asyncio",
                )
            })?;
            let asyncio = py.import("asyncio")?;
            let future = asyncio.call_method1(
                "run_coroutine_threadsafe",
                (result.bind(py), loop_obj.bind(py)),
            )?;
            Ok(CallbackReturn::ThreadsafeFuture(future.unbind()))
        } else {
            Ok(CallbackReturn::Value(result))
        }
    })
    .map_err(|err| err.to_string())?;

    let obj = match returned {
        CallbackReturn::Value(obj) => obj,
        CallbackReturn::ThreadsafeFuture(future) => tokio::task::spawn_blocking(move || {
            Python::with_gil(|py| future.call_method0(py, "result"))
        })
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| err.to_string())?,
    };

    Python::with_gil(|py| py_any_to_json_value(py, obj.bind(py))).map_err(|err| err.to_string())
}

fn capture_python_loop(py: Python<'_>, python_loop: &Arc<StdMutex<Option<Py<PyAny>>>>) {
    let Ok(asyncio) = py.import("asyncio") else {
        return;
    };
    let Ok(loop_obj) = asyncio.call_method0("get_running_loop") else {
        return;
    };
    if let Ok(mut guard) = python_loop.lock() {
        *guard = Some(loop_obj.unbind());
    }
}

fn duration_from_secs(seconds: f64) -> PyResult<std::time::Duration> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(PyValueError::new_err(
            "call_timeout_secs must be a positive finite number",
        ));
    }
    Ok(std::time::Duration::from_secs_f64(seconds))
}

fn parse_bind_addr(host: &str, port: u16) -> PyResult<std::net::SocketAddr> {
    use std::net::ToSocketAddrs;

    let raw = format!("{host}:{port}");
    raw.to_socket_addrs()
        .map_err(|err| PyValueError::new_err(format!("invalid MCP bind address {raw}: {err}")))?
        .next()
        .ok_or_else(|| PyValueError::new_err(format!("invalid MCP bind address {raw}")))
}

fn tool_success_with_ui_channels(tool_use_id: &str, mut content: JsonValue) -> ToolCallResult {
    let mut approval_dsl = None;
    let mut display_summary = None;
    if let Some(obj) = content.as_object_mut() {
        if let Some(value) = obj.remove("approvalDsl") {
            approval_dsl = value
                .as_str()
                .map(String::from)
                .or_else(|| serde_json::to_string(&value).ok());
        }
        if let Some(value) = obj.remove("displaySummary") {
            display_summary = value.as_str().map(String::from);
        }
        obj.remove("_cardEmitted");
    }
    let mut result = ToolCallResult::success(tool_use_id, content);
    result.approval_dsl = approval_dsl;
    result.display_summary = display_summary;
    result
}

fn py_callable_map(dict: &Bound<'_, PyDict>) -> PyResult<BTreeMap<String, Py<PyAny>>> {
    let mut result = BTreeMap::new();
    for (key, value) in dict.iter() {
        result.insert(key.extract::<String>()?, value.unbind());
    }
    Ok(result)
}

fn reference_create_parts(
    py: Python<'_>,
    reference: &Bound<'_, PyAny>,
    value: &Bound<'_, PyAny>,
    explicit_glimpse: Option<&Py<PyAny>>,
) -> PyResult<(String, JsonValue, JsonValue)> {
    let kind = reference_kind(reference)?;
    let value_json = py_any_to_json_value(py, value)?;
    let glimpse_json = match explicit_glimpse {
        Some(glimpse) => py_any_to_json_value(py, glimpse.bind(py))?,
        None => reference_glimpse_value(py, reference, value)?,
    };
    Ok((kind, value_json, glimpse_json))
}

fn reference_kind(reference: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(kind) = reference.extract::<String>() {
        if kind.is_empty() {
            return Err(PyValueError::new_err("reference kind must not be empty"));
        }
        return Ok(kind);
    }
    py_string_member(reference, "name")
}

fn reference_glimpse_value(
    py: Python<'_>,
    reference: &Bound<'_, PyAny>,
    value: &Bound<'_, PyAny>,
) -> PyResult<JsonValue> {
    if reference.extract::<String>().is_ok() {
        return Ok(serde_json::json!({}));
    }
    if !reference.hasattr("glimpse")? {
        return Ok(serde_json::json!({}));
    }
    let callback = reference.getattr("glimpse")?;
    if callback.is_none() {
        return Ok(serde_json::json!({}));
    }
    if !callback.is_callable() {
        return Err(PyValueError::new_err(
            "ReferenceSpec.glimpse must be callable or None",
        ));
    }
    let glimpse = callback.call1((value,))?;
    py_any_to_json_value(py, &glimpse)
}

fn artifact_ref_from_py(reference: &Bound<'_, PyAny>) -> PyResult<ArtifactRef> {
    Ok(ArtifactRef {
        kind: py_string_member(reference, "kind")?,
        id: py_string_member(reference, "id")?,
    })
}

fn py_string_member(value: &Bound<'_, PyAny>, key: &str) -> PyResult<String> {
    if let Ok(dict) = value.downcast::<PyDict>() {
        if let Some(item) = dict.get_item(key)? {
            let extracted = item.extract::<String>()?;
            if extracted.is_empty() {
                return Err(PyValueError::new_err(format!("{key} must not be empty")));
            }
            return Ok(extracted);
        }
    }
    if value.hasattr(key)? {
        let extracted = value.getattr(key)?.extract::<String>()?;
        if extracted.is_empty() {
            return Err(PyValueError::new_err(format!("{key} must not be empty")));
        }
        return Ok(extracted);
    }
    Err(PyValueError::new_err(format!(
        "reference must provide string field '{key}'"
    )))
}

fn json_value_to_py(py: Python<'_>, value: &JsonValue) -> PyResult<Py<PyAny>> {
    let json = serde_json::to_string(value).map_err(py_runtime_error)?;
    let json_module = py.import("json")?;
    Ok(json_module.call_method1("loads", (json,))?.unbind())
}

fn py_any_to_json_value(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<JsonValue> {
    if value.hasattr("model_dump")? {
        let kwargs = PyDict::new(py);
        kwargs.set_item("mode", "json")?;
        let dumped = value.call_method("model_dump", (), Some(&kwargs))?;
        return py_any_to_json_value(py, &dumped);
    }
    let json_module = py.import("json")?;
    let json = json_module
        .call_method1("dumps", (value,))?
        .extract::<String>()?;
    serde_json::from_str(&json).map_err(py_runtime_error)
}

fn py_model_or_json_to_string(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(json) = value.extract::<String>() {
        return Ok(json);
    }
    if value.hasattr("model_dump")? {
        let kwargs = PyDict::new(py);
        kwargs.set_item("by_alias", true)?;
        kwargs.set_item("mode", "json")?;
        let dumped = value.call_method("model_dump", (), Some(&kwargs))?;
        let json_module = py.import("json")?;
        return json_module
            .call_method1("dumps", (dumped,))?
            .extract::<String>();
    }
    let json_module = py.import("json")?;
    json_module
        .call_method1("dumps", (value,))?
        .extract::<String>()
}

fn runtime_error_to_py(err: RuntimeError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

fn eval_runner_error_to_py(err: flowai_runtime::EvalRunnerError) -> PyErr {
    match err {
        flowai_runtime::EvalRunnerError::TenantMismatch { .. }
        | flowai_runtime::EvalRunnerError::ModeNotSupported(_)
        | flowai_runtime::EvalRunnerError::Scorer(_)
        | flowai_runtime::EvalRunnerError::ScoreWeights(_) => {
            PyValueError::new_err(err.to_string())
        }
        flowai_runtime::EvalRunnerError::SampleExecution(_)
        | flowai_runtime::EvalRunnerError::Cancelled => PyRuntimeError::new_err(err.to_string()),
    }
}

fn preset_matches_mode(preset: &str, mode: EvalMode) -> bool {
    matches!(
        (preset, mode),
        (flowai_runtime::PRESET_PLANNER, EvalMode::Planner)
            | (flowai_runtime::PRESET_EXECUTOR, EvalMode::Executor)
            | (flowai_runtime::PRESET_SEQUENTIAL, EvalMode::Sequential)
            | (flowai_runtime::PRESET_SPECIALIST, EvalMode::Specialist)
            | (
                flowai_runtime::PRESET_TEST_CASE_BUILDER,
                EvalMode::TestCaseBuilder
            )
    )
}

fn preset_for_mode(mode: EvalMode) -> &'static str {
    match mode {
        EvalMode::Planner => flowai_runtime::PRESET_PLANNER,
        EvalMode::Executor => flowai_runtime::PRESET_EXECUTOR,
        EvalMode::Sequential => flowai_runtime::PRESET_SEQUENTIAL,
        EvalMode::Specialist => flowai_runtime::PRESET_SPECIALIST,
        EvalMode::TestCaseBuilder => flowai_runtime::PRESET_TEST_CASE_BUILDER,
    }
}

fn eval_mode_wire_name(mode: EvalMode) -> &'static str {
    match mode {
        EvalMode::Planner => "planner",
        EvalMode::Executor => "executor",
        EvalMode::Sequential => "sequential",
        EvalMode::Specialist => "specialist",
        EvalMode::TestCaseBuilder => "testCaseBuilder",
    }
}

fn runtime_mcp_error_to_py(err: RuntimeMcpError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

fn py_value_error(err: impl std::fmt::Display) -> PyErr {
    let message = err.to_string();
    if looks_like_schema_drift_error(&message) {
        return PyValueError::new_err(stale_extension_message(&message));
    }
    PyValueError::new_err(message)
}

fn py_runtime_error(err: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(err.to_string())
}

fn looks_like_schema_drift_error(message: &str) -> bool {
    message.contains("unknown field")
}

fn stale_extension_message(raw: &str) -> String {
    format!(
        "flowai_harness._internal is stale or schema-incompatible with the Python facade; rebuild/reinstall the extension. Raw native error: {raw}"
    )
}

#[pyfunction]
fn native_api_version() -> u32 {
    NATIVE_API_VERSION
}

/// Private Python module: `flowai_harness._internal`.
#[pymodule]
fn _internal(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("NATIVE_API_VERSION", NATIVE_API_VERSION)?;
    m.add_class::<PyRuntime>()?;
    m.add_class::<PyRuntimeEventStream>()?;
    m.add_class::<PyEvalEventStream>()?;
    m.add_class::<PyReferenceClient>()?;
    m.add_class::<PyToolContext>()?;
    m.add_function(wrap_pyfunction!(run_cli, m)?)?;
    m.add_function(wrap_pyfunction!(native_api_version, m)?)?;
    m.add_function(wrap_pyfunction!(describe_toolkit_tools, m)?)?;
    m.add_function(wrap_pyfunction!(score_eval_sample, m)?)?;
    m.add_function(wrap_pyfunction!(create_runtime, m)?)?;
    m.add_function(wrap_pyfunction!(data_list_schemas, m)?)?;
    m.add_function(wrap_pyfunction!(data_list_tables, m)?)?;
    m.add_function(wrap_pyfunction!(data_get_table_detail, m)?)?;
    m.add_function(wrap_pyfunction!(data_sample_table, m)?)?;
    m.add_function(wrap_pyfunction!(data_profile_estimate, m)?)?;
    m.add_function(wrap_pyfunction!(data_profile_table, m)?)?;
    m.add_function(wrap_pyfunction!(data_profile_database, m)?)?;
    m.add_function(wrap_pyfunction!(data_ingest_knowledge, m)?)?;
    m.add_function(wrap_pyfunction!(data_list_knowledge_documents, m)?)?;
    m.add_function(wrap_pyfunction!(data_list_knowledge_items, m)?)?;
    m.add_function(wrap_pyfunction!(data_search_catalog, m)?)?;
    m.add_function(wrap_pyfunction!(data_list_tools, m)?)?;
    m.add_function(wrap_pyfunction!(data_execute_tool, m)?)?;
    m.add_function(wrap_pyfunction!(data_list_metrics, m)?)?;
    m.add(
        "__all__",
        vec![
            "run_cli",
            "native_api_version",
            "describe_toolkit_tools",
            "score_eval_sample",
            "create_runtime",
            "data_list_schemas",
            "data_list_tables",
            "data_get_table_detail",
            "data_sample_table",
            "data_profile_estimate",
            "data_profile_table",
            "data_profile_database",
            "data_ingest_knowledge",
            "data_list_knowledge_documents",
            "data_list_knowledge_items",
            "data_search_catalog",
            "data_list_tools",
            "data_execute_tool",
            "data_list_metrics",
            "PyRuntime",
            "PyRuntimeEventStream",
            "PyEvalEventStream",
            "ToolContext",
        ],
    )?;
    Ok(())
}
