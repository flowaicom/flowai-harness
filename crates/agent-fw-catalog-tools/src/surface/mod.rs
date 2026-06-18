//! Shared services for the replacement catalog tool surface.
//!
//! This module owns the Task 3 service layer for the seven-tool contract. It
//! intentionally does not register handlers or runtime toolkits; that wiring is
//! handled by the follow-up runtime task.

pub mod entity_output;
pub mod filters;
pub mod graph;
pub mod handlers;
pub mod output_policy;
pub mod ref_resolution;
pub mod schema;
pub mod types;

pub use entity_output::CatalogEntityAssembler;
pub use filters::CatalogFilterResolver;
pub use graph::CatalogGraphService;
pub use handlers::{
    execute_query as execute_query_surface, get_catalog_entities, get_catalog_relations,
    get_relation_paths_between, list_schema_fields, sample_table_data as sample_table_data_surface,
    search_catalog, surface_handlers,
};
pub use output_policy::{
    DetailsMode, DiagnosticsMode, OutputPolicy, OutputPolicyRegistry, RelationMode, TruncationMode,
};
pub use ref_resolution::CatalogRefResolver;
pub use types::{
    AmbiguousFilterValue, CanonicalizedFilterValue, CatalogEntity, CatalogEntityAssembly,
    CatalogEntityKind, CatalogFilterRef, CatalogFilters, CatalogGraphEdge, CatalogRef,
    CatalogRefResolution, CatalogRelationPath, CatalogRelationPathStep, CatalogRelationResult,
    CatalogRelationsForRef, ExecuteQueryInput, ExecuteQueryOutput, FacetValue, FilterResolution,
    GetCatalogEntitiesInput, GetCatalogEntitiesOutput, GetCatalogRelationsInput,
    GetCatalogRelationsOutput, GetRelationPathsBetweenInput, GetRelationPathsBetweenOutput,
    ListSchemaFieldsInput, ListSchemaFieldsOutput, MatchDiagnostics, Pagination, PathType,
    RelationDirection, ResolvedCatalogFilters, ResolvedCatalogRef, SampleTableDataInput,
    SampleTableDataOutput, SchemaFieldsForTable, SearchCatalogDiagnostics, SearchCatalogFacets,
    SearchCatalogInput, SearchCatalogOutput, UnresolvedFilterValue,
};
