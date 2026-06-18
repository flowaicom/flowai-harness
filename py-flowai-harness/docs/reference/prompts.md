# Prompts

Prompts are the system instructions attached to agents. They define the
agent's role, communication style, operating rules, tool-use guidance, domain
knowledge, safety constraints, output format, and examples.

Use `layered_prompt(...)` to build those instructions from named sections
instead of one large free-form string. The helper renders deterministic prompt
text, omits empty sections, and returns a [`LayeredPrompt`](#flowai_harness.prompts.LayeredPrompt)
with both the text and a cache key.

The cache key is a SHA-256 hash of the rendered prompt text. The harness uses
it as a stable fingerprint for change detection and traceability when a
`LayeredPrompt` is attached to an agent or when runtime assembly augments the
prompt with tool descriptions. It is not a secret, and it is not a provider
credential or provider-side cache directive.

::: flowai_harness.prompts.layered_prompt

::: flowai_harness.prompts.LayeredPrompt
