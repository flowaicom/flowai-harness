# Security

This document describes the security posture of the Flow AI Harness
(`flowai-harness`) and the deployment model it is designed for.

**This is the first (`v0.1.0-alpha.1`) release and is intended for use on the
operator's local machine (loopback) only.** The limitations listed below are
accepted for this release and tracked for a future release; they are safe to
operate behind within the loopback model described here, but the servers must
not be exposed to untrusted networks.

## Supported deployment model

Flow AI Harness is designed to run **on the operator's local machine** and to
bind to the **loopback interface** (`127.0.0.1` / `::1`) only.

- The **Studio** web UI (`flowai-harness dev` / `serve`) defaults to
  `127.0.0.1`.
- The **MCP Streamable-HTTP** server (`flowai-mcp ... --transport
  streamable-http`) defaults to `127.0.0.1:8765`.

Loopback is a convenience, **not a security boundary**. On shared hosts, CI
runners, containers, and port-forwarded environments (e.g. WSL), any local
process or user that can reach the bound port can reach the server.

## Mitigations present in this release

The following controls are in place:

- **Read-only query validation** rejects DDL/DML, multi-statement, locking, and
  `SELECT INTO` at the SQL AST level (`agent-fw-algebra`), with bind parameters
  on the write path (no string-interpolated values) and a dangerous-function
  denylist (`pg_read_file`, `dblink`, `lo_import`, `pg_execute_server_program`,
  …) as defense-in-depth.
- **Credential redaction** strips userinfo and secret query parameters from
  connection strings before logging or display, and redacts URIs in Neon
  provisioner error messages.
- **Content-Security-Policy** is shipped in `Report-Only` mode on the Studio
  API to collect the violation set needed to enforce a strict policy later,
  without risk to this release.
- **No telemetry, analytics, or phone-home** of any kind; TLS verification is
  never disabled; provider keys are sent to the configured provider via headers
  and are not logged.
- **Supply-chain pinning**: dependencies are pinned; the `hegeltest` dependency
  is pinned to a fixed revision; the vendored `rig` patch is audited and
  documented (`third_party/rig/UPSTREAM.md`).

## Known limitations (this release)

The following are **known limitations, accepted for this first release and
tracked for a future release** — not features, and not safe to rely on beyond
the loopback model above:

1. **No authentication on the Studio HTTP API or the MCP HTTP transport.**
   Any client that can reach the port can invoke tools, run agents and evals,
   respond to pending approvals, and read traces. Do **not** expose either
   server to an untrusted network (e.g. do **not** pass `--host 0.0.0.0` on a
   cloud VM or shared network) without an authenticating reverse proxy in
   front.
2. **Studio renders model output, tool results, and trace/eval content as
   Markdown via `marked`, which passes raw HTML through.** That output is
   currently **not sanitized** (see `studio/app/components/shared/markdown.tsx`).
   Treat data sources and model output as untrusted. A strict Content-Security-
   Policy is shipped in `Report-Only` mode to collect the violation set needed
   to enforce one safely.
3. **Provider API keys entered in Studio settings are persisted in the
   browser's `localStorage` in plaintext.** Prefer providing keys via
   environment variables to the backend where possible.
4. **Read-only enforcement of target databases is AST-level.** A read-only
   database role is the intended primary control; until then, a
   dangerous-function denylist (`pg_read_file`, `dblink`, `lo_import`, …)
   provides defense-in-depth, but only point data agents at data you are
   entitled to and willing to expose to the agent's model.
5. **Tool approval defaults to `Never`.** Side-effecting tools registered
   without an explicit approval policy execute without a human gate. Set an
   explicit approval policy for any tool that mutates state.
6. **MCP tool arguments are validated by type deserialization only.** The
   declared JSON Schema (enums, `max_length`, patterns) is advertised to the
   model but not re-enforced at the MCP dispatch boundary, and there is no
   payload-size cap. Only connect MCP clients you trust.

## Prompt injection

This is an agent framework: tool results, retrieved rows, and documents are
placed in the model's context. Treat all such content as untrusted input. Do
not point agents at untrusted data sources in combination with side-effecting
tools unless those tools carry an approval gate.

## Credential handling

- Provider keys are sent to the configured provider endpoint via headers, not
  query parameters, and are not logged.
- Connection-string credentials are redacted (`redact_url`) before logging or
  display in the runtime; the Neon provisioner redacts URIs in its error
  messages.
- Persisted data-source credentials are stored **plaintext** by default in the
  preview (`NoOpEncryptionService`). Do not persist production credentials
  through this path yet.

## Reporting a vulnerability

Please report suspected vulnerabilities **privately** before any public
disclosure:

> Report to: **hello (at) flow-ai.com**

We aim to acknowledge reports within 2 business days.
