# Glimpse

A glimpse is the small summary stored beside a reference. It lets an agent
reason about what a referenced value contains without resolving the full
payload into the prompt.

Use `glimpse(...)` inside reference callbacks or tools when you already have
domain-specific summary data and want to normalize it into JSON-compatible
values. Customer code chooses the meaningful fields, such as counts, labels,
sample ids, or aggregate metrics. For sequence inputs, the helper produces a
safe `{count, sample}` fallback.

For how glimpses work with reference handles, see
[References & glimpses](../concepts/references.md).

::: flowai_harness.glimpse.glimpse
