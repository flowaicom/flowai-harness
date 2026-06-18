from itertools import permutations

import pytest
from pydantic import BaseModel

from flowai_harness import define_tenant, define_tool, layered_prompt
from flowai_harness.prompts import augment_prompt_tools


class DomainKnowledge(BaseModel):
    segments: list[str]
    channels: list[str]


def test_layered_prompt_is_deterministic_and_sorts_tools_by_name():
    domain_knowledge = DomainKnowledge(
        segments=["retail"],
        channels=["online"],
    )
    zeta = define_tool("zeta_tool", {"query": str}, description="Last tool")
    alpha = define_tool("alpha_tool", {"query": str}, description="First tool")

    first = layered_prompt(
        identity="You coordinate analytical work.",
        communication="Use concise status updates.",
        operational_rules=["Plan before execution.", "Parallelize independent discovery."],
        tools=[zeta, alpha],
        domain_knowledge=domain_knowledge,
        safety=["Never execute side effects without approval."],
        output_format={"kind": "typed_events"},
        examples=[{"user": "Build a Q3 pricing scenario."}],
    )
    second = layered_prompt(
        identity="You coordinate analytical work.",
        communication="Use concise status updates.",
        operational_rules=["Plan before execution.", "Parallelize independent discovery."],
        tools=[zeta, alpha],
        domain_knowledge=domain_knowledge,
        safety=["Never execute side effects without approval."],
        output_format={"kind": "typed_events"},
        examples=[{"user": "Build a Q3 pricing scenario."}],
    )

    assert first == second
    assert first.text == second.text
    assert first.cache_key == second.cache_key
    assert str(first) == first.text
    assert first.text.index("alpha_tool") < first.text.index("zeta_tool")
    assert "# Operational Rules\n- Plan before execution." in first.text
    assert "# Domain Knowledge\n```json" in first.text


def test_layered_prompt_cache_key_changes_when_non_domain_sections_change():
    domain_knowledge = {"segments": ["retail"]}

    first = layered_prompt(identity="You plan.", domain_knowledge=domain_knowledge)
    second = layered_prompt(identity="You execute.", domain_knowledge=domain_knowledge)

    assert first.text != second.text
    assert first.cache_key != second.cache_key


def test_layered_prompt_cache_key_is_stable_for_equivalent_domain_ordering():
    left = {"domain": {"b": 2, "a": 1}}
    right = {"domain": {"a": 1, "b": 2}}

    assert (
        layered_prompt(identity="You plan.", domain_knowledge=left).cache_key
        == layered_prompt(identity="You plan.", domain_knowledge=right).cache_key
    )


def test_layered_prompt_cache_key_is_stable_across_domain_key_permutations():
    items = [
        ("entities", [{"id": "product"}]),
        ("rules", ["plan first"]),
        ("dataModel", {"b": 2, "a": 1}),
    ]

    cache_keys = {
        layered_prompt(
            identity="You plan.",
            domain_knowledge=dict(ordering),
        ).cache_key
        for ordering in permutations(items)
    }

    assert len(cache_keys) == 1


def test_layered_prompt_cache_key_ignores_resource_identity_unless_rendered():
    acme = define_tenant("acme", "v1")
    beta = define_tenant("beta", "v1")
    domain_knowledge = {"segments": ["retail"]}

    first = layered_prompt(identity="You plan.", domain_knowledge=domain_knowledge)
    second = layered_prompt(identity="You plan.", domain_knowledge=domain_knowledge)
    with_tenant = layered_prompt(
        identity="You plan.",
        domain_knowledge={"tenant": acme, "segments": ["retail"]},
    )
    with_other_tenant = layered_prompt(
        identity="You plan.",
        domain_knowledge={"tenant": beta, "segments": ["retail"]},
    )

    assert first.cache_key == second.cache_key
    assert first.cache_key != with_tenant.cache_key
    assert with_tenant.cache_key != with_other_tenant.cache_key


def test_layered_prompt_rejects_structured_values_in_text_sections():
    domain_knowledge = {"segments": ["retail"]}

    with pytest.raises(TypeError, match="operational_rules"):
        layered_prompt(
            identity="You plan.",
            operational_rules=domain_knowledge,
            domain_knowledge=domain_knowledge,
        )


def test_layered_prompt_rejects_invalid_tool_approval_shape():
    with pytest.raises(TypeError, match="approval"):
        layered_prompt(
            identity="You plan.",
            tools=[{"name": "bad_tool", "approval": 123}],
        )


def test_layered_prompt_rejects_unknown_structured_types_at_boundary():
    with pytest.raises(TypeError, match="domain_knowledge"):
        layered_prompt(identity="You plan.", domain_knowledge={1, 2, 3})


def test_augment_prompt_tools_merges_existing_tools_and_preserves_explicit_rows():
    prompt = layered_prompt(
        identity="You read data.",
        tools=[
            {
                "name": "execute_query",
                "description": "Customer-specific query guidance.",
                "approval": "always",
            }
        ],
        safety=["Read-only operations only."],
    )

    augmented = augment_prompt_tools(
        prompt.text,
        [
            {
                "name": "execute_query",
                "description": "Runtime toolkit description.",
                "approval": "runtime",
            },
            {
                "name": "list_tables",
                "description": "List tables.",
                "approval": "runtime",
            },
        ],
    )

    assert augmented.text.count("execute_query") == 1
    assert "Customer-specific query guidance." in augmented.text
    assert "Runtime toolkit description." not in augmented.text
    assert augmented.text.index("# Tools") < augmented.text.index("# Safety")
    assert augmented.cache_key != prompt.cache_key


def test_augment_prompt_tools_appends_tools_section_to_raw_prompt():
    augmented = augment_prompt_tools(
        "You are a specialist.",
        [{"name": "search_catalog", "description": "Search catalog entities."}],
    )

    assert augmented.text == (
        "You are a specialist.\n\n"
        "# Tools\n"
        "| Tool | Description | Approval |\n"
        "| --- | --- | --- |\n"
        "| search_catalog | Search catalog entities. |  |"
    )
