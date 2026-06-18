use agent_fw_catalog::{relation_kind, CatalogSearchFilters};

use crate::CatalogToolError;

use super::ref_resolution::CatalogRefResolver;
use super::types::{
    AmbiguousFilterValue, CanonicalizedFilterValue, CatalogEntityKind, CatalogFilterRef,
    CatalogFilters, FilterResolution, ResolvedCatalogFilters, UnresolvedFilterValue,
};

pub struct CatalogFilterResolver<'a> {
    ref_resolver: CatalogRefResolver<'a>,
}

impl<'a> CatalogFilterResolver<'a> {
    pub fn new(ref_resolver: CatalogRefResolver<'a>) -> Self {
        Self { ref_resolver }
    }

    pub async fn resolve(
        &self,
        filters: &CatalogFilters,
    ) -> Result<ResolvedCatalogFilters, CatalogToolError> {
        let mut backend_filters = CatalogSearchFilters::default();
        let mut resolution = FilterResolution {
            applied: CatalogFilters::default(),
            canonicalized: Vec::new(),
            ambiguous: Vec::new(),
            unresolved: Vec::new(),
        };

        copy_string_filter(
            "database_id",
            &filters.database_id,
            &mut resolution.applied.database_id,
            &mut backend_filters.database_id,
        );
        copy_string_filter(
            "schema",
            &filters.schema,
            &mut resolution.applied.schema,
            &mut backend_filters.schema,
        );
        copy_string_filter(
            "data_type",
            &filters.data_type,
            &mut resolution.applied.data_type,
            &mut backend_filters.data_type,
        );
        copy_string_filter(
            "semantic_type",
            &filters.semantic_type,
            &mut resolution.applied.semantic_type,
            &mut backend_filters.semantic_type,
        );
        copy_string_filter(
            "knowledge_type",
            &filters.knowledge_type,
            &mut resolution.applied.knowledge_type,
            &mut backend_filters.knowledge_type,
        );

        if !filters.tags.is_empty() {
            resolution.applied.tags = filters.tags.clone();
            backend_filters.tags = filters.tags.clone();
        }

        if let Some(relation_kind) = filters.relation_kind.as_deref() {
            if is_allowed_relation_kind(relation_kind) {
                resolution.applied.relation_kind = Some(relation_kind.to_string());
                backend_filters.relation_kind = Some(relation_kind.to_string());
            } else {
                resolution.unresolved.push(UnresolvedFilterValue {
                    field: "relation_kind".to_string(),
                    input: serde_json::Value::String(relation_kind.to_string()),
                    reason: "unknown_relation_kind".to_string(),
                });
            }
        }

        resolution.applied.preferred_query_surface = filters.preferred_query_surface;
        backend_filters.preferred_query_surface = filters.preferred_query_surface;
        resolution.applied.low_cardinality_enum = filters.low_cardinality_enum;
        backend_filters.low_cardinality_enum = filters.low_cardinality_enum;

        let resolved_table = self
            .resolve_ref_filter("table", filters.table.as_ref(), CatalogEntityKind::Table)
            .await?;
        if let Some(value) = resolved_table.backend_value.clone() {
            backend_filters.table = Some(value);
        }
        if let Some(value) = resolved_table.applied_value.clone() {
            resolution.applied.table = Some(value);
        }
        resolved_table.apply_to_resolution(&mut resolution);

        let resolved_column = self
            .resolve_ref_filter("column", filters.column.as_ref(), CatalogEntityKind::Column)
            .await?;
        if let Some(value) = resolved_column.backend_value.clone() {
            backend_filters.column = Some(value);
        }
        if let Some(value) = resolved_column.applied_value.clone() {
            resolution.applied.column = Some(value);
        }
        resolved_column.apply_to_resolution(&mut resolution);

        let resolved_source_table = self
            .resolve_ref_filter(
                "source_table",
                filters.source_table.as_ref(),
                CatalogEntityKind::Table,
            )
            .await?;
        if let Some(value) = resolved_source_table.backend_value.clone() {
            backend_filters.source_table = Some(value);
        }
        if let Some(value) = resolved_source_table.applied_value.clone() {
            resolution.applied.source_table = Some(value);
        }
        resolved_source_table.apply_to_resolution(&mut resolution);

        let resolved_source_column = self
            .resolve_ref_filter(
                "source_column",
                filters.source_column.as_ref(),
                CatalogEntityKind::Column,
            )
            .await?;
        if let Some(value) = resolved_source_column.backend_value.clone() {
            backend_filters.source_column = Some(value);
        }
        if let Some(value) = resolved_source_column.applied_value.clone() {
            resolution.applied.source_column = Some(value);
        }
        resolved_source_column.apply_to_resolution(&mut resolution);

        let resolved_target_table = self
            .resolve_ref_filter(
                "target_table",
                filters.target_table.as_ref(),
                CatalogEntityKind::Table,
            )
            .await?;
        if let Some(value) = resolved_target_table.backend_value.clone() {
            backend_filters.target_table = Some(value);
        }
        if let Some(value) = resolved_target_table.applied_value.clone() {
            resolution.applied.target_table = Some(value);
        }
        resolved_target_table.apply_to_resolution(&mut resolution);

        let resolved_target_column = self
            .resolve_ref_filter(
                "target_column",
                filters.target_column.as_ref(),
                CatalogEntityKind::Column,
            )
            .await?;
        if let Some(value) = resolved_target_column.backend_value.clone() {
            backend_filters.target_column = Some(value);
        }
        if let Some(value) = resolved_target_column.applied_value.clone() {
            resolution.applied.target_column = Some(value);
        }
        resolved_target_column.apply_to_resolution(&mut resolution);

        Ok(ResolvedCatalogFilters {
            backend_filters,
            resolution,
        })
    }

    async fn resolve_ref_filter(
        &self,
        field: &str,
        input: Option<&CatalogFilterRef>,
        default_kind: CatalogEntityKind,
    ) -> Result<ResolvedRefFilter, CatalogToolError> {
        let Some(input) = input else {
            return Ok(ResolvedRefFilter::default());
        };

        let reference = input.to_catalog_ref(default_kind);
        let ref_resolution = self.ref_resolver.resolve_exact(&reference).await?;

        if let Some(resolved) = ref_resolution.resolved {
            let canonical = resolved.catalog_ref();
            return Ok(ResolvedRefFilter {
                backend_value: Some(resolved.canonical_filter_value()),
                applied_value: Some(CatalogFilterRef::Ref(canonical.clone())),
                canonicalized: Some(CanonicalizedFilterValue {
                    field: field.to_string(),
                    input: input.display_input(),
                    canonical: serde_json::to_value(canonical).unwrap_or_default(),
                }),
                ambiguous: None,
                unresolved: None,
            });
        } else if !ref_resolution.ambiguous.is_empty() {
            return Ok(ResolvedRefFilter {
                ambiguous: Some(AmbiguousFilterValue {
                    field: field.to_string(),
                    input: input.display_input(),
                    candidates: ref_resolution
                        .ambiguous
                        .into_iter()
                        .map(|candidate| candidate.catalog_ref())
                        .collect(),
                }),
                ..ResolvedRefFilter::default()
            });
        }

        Ok(ResolvedRefFilter {
            unresolved: Some(UnresolvedFilterValue {
                field: field.to_string(),
                input: input.display_input(),
                reason: ref_resolution
                    .unresolved
                    .unwrap_or_else(|| "unresolved_reference".to_string()),
            }),
            ..ResolvedRefFilter::default()
        })
    }
}

#[derive(Default)]
struct ResolvedRefFilter {
    backend_value: Option<String>,
    applied_value: Option<CatalogFilterRef>,
    canonicalized: Option<CanonicalizedFilterValue>,
    ambiguous: Option<AmbiguousFilterValue>,
    unresolved: Option<UnresolvedFilterValue>,
}

impl ResolvedRefFilter {
    fn apply_to_resolution(self, resolution: &mut FilterResolution) {
        if let Some(value) = self.canonicalized {
            resolution.canonicalized.push(value);
        }
        if let Some(value) = self.ambiguous {
            resolution.ambiguous.push(value);
        }
        if let Some(value) = self.unresolved {
            resolution.unresolved.push(value);
        }
    }
}

fn copy_string_filter(
    _field: &str,
    input: &Option<String>,
    applied: &mut Option<String>,
    backend: &mut Option<String>,
) {
    if let Some(value) = input.as_ref().filter(|value| !value.trim().is_empty()) {
        *applied = Some(value.clone());
        *backend = Some(value.clone());
    }
}

pub(crate) fn is_allowed_relation_kind(kind: &str) -> bool {
    matches!(
        kind,
        relation_kind::HAS_COLUMN
            | relation_kind::BELONGS_TO
            | relation_kind::REFERENCES
            | relation_kind::REFERENCED_BY
            | relation_kind::REFERENCES_TABLE
            | relation_kind::REFERENCED_BY_TABLE
            | relation_kind::RELATIONSHIP_SOURCE_TABLE
            | relation_kind::RELATIONSHIP_TARGET_TABLE
            | relation_kind::ENUM_VALUE_OF
            | relation_kind::HAS_ENUM_VALUE
            | relation_kind::METRIC_USES
            | relation_kind::KNOWLEDGE_APPLIES_TO
            | relation_kind::DATA_QUALITY_FINDING_APPLIES_TO
            | relation_kind::EXTRACTED_FROM
            | relation_kind::SYNONYM_OF
            | relation_kind::EQUIVALENT_TO
            | relation_kind::SUB_CLASS_OF
            | relation_kind::APPLIES_TO
    )
}
