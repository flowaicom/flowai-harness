use agent_fw_tool::ToolSchema;
use serde_json::{json, Value};

use super::types::{
    CatalogFilterRef, CatalogFilters, CatalogRef, ExecuteQueryInput, GetCatalogEntitiesInput,
    GetCatalogRelationsInput, GetRelationPathsBetweenInput, ListSchemaFieldsInput, PathType,
    RelationDirection, SampleTableDataInput, SearchCatalogInput,
};

impl ToolSchema for CatalogRef {
    fn json_schema() -> Value {
        catalog_ref_schema()
    }
}

impl ToolSchema for CatalogFilterRef {
    fn json_schema() -> Value {
        catalog_filter_ref_schema()
    }
}

impl ToolSchema for CatalogFilters {
    fn json_schema() -> Value {
        catalog_filters_schema()
    }
}

impl ToolSchema for SearchCatalogInput {
    fn json_schema() -> Value {
        with_definitions(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language search phrase or identifier to find in the catalog."
                },
                "kinds": {
                    "type": "array",
                    "items": { "$ref": "#/definitions/CatalogEntityKind" },
                    "description": "Optional entity kinds to search."
                },
                "filters": {
                    "$ref": "#/definitions/CatalogFilters",
                    "description": "Optional structured filters applied before or during search."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Requested maximum number of results."
                },
                "cursor": {
                    "type": "string",
                    "description": "Opaque cursor returned by a previous search_catalog call."
                }
            }
        }))
    }
}

impl ToolSchema for GetCatalogEntitiesInput {
    fn json_schema() -> Value {
        with_definitions(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["refs"],
            "properties": {
                "refs": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 50,
                    "items": { "$ref": "#/definitions/CatalogRef" },
                    "description": "Catalog entity references to hydrate."
                }
            }
        }))
    }
}

impl ToolSchema for ListSchemaFieldsInput {
    fn json_schema() -> Value {
        with_definitions(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["tables"],
            "properties": {
                "tables": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 10,
                    "items": { "$ref": "#/definitions/CatalogRef" },
                    "description": "Table or query-surface references whose fields should be listed."
                },
                "filters": {
                    "$ref": "#/definitions/CatalogFilters",
                    "description": "Optional field filters."
                },
                "limit_per_table": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "description": "Requested maximum fields per table."
                }
            }
        }))
    }
}

impl ToolSchema for GetCatalogRelationsInput {
    fn json_schema() -> Value {
        with_definitions(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["refs"],
            "properties": {
                "refs": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 20,
                    "items": { "$ref": "#/definitions/CatalogRef" },
                    "description": "Entity references whose adjacent relations should be returned."
                },
                "direction": {
                    "type": "string",
                    "enum": relation_direction_values(),
                    "description": "Relation direction relative to each input entity."
                },
                "relation_kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional relation kinds to include."
                },
                "target_kinds": {
                    "type": "array",
                    "items": { "$ref": "#/definitions/CatalogEntityKind" },
                    "description": "Optional target entity kinds to return."
                },
                "limit_per_ref": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Requested maximum related entities per input reference."
                }
            }
        }))
    }
}

impl ToolSchema for GetRelationPathsBetweenInput {
    fn json_schema() -> Value {
        with_definitions(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["from", "to"],
            "properties": {
                "from": {
                    "$ref": "#/definitions/CatalogRef",
                    "description": "Starting entity reference."
                },
                "to": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 20,
                    "items": { "$ref": "#/definitions/CatalogRef" },
                    "description": "One or more target entity references."
                },
                "path_type": {
                    "type": "string",
                    "enum": path_type_values(),
                    "description": "Path family."
                },
                "max_depth": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 6,
                    "description": "Maximum graph depth to search."
                }
            }
        }))
    }
}

impl ToolSchema for SampleTableDataInput {
    fn json_schema() -> Value {
        with_definitions(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["table"],
            "properties": {
                "table": {
                    "$ref": "#/definitions/CatalogRef",
                    "description": "Table or query-surface reference to sample."
                },
                "columns": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional column names to include."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 20,
                    "description": "Requested maximum sample rows."
                }
            }
        }))
    }
}

impl ToolSchema for ExecuteQueryInput {
    fn json_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["sql"],
            "properties": {
                "sql": {
                    "type": "string",
                    "description": "Read-only SQL query. Must be a single SELECT or WITH query."
                },
                "params": {
                    "type": "array",
                    "items": {},
                    "description": "Optional positional bind parameters."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 1000,
                    "description": "Requested maximum output rows."
                },
                "purpose": {
                    "type": "string",
                    "description": "Short explanation of why this query is being run."
                }
            }
        })
    }
}

pub fn catalog_ref_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": {
                "type": "string",
                "description": "Stable catalog entity id returned by a prior catalog tool."
            },
            "qualified_name": {
                "type": "string",
                "description": "Fully qualified reference such as public.fact_scenario."
            },
            "name": {
                "type": "string",
                "description": "Human-readable or unqualified name."
            },
            "kind": {
                "$ref": "#/definitions/CatalogEntityKind",
                "description": "Entity kind used to disambiguate name-based references."
            }
        },
        "oneOf": [
            {
                "required": ["id"],
                "not": { "anyOf": [{ "required": ["qualified_name"] }, { "required": ["name"] }] }
            },
            {
                "required": ["qualified_name"],
                "not": { "anyOf": [{ "required": ["id"] }, { "required": ["name"] }] }
            },
            {
                "required": ["name"],
                "not": { "anyOf": [{ "required": ["id"] }, { "required": ["qualified_name"] }] }
            }
        ],
        "definitions": {
            "CatalogEntityKind": catalog_entity_kind_schema()
        }
    })
}

pub fn catalog_filter_ref_schema() -> Value {
    json!({
        "oneOf": [
            { "$ref": "#/definitions/CatalogRef" },
            { "type": "string" }
        ],
        "definitions": {
            "CatalogEntityKind": catalog_entity_kind_schema(),
            "CatalogRef": bare_catalog_ref_schema()
        }
    })
}

pub fn catalog_filters_schema() -> Value {
    let mut schema = bare_catalog_filters_schema();
    schema["definitions"] = common_definitions();
    schema
}

fn bare_catalog_filters_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "database_id": { "type": "string" },
            "schema": { "type": "string" },
            "table": { "$ref": "#/definitions/CatalogFilterRef" },
            "column": { "$ref": "#/definitions/CatalogFilterRef" },
            "data_type": { "type": "string" },
            "semantic_type": { "type": "string" },
            "tags": { "type": "array", "items": { "type": "string" } },
            "knowledge_type": { "type": "string" },
            "relation_kind": { "type": "string" },
            "source_table": { "$ref": "#/definitions/CatalogFilterRef" },
            "source_column": { "$ref": "#/definitions/CatalogFilterRef" },
            "target_table": { "$ref": "#/definitions/CatalogFilterRef" },
            "target_column": { "$ref": "#/definitions/CatalogFilterRef" },
            "preferred_query_surface": { "type": "boolean" },
            "low_cardinality_enum": { "type": "boolean" }
        }
    })
}

fn with_definitions(mut schema: Value) -> Value {
    schema["definitions"] = common_definitions();
    schema
}

fn common_definitions() -> Value {
    json!({
        "CatalogEntityKind": catalog_entity_kind_schema(),
        "CatalogRef": bare_catalog_ref_schema(),
        "CatalogFilterRef": {
            "oneOf": [
                { "$ref": "#/definitions/CatalogRef" },
                { "type": "string" }
            ]
        },
        "CatalogFilters": bare_catalog_filters_schema()
    })
}

fn bare_catalog_ref_schema() -> Value {
    let mut schema = catalog_ref_schema();
    if let Some(object) = schema.as_object_mut() {
        object.remove("definitions");
    }
    schema
}

fn catalog_entity_kind_schema() -> Value {
    json!({
        "type": "string",
        "enum": super::types::CatalogEntityKind::ALL_NAMES
    })
}

fn relation_direction_values() -> [&'static str; 3] {
    match RelationDirection::Both {
        RelationDirection::Both | RelationDirection::Incoming | RelationDirection::Outgoing => {
            ["outgoing", "incoming", "both"]
        }
    }
}

fn path_type_values() -> [&'static str; 3] {
    match PathType::Any {
        PathType::Any | PathType::Join | PathType::Semantic => ["join", "semantic", "any"],
    }
}
