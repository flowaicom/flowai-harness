from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ExpectedRelationship = tuple[str, str, str, str]


FIXTURES_ROOT = Path(__file__).parent / "fixtures"


@dataclass(frozen=True)
class TruthQuestion:
    id: str
    kind: str
    question: str
    expected_tables: list[str]
    expected_terms: list[str]
    expected_rows: dict[str, str]
    expected_total: str | None
    validation_sql: str | None


@dataclass(frozen=True)
class Scenario:
    name: str
    database_id: str
    target_schema: str
    profile_tables: list[str]
    expected_columns: dict[str, list[str]]
    expected_row_counts: dict[str, int]
    expected_relationships: list[ExpectedRelationship]
    document_paths: list[str]
    limits: dict[str, int]
    truth_questions: list[TruthQuestion]
    root: Path

    def question(self, question_id: str) -> TruthQuestion:
        for question in self.truth_questions:
            if question.id == question_id:
                return question
        raise KeyError(question_id)

    @property
    def documents_dir(self) -> Path:
        return self.root / "documents"

    @property
    def expected_column_count(self) -> int:
        return sum(len(columns) for columns in self.expected_columns.values())

    @property
    def schema_sql(self) -> Path:
        return self.root / "schema.sql"

    @property
    def seed_sql(self) -> Path:
        return self.root / "seed.sql"


def load_scenario(name: str) -> Scenario:
    root = FIXTURES_ROOT / name
    raw = json.loads((root / "scenario.json").read_text())
    questions = [
        TruthQuestion(
            id=item["id"],
            kind=item["kind"],
            question=item["question"],
            expected_tables=list(item.get("expected_tables", [])),
            expected_terms=list(item.get("expected_terms", [])),
            expected_rows=dict(item.get("expected_rows", {})),
            expected_total=item.get("expected_total"),
            validation_sql=item.get("validation_sql"),
        )
        for item in raw["truth_questions"]
    ]
    scenario = Scenario(
        name=raw["name"],
        database_id=raw["database_id"],
        target_schema=raw["target_schema"],
        profile_tables=list(raw["profile_tables"]),
        expected_columns={key: list(value) for key, value in raw["expected_columns"].items()},
        expected_row_counts={key: int(value) for key, value in raw["expected_row_counts"].items()},
        expected_relationships=[
            (
                item["from_table"],
                item["from_column"],
                item["to_table"],
                item["to_column"],
            )
            for item in raw["expected_relationships"]
        ],
        document_paths=list(raw["document_paths"]),
        limits=dict(raw["limits"]),
        truth_questions=questions,
        root=root,
    )
    _validate_scenario(scenario)
    return scenario


def _validate_scenario(scenario: Scenario) -> None:
    missing = [
        path
        for path in [scenario.schema_sql, scenario.seed_sql]
        if not path.exists()
    ]
    missing.extend(
        scenario.root / document_path
        for document_path in scenario.document_paths
        if not (scenario.root / document_path).exists()
    )
    if missing:
        rendered = ", ".join(str(path) for path in missing)
        raise FileNotFoundError(f"scenario '{scenario.name}' is missing fixtures: {rendered}")

    unknown_tables = set(scenario.expected_columns) - set(scenario.profile_tables)
    if unknown_tables:
        raise ValueError(
            f"scenario '{scenario.name}' expected_columns contains tables not in profile_tables: "
            f"{sorted(unknown_tables)}"
        )

    row_count_tables = set(scenario.expected_row_counts)
    profile_tables = set(scenario.profile_tables)
    if row_count_tables != profile_tables:
        raise ValueError(
            f"scenario '{scenario.name}' expected_row_counts must cover profile_tables exactly: "
            f"missing={sorted(profile_tables - row_count_tables)} "
            f"extra={sorted(row_count_tables - profile_tables)}"
        )

    known_columns = {
        (table, column)
        for table, columns in scenario.expected_columns.items()
        for column in columns
    }
    invalid_relationships = [
        relationship
        for relationship in scenario.expected_relationships
        if (relationship[0], relationship[1]) not in known_columns
        or (relationship[2], relationship[3]) not in known_columns
    ]
    if invalid_relationships:
        raise ValueError(
            f"scenario '{scenario.name}' expected_relationships reference unknown columns: "
            f"{invalid_relationships}"
        )
