from pydantic import BaseModel

from flowai_harness import define_reference, glimpse


class SelectionPayload(BaseModel):
    ids: list[str]
    label: str


def test_glimpse_returns_customer_mapping_summaries_unchanged():
    result = glimpse({"selectionCount": 2, "preview": ["north", "south"]})

    assert result == {"selectionCount": 2, "preview": ["north", "south"]}


def test_glimpse_summarizes_sequences_generically():
    result = glimpse(
        [
            {"id": "north", "label": "North"},
            {"id": "south", "label": "South"},
            {"id": "west", "label": "West"},
        ],
        max_items=2,
    )

    assert result == {
        "count": 3,
        "sample": [
            {"id": "north", "label": "North"},
            {"id": "south", "label": "South"},
        ],
    }


def test_reference_glimpse_is_customer_code():
    reference = define_reference(
        name="Selection",
        schema=SelectionPayload,
        glimpse=lambda value: {
            "selectionCount": len(value.ids),
            "label": value.label,
            "preview": value.ids[:2],
        },
    )

    assert reference.glimpse is not None
    assert reference.glimpse(SelectionPayload(ids=["a", "b", "c"], label="Control")) == {
        "selectionCount": 3,
        "label": "Control",
        "preview": ["a", "b"],
    }


def test_glimpse_accepts_pydantic_values():
    result = glimpse(SelectionPayload(ids=["a", "b"], label="Control"))

    assert result == {"ids": ["a", "b"], "label": "Control"}
