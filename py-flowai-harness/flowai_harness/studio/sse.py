from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any
from uuid import uuid4

from flowai_harness.studio.app import STUDIO_API_VERSION


@dataclass(frozen=True)
class ChatRequest:
    prompt: str
    thread_id: str
    run_id: str
    legacy_messages: bool = False


def normalize_chat_request(payload: dict[str, Any]) -> ChatRequest:
    prompt = payload.get("prompt")
    if prompt is None:
        prompt = payload.get("message")
    legacy_messages = False
    if prompt is None and "messages" in payload:
        prompt = _final_user_message(payload["messages"])
        legacy_messages = True
    if not isinstance(prompt, str) or prompt == "":
        raise ValueError("chat stream request requires non-empty prompt, message, or messages[]")
    thread_id = payload.get("threadId") or payload.get("thread_id") or f"thread_{uuid4().hex}"
    if not isinstance(thread_id, str) or thread_id == "":
        raise ValueError("threadId must be a non-empty string")
    run_id = payload.get("runId") or payload.get("run_id") or f"run_{uuid4().hex}"
    if not isinstance(run_id, str) or run_id == "":
        raise ValueError("runId must be a non-empty string")
    return ChatRequest(
        prompt=prompt,
        thread_id=thread_id,
        run_id=run_id,
        legacy_messages=legacy_messages,
    )


def studio_event(
    *,
    workspace_key: str,
    run_id: str,
    thread_id: str,
    agent_id: str,
    seq: int,
    kind: str,
    payload: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schemaVersion": STUDIO_API_VERSION,
        "workspaceKey": workspace_key,
        "runId": run_id,
        "threadId": thread_id,
        "agentId": agent_id,
        "seq": seq,
        "kind": kind,
        "payload": payload,
    }


def encode_sse(event: dict[str, Any]) -> str:
    return (
        f"id: {event['seq']}\n"
        f"event: {event['kind']}\n"
        f"data: {json.dumps(event, sort_keys=True)}\n\n"
    )


def project_runtime_event(raw: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    event_type = raw.get("type")
    if event_type == "text":
        return "message.delta", {"role": "assistant", "text": str(raw.get("text", ""))}
    if event_type == "tool-invocation":
        return _tool_event(raw)
    if event_type == "tool-agent":
        return _sub_agent_event(raw)
    if event_type == "approval-required":
        data = raw.get("data") if isinstance(raw.get("data"), dict) else {}
        return (
            "approval.required",
            {
                "approvalId": str(data.get("id", "")),
                "kind": data.get("kind", "custom"),
                "title": data.get("title") or data.get("target") or "Approval required",
                "raw": data,
            },
        )
    if event_type == "approval-decision":
        data = raw.get("data") if isinstance(raw.get("data"), dict) else {}
        outcome = data.get("outcome")
        if isinstance(outcome, dict):
            status = outcome.get("outcome")
        else:
            status = outcome
        return (
            "approval.decision",
            {
                "approvalId": str(data.get("id", "")),
                "status": str(status or "resolved"),
                "raw": data,
            },
        )
    if event_type == "finish":
        return "runtime.finish", {"raw": raw}
    if event_type == "error":
        message = raw.get("message")
        if isinstance(raw.get("error"), dict):
            message = raw["error"].get("message", message)
        return (
            "run.failed",
            {
                "error": {
                    "code": "runtime.error",
                    "message": str(message or "Runtime stream failed."),
                    "retryable": False,
                    "details": {},
                }
            },
        )
    return "runtime.event", {"raw": raw}


def _tool_event(raw: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    state = raw.get("state")
    tool_name = str(raw.get("toolName") or raw.get("tool_name") or "")
    tool_call_id = str(
        raw.get("toolInvocationId")
        or raw.get("tool_invocation_id")
        or raw.get("toolCallId")
        or raw.get("tool_call_id")
        or ""
    )
    if state == "call":
        return (
            "tool.call.started",
            {
                "toolCallId": tool_call_id,
                "toolName": tool_name,
                "arguments": raw.get("args") or raw.get("arguments") or {},
            },
        )
    if state == "result":
        return (
            "tool.call.completed",
            {
                "toolCallId": tool_call_id,
                "toolName": tool_name,
                "status": "completed",
                "result": raw.get("result"),
            },
        )
    return "tool.call.event", {"raw": raw}


def _sub_agent_event(raw: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    state = raw.get("state")
    agent_name = str(raw.get("agentName") or raw.get("agent_name") or "")
    invocation_id = str(
        raw.get("toolInvocationId")
        or raw.get("tool_invocation_id")
        or raw.get("toolCallId")
        or raw.get("tool_call_id")
        or raw.get("invocationId")
        or raw.get("invocation_id")
        or ""
    )
    if state == "call":
        return (
            "sub_agent.call.started",
            {
                "toolCallId": invocation_id,
                "targetAgentId": agent_name,
                "message": raw.get("prompt") or raw.get("message") or "",
            },
        )
    if state == "result":
        return (
            "sub_agent.call.completed",
            {
                "toolCallId": invocation_id,
                "targetAgentId": agent_name,
                "status": "completed",
                "result": raw.get("result"),
            },
        )
    return "sub_agent.call.event", {"raw": raw}


def _final_user_message(messages: Any) -> str | None:
    if not isinstance(messages, list):
        raise ValueError("messages must be a list")
    for message in reversed(messages):
        if not isinstance(message, dict) or message.get("role") != "user":
            continue
        content = message.get("content")
        if isinstance(content, str):
            return content
        if isinstance(content, list):
            parts = [
                part.get("text")
                for part in content
                if isinstance(part, dict) and isinstance(part.get("text"), str)
            ]
            if parts:
                return "\n".join(parts)
    raise ValueError("messages[] must contain at least one user message")
