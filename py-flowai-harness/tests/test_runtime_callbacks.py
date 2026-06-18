import asyncio
import json

import pytest
from pydantic import BaseModel

from flowai_harness import (
    AgentSpec,
    Runtime,
    create_runtime,
    define_reference,
    define_runtime,
    define_specialist,
    define_tenant,
    define_tool,
)


async def _collect(stream):
    events = []
    async for event in stream:
        events.append(event)
    return events


def _spec_with_agent(agent: AgentSpec):
    agents = [agent]
    if agent.name != "worker" and "worker" in agent.routes:
        agents.append(
            define_specialist(
                name="worker",
                model="claude-sonnet-4-6",
                prompt="Handle delegated test requests.",
            )
        )

    return define_runtime(
        tenant=define_tenant("acme", "v1"),
        agents=agents,
        providers={"anthropic": {"apiKey": "unused"}},
    )


def test_create_runtime_returns_native_handle_and_query_streams_events():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Return a minimal response.",
        routes=["worker"],
    )
    runtime = create_runtime(
        _spec_with_agent(coordinator),
        interpreter="noop",
    )

    events = asyncio.run(_collect(runtime.query("hello", thread_id="thread-1")))

    assert type(runtime).__name__ == "PyRuntime"
    assert any(event["type"] == "step-start" for event in events)
    assert any(event["type"] == "finish" for event in events)


def test_create_runtime_exports_runtime_alias_and_public_testing_mock_response():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Return a minimal response.",
        routes=["worker"],
    )

    runtime = create_runtime(
        _spec_with_agent(coordinator),
        testing={"mock_response": "mocked runtime response"},
    )

    events = asyncio.run(_collect(runtime.query("hello", thread_id="thread-1")))

    assert isinstance(runtime, Runtime)
    assert any(
        event["type"] == "text" and "mocked runtime response" in event["text"]
        for event in events
    )


def test_create_runtime_testing_rejects_non_default_interpreter():
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Return a minimal response.",
        routes=["worker"],
    )

    with pytest.raises(ValueError, match="testing.*interpreter"):
        create_runtime(
            _spec_with_agent(coordinator),
            testing={"mock_response": "mocked runtime response"},
            interpreter="scripted",
        )


@pytest.mark.parametrize(
    ("testing", "error_type", "message"),
    [
        ({"mock_response": 123}, TypeError, "must be a string"),
        ({"unknown_key": "mocked"}, ValueError, "unknown_key"),
        ({}, ValueError, "mock_response"),
    ],
)
def test_create_runtime_testing_validates_shape(testing, error_type, message):
    coordinator = AgentSpec(
        name="coordinator",
        role="coordinator",
        model="claude-sonnet-4-6",
        system_prompt="Return a minimal response.",
        routes=["worker"],
    )

    with pytest.raises(error_type, match=message):
        create_runtime(
            _spec_with_agent(coordinator),
            testing=testing,
        )


def test_python_tool_callback_is_invoked_through_rust_specialist_dispatch():
    calls = []

    @define_tool("echo", {"value": str}, approval="never")
    async def echo(args, ctx):
        calls.append((args, ctx["tool_use_id"]))
        return {"echo": args["value"]}

    specialist = define_specialist(
        name="worker",
        model="claude-sonnet-4-6",
        prompt="Use the requested tool.",
        tools=[echo],
    )
    runtime = create_runtime(_spec_with_agent(specialist), interpreter="scripted")

    prompt = json.dumps({"tool": "echo", "args": {"value": "hello"}})
    events = asyncio.run(_collect(runtime.run_specialist("worker", prompt, thread_id="thread-1")))

    assert calls == [({"value": "hello"}, "scripted-tool-1")]
    assert any(
        event["type"] == "tool-invocation"
        and event["toolName"] == "echo"
        and event["state"] == "result"
        and event["result"] == {"echo": "hello"}
        for event in events
    )


def test_python_tool_callback_receives_runtime_services_in_context():
    class ProductService:
        async def search(self, query, *, limit):
            return [f"{query}-{index}" for index in range(limit)]

    calls = []

    @define_tool("search_products", {"query": str, "limit": int}, approval="never")
    async def search_products(args, ctx):
        products = await ctx.acme.search(args["query"], limit=args["limit"])
        calls.append((ctx["tool_use_id"], ctx.get("acme") is ctx.acme))
        return {"products": products, "hasServices": "services" in ctx}

    specialist = define_specialist(
        name="worker",
        model="claude-sonnet-4-6",
        prompt="Use the requested tool.",
        tools=[search_products],
    )
    runtime = create_runtime(
        _spec_with_agent(specialist),
        services={"acme": ProductService()},
        interpreter="scripted",
    )

    prompt = json.dumps({"tool": "search_products", "args": {"query": "sku", "limit": 2}})
    events = asyncio.run(_collect(runtime.run_specialist("worker", prompt, thread_id="thread-1")))

    assert calls == [("scripted-tool-1", True)]
    assert any(
        event["type"] == "tool-invocation"
        and event["toolName"] == "search_products"
        and event["state"] == "result"
        and event["result"] == {"products": ["sku-0", "sku-1"], "hasServices": True}
        for event in events
    )


class ProductSetPayload(BaseModel):
    product_ids: list[str]


def test_runtime_can_create_resolve_and_glimpse_reference_from_python():
    glimpse_calls = []

    product_set = define_reference(
        "ProductSet",
        ProductSetPayload,
        glimpse=lambda value: glimpse_calls.append(list(value.product_ids))
        or {
            "productCount": len(value.product_ids),
            "preview": value.product_ids[:2],
        },
    )
    specialist = define_specialist(
        name="worker",
        model="claude-sonnet-4-6",
        prompt="Use tools when requested.",
        toolkits=["references"],
    )
    runtime = create_runtime(
        define_runtime(
            tenant=define_tenant("acme", "v1"),
            agents=[specialist],
            references=[product_set],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        interpreter="scripted",
    )

    async def flow():
        ref = await runtime.create_reference(
            product_set,
            ProductSetPayload(product_ids=["sku-1", "sku-2", "sku-3"]),
        )
        resolved = await runtime.resolve_reference(ref)
        cached_glimpse = await runtime.reference_glimpse(ref)
        return ref, resolved, cached_glimpse

    ref, resolved, cached_glimpse = asyncio.run(flow())

    assert ref["kind"] == "ProductSet"
    assert isinstance(ref["id"], str)
    assert ref["glimpse"] == {"productCount": 3, "preview": ["sku-1", "sku-2"]}
    assert resolved == {"product_ids": ["sku-1", "sku-2", "sku-3"]}
    assert cached_glimpse == {"productCount": 3, "preview": ["sku-1", "sku-2"]}
    assert glimpse_calls == [["sku-1", "sku-2", "sku-3"]]


def test_python_tool_context_can_create_reference_readable_by_references_toolkit():
    product_set = define_reference(
        "ProductSet",
        ProductSetPayload,
        glimpse=lambda value: {"productCount": len(value.product_ids)},
    )
    created_refs = []

    @define_tool("make_product_set", {"product_ids": list[str]}, approval="never")
    async def make_product_set(args, ctx):
        ref = await ctx.references.create(
            product_set,
            ProductSetPayload(product_ids=args["product_ids"]),
        )
        created_refs.append(ref)
        return {"ref": {"kind": ref["kind"], "id": ref["id"]}, "glimpse": ref["glimpse"]}

    specialist = define_specialist(
        name="worker",
        model="claude-sonnet-4-6",
        prompt="Use tools when requested.",
        tools=[make_product_set],
        toolkits=["references"],
    )
    runtime = create_runtime(
        define_runtime(
            tenant=define_tenant("acme", "v1"),
            agents=[specialist],
            references=[product_set],
            providers={"anthropic": {"apiKey": "unused"}},
        ),
        interpreter="scripted",
    )

    create_prompt = json.dumps(
        {"tool": "make_product_set", "args": {"product_ids": ["sku-1", "sku-2"]}}
    )
    create_events = asyncio.run(
        _collect(runtime.run_specialist("worker", create_prompt, thread_id="thread-1"))
    )

    assert created_refs == [
        {
            "kind": "ProductSet",
            "id": created_refs[0]["id"],
            "glimpse": {"productCount": 2},
        }
    ]
    assert any(
        event["type"] == "tool-invocation"
        and event["toolName"] == "make_product_set"
        and event["state"] == "result"
        and event["result"]["glimpse"] == {"productCount": 2}
        for event in create_events
    )

    ref = created_refs[0]
    resolve_prompt = json.dumps(
        {"tool": "resolveRef", "args": {"kind": ref["kind"], "id": ref["id"]}}
    )
    resolve_events = asyncio.run(
        _collect(runtime.run_specialist("worker", resolve_prompt, thread_id="thread-1"))
    )

    assert any(
        event["type"] == "tool-invocation"
        and event["toolName"] == "resolveRef"
        and event["state"] == "result"
        and event["result"]["value"] == {"product_ids": ["sku-1", "sku-2"]}
        and event["result"]["glimpse"] == {"productCount": 2}
        for event in resolve_events
    )


@pytest.mark.parametrize(
    "services",
    [{"tool_use_id": object()}, {"services": object()}, {"references": object()}],
)
def test_create_runtime_rejects_reserved_service_names(services):
    specialist = define_specialist(
        name="worker",
        model="claude-sonnet-4-6",
        prompt="Use tools when requested.",
    )

    with pytest.raises(ValueError, match="reserved"):
        create_runtime(_spec_with_agent(specialist), services=services)


def test_python_dynamic_approval_callback_gates_tool_before_handler_runs():
    approvals = []
    handler_calls = []

    def needs_approval(args, ctx):
        approvals.append((args, ctx["target"], ctx["kind"]))
        return True

    @define_tool("guarded_echo", {"value": str}, approval=needs_approval)
    async def guarded_echo(args, ctx):
        handler_calls.append((args, ctx["tool_use_id"]))
        return {"echo": args["value"]}

    specialist = define_specialist(
        name="worker",
        model="claude-sonnet-4-6",
        prompt="Use the requested tool.",
        tools=[guarded_echo],
    )
    runtime = create_runtime(_spec_with_agent(specialist), interpreter="scripted")

    async def run_flow():
        prompt = json.dumps({"tool": "guarded_echo", "args": {"value": "hold"}})
        stream = runtime.run_specialist("worker", prompt, thread_id="thread-1")
        events = []
        async for event in stream:
            events.append(event)
            if event["type"] == "approval-required":
                assert handler_calls == []
                await runtime.respond_to_approval(
                    event["data"]["id"],
                    "approve",
                    feedback="approved in test",
                )
        return events

    events = asyncio.run(run_flow())

    assert approvals == [({"value": "hold"}, "guarded_echo", "tool")]
    assert handler_calls == [({"value": "hold"}, "scripted-tool-1")]
    assert any(event["type"] == "approval-decision" for event in events)
    assert any(
        event["type"] == "tool-invocation"
        and event["toolName"] == "guarded_echo"
        and event["state"] == "result"
        for event in events
    )
