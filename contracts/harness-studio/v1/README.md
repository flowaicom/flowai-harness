# Harness Studio v1 Contract

This directory contains the normative M0 contract for `harness-studio/v1`.
It is intentionally outside `docs/` because this is an implementation contract
owned by code and tests, not only a documentation reference.

## Files

- `openapi.yaml`: REST and SSE schema fixture.
- `fixtures/*.json`: REST response examples.
- `fixtures/sse/*.json`: SSE event examples.
- `conformance.md`: shared Python, TypeScript, Enterprise, and compatibility-server test strategy.
- `deprecations.md`: route alias and breaking-change policy.
- `validate_contract.py`: lightweight local fixture drift check.

## Validate

```bash
python3 contracts/harness-studio/v1/validate_contract.py
```

This check intentionally uses only the Python standard library. It is not a full OpenAPI validator; it catches obvious contract drift before server implementations are added.

## Version

The version string is:

```text
harness-studio/v1
```

Servers must report it from `GET /api/status` before the shared UI treats the server as a Harness Studio implementation.
