__all__: list[str]
NATIVE_API_VERSION: int

def native_api_version() -> int: ...

def create_runtime(
    spec_json: str,
    resource_id: str,
    agent_tools_json: str,
    tool_specs_json: str,
    tool_callbacks: dict[str, object],
    approval_callbacks: dict[str, object],
    action_dispatcher: object | None,
    event_hooks: list[object],
    interpreter: str,
    mock_response: str | None,
    services: dict[str, object],
    data_environment_json: str | None,
) -> PyRuntime: ...

def describe_toolkit_tools(toolkit_json: str, agent_json: str) -> str: ...
def score_eval_sample(
    test_case_json: str,
    output_json: str,
    options_json: str,
) -> dict[str, object]: ...

class PyRuntime:
    """Native Flow AI runtime handle returned by `create_runtime(...)`.

    Owns agent orchestration, provider routing, approval gates, plan
    lifecycle, reference storage, eval execution, and MCP tool serving.
    Exported as `flowai_harness.Runtime`.
    """

    resource_id: str

    def query(
        self,
        prompt: str,
        thread_id: str,
        resume: str | None = None,
    ) -> PyRuntimeEventStream:
        """Run a coordinator turn and return an async-iterable event stream.

        Args:
            prompt: User prompt for this turn.
            thread_id: Conversation thread identifier.
            resume: Optional resume token continuing an interrupted run.

        Returns:
            An async-iterable stream yielding runtime event dicts.
        """

    def run_specialist( # ! FIXME: This is probably not needed - We need: specialist as subagent and also ability to run single agent right?
        self,
        specialist: str,
        prompt: str,
        thread_id: str | None = None,
    ) -> PyRuntimeEventStream:
        """Dispatch a specialist agent directly, bypassing the coordinator.

        Args:
            specialist: Name of the registered specialist agent.
            prompt: User prompt for the specialist.
            thread_id: Optional conversation thread identifier.

        Returns:
            An async-iterable stream yielding runtime event dicts.
        """

    async def run_eval(
        self,
        eval_request: str | object,
    ) -> dict[str, object]:
        """Run an eval to completion and return the eval artifact.

        Args:
            eval_request: `EvalRequest` model, mapping, or JSON string.

        Returns:
            The eval artifact dict (validate with `EvalArtifact`).

        Raises:
            ValueError: If the request cannot be parsed.
            RuntimeError: If the eval run fails.
        """

    def stream_eval(
        self,
        eval_request: str | object,
    ) -> PyEvalEventStream:
        """Run an eval and stream progress event envelopes.

        Args:
            eval_request: `EvalRequest` model, mapping, or JSON string.

        Returns:
            An async-iterable stream yielding eval event envelope dicts
            (validate with `HarnessEvalEventEnvelope`).
        """

    def get_trace(
        self,
        trace_id: str,
    ) -> dict[str, object] | None:
        """Return one recorded trace by id, or None when not found.

        Args:
            trace_id: Trace identifier from an eval artifact or event.
        """

    def list_traces(
        self,
        eval_run_id: str | None = None,
        test_case_id: str | None = None,
        thread_id: str | None = None,
    ) -> list[dict[str, object]]:
        """List recorded traces, optionally filtered.

        Args:
            eval_run_id: Only traces recorded for this eval run.
            test_case_id: Only traces recorded for this test case.
            thread_id: Only traces recorded for this thread.

        Returns:
            A list of trace dicts matching every supplied filter.
        """

    async def create_reference(
        self,
        reference: str | object,
        value: object,
        glimpse: object | None = None,
    ) -> dict[str, object]:
        """Store a value and return its typed reference envelope.

        Args:
            reference: `ReferenceSpec` or reference kind name. A spec's
                Python `glimpse` callback runs once before storing.
            value: JSON-serializable payload to store.
            glimpse: Explicit glimpse value; defaults to `{}` when neither a
                callback nor a value is supplied.
        """

    async def resolve_reference(
        self,
        reference: object,
    ) -> object:
        """Resolve a reference envelope to its full stored payload."""

    async def reference_glimpse(
        self,
        reference: object,
    ) -> object:
        """Return the cached glimpse for a reference without resolving it."""

    async def respond_to_approval(
        self,
        approval_id: str,
        outcome: str,
        feedback: str | None = None,
        partial: object | None = None,
    ) -> None:
        """Resolve a pending approval gate.

        Args:
            approval_id: Approval id from the approval event.
            outcome: `"approve"`, `"reject"`, or `"revise"`.
            feedback: Optional reviewer feedback forwarded to the agent.
            partial: Optional partial revision payload.

        Raises:
            ValueError: If `outcome` is not a supported value.
        """

    def list_mcp_tools(
        self,
        agent: str,
        *,
        expose_agent_tools: bool = False,
    ) -> list[dict[str, object]]:
        """Return MCP tool descriptors for one runtime agent.

        Args:
            agent: Runtime agent whose direct tools are listed.
            expose_agent_tools: Reserved for future recursive agent tools.
        """

    async def serve_mcp_stdio(
        self,
        agent: str,
        *,
        thread_id: str | None = None,
        call_timeout_secs: float = 30.0,
        expose_agent_tools: bool = False,
    ) -> None:
        """Serve one agent's tools over MCP stdio until the client disconnects.

        Args:
            agent: Runtime agent whose tools are exposed.
            thread_id: Optional fixed thread id for runtime tool dispatch.
            call_timeout_secs: Per tool-call timeout in seconds.
            expose_agent_tools: Reserved for future recursive agent tools.
        """

    async def serve_mcp_http(
        self,
        agent: str,
        *,
        host: str = "127.0.0.1",
        port: int = 8765,
        path: str = "/mcp",
        transport: str = "streamable-http",
        thread_id: str | None = None,
        call_timeout_secs: float = 30.0,
        expose_agent_tools: bool = False,
        allowed_origins: list[str] | None = None,
        require_origin: bool = True,
        require_auth: bool = True,
        auth_token: str | None = None,
    ) -> None:
        """Serve one agent's tools over MCP Streamable HTTP.

        Args:
            agent: Runtime agent whose tools are exposed.
            host: Host to bind.
            port: Port to bind.
            path: HTTP endpoint path.
            transport: Only `"streamable-http"` is supported.
            thread_id: Optional fixed thread id for runtime tool dispatch.
            call_timeout_secs: Per tool-call timeout in seconds.
            expose_agent_tools: Reserved for future recursive agent tools.
            allowed_origins: Additional `Origin` header values to accept.
            require_origin: Validate browser `Origin` headers.
            require_auth: Require bearer/header authentication.
            auth_token: Required bearer/header token for Streamable HTTP when
                authentication is enabled.

        Raises:
            ValueError: If `transport` is not `"streamable-http"`.
            RuntimeError: If the server fails to bind or serving fails.
        """

class PyRuntimeEventStream:
    def __aiter__(self) -> PyRuntimeEventStream: ...
    async def __anext__(self) -> dict[str, object]: ...

class PyEvalEventStream:
    def __aiter__(self) -> PyEvalEventStream: ...
    async def __anext__(self) -> dict[str, object]: ...

class ReferenceClient:
    async def create(
        self,
        reference: str | object,
        value: object,
        glimpse: object | None = None,
    ) -> dict[str, object]: ...
    async def resolve(self, reference: object) -> object: ...
    async def glimpse(self, reference: object) -> object: ...

class ToolContext:
    references: ReferenceClient

    def __getitem__(self, key: str) -> object: ...
    def __getattr__(self, key: str) -> object: ...
    def get(self, key: str, default: object | None = None) -> object: ...
    def __contains__(self, key: str) -> bool: ...
    def keys(self) -> object: ...
    def as_dict(self) -> dict[str, object]: ...

Runtime = PyRuntime
