use agent_fw_catalog::{
    CatalogScope, CatalogSearchCursor, CatalogSearchFilters, CatalogSearchRequest,
};
use sha2::{Digest, Sha256};

use crate::error::CatalogIndexError;

const PREFIX: &str = "v1:";

pub(crate) fn encode_offset(offset: usize, signature: &str) -> CatalogSearchCursor {
    CatalogSearchCursor::new(format!("{PREFIX}{signature}:{offset}"))
}

pub(crate) fn decode_offset(
    cursor: Option<&CatalogSearchCursor>,
    expected_signature: &str,
) -> Result<usize, CatalogIndexError> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let Some(raw_cursor) = cursor.as_str().strip_prefix(PREFIX) else {
        return Err(CatalogIndexError::InvalidCursor(
            cursor.as_str().to_string(),
        ));
    };
    let Some((signature, raw_offset)) = raw_cursor.rsplit_once(':') else {
        return Err(CatalogIndexError::InvalidCursor(
            cursor.as_str().to_string(),
        ));
    };
    if signature != expected_signature {
        return Err(CatalogIndexError::InvalidCursor(
            "cursor does not match the current catalog search request".to_string(),
        ));
    }
    raw_offset
        .parse::<usize>()
        .map_err(|_| CatalogIndexError::InvalidCursor(cursor.as_str().to_string()))
}

pub(crate) fn request_signature(scope: &CatalogScope, request: &CatalogSearchRequest) -> String {
    let mut hasher = Sha256::new();
    hash_part(&mut hasher, "tenant", scope.tenant_id.as_str());
    hash_part(&mut hasher, "workspace", scope.workspace_id.as_str());
    hash_part(&mut hasher, "query", request.query.trim());

    let mut kinds = request
        .kinds
        .iter()
        .map(|kind| kind.public_name())
        .collect::<Vec<_>>();
    kinds.sort_unstable();
    for kind in kinds {
        hash_part(&mut hasher, "kind", kind);
    }

    hash_filters(&mut hasher, &request.filters);
    hex::encode(&hasher.finalize()[..16])
}

fn hash_filters(hasher: &mut Sha256, filters: &CatalogSearchFilters) {
    hash_optional(hasher, "database_id", filters.database_id.as_deref());
    hash_optional(hasher, "schema", filters.schema.as_deref());
    hash_optional(hasher, "table", filters.table.as_deref());
    hash_optional(hasher, "column", filters.column.as_deref());
    hash_optional(hasher, "data_type", filters.data_type.as_deref());
    hash_optional(hasher, "semantic_type", filters.semantic_type.as_deref());
    let mut tags = filters.tags.iter().map(String::as_str).collect::<Vec<_>>();
    tags.sort_unstable();
    for tag in tags {
        hash_part(hasher, "tag", tag);
    }
    hash_optional(hasher, "knowledge_type", filters.knowledge_type.as_deref());
    hash_optional(hasher, "relation_kind", filters.relation_kind.as_deref());
    hash_optional(hasher, "source_table", filters.source_table.as_deref());
    hash_optional(hasher, "source_column", filters.source_column.as_deref());
    hash_optional(hasher, "target_table", filters.target_table.as_deref());
    hash_optional(hasher, "target_column", filters.target_column.as_deref());
    hash_optional_bool(
        hasher,
        "preferred_query_surface",
        filters.preferred_query_surface,
    );
    hash_optional_bool(hasher, "low_cardinality_enum", filters.low_cardinality_enum);
}

fn hash_optional(hasher: &mut Sha256, label: &str, value: Option<&str>) {
    if let Some(value) = value {
        hash_part(hasher, label, value.trim());
    }
}

fn hash_optional_bool(hasher: &mut Sha256, label: &str, value: Option<bool>) {
    if let Some(value) = value {
        hash_part(hasher, label, if value { "true" } else { "false" });
    }
}

fn hash_part(hasher: &mut Sha256, label: &str, value: &str) {
    hasher.update(label.as_bytes());
    hasher.update([0]);
    hasher.update(value.len().to_le_bytes());
    hasher.update(value.as_bytes());
    hasher.update([0xff]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_catalog::{CatalogSearchFilters, SemanticEntityKind};
    use agent_fw_core::{TenantId, WorkspaceId};

    #[test]
    fn cursor_roundtrips_offsets() {
        let signature = "abcdef";
        let cursor = encode_offset(25, signature);
        assert_eq!(decode_offset(Some(&cursor), signature).unwrap(), 25);
        assert_eq!(decode_offset(None, signature).unwrap(), 0);
        assert!(decode_offset(Some(&cursor), "different").is_err());
    }

    #[test]
    fn request_signature_is_stable_for_unordered_filters() {
        let scope = CatalogScope::new(
            TenantId::new_unchecked("tenant-a"),
            WorkspaceId::new_unchecked("workspace-a"),
        );
        let mut first = CatalogSearchRequest {
            query: "sales".to_string(),
            kinds: Vec::new(),
            filters: CatalogSearchFilters {
                tags: vec!["b".to_string(), "a".to_string()],
                ..CatalogSearchFilters::default()
            },
            limit: 1,
            cursor: None,
        };
        let mut second = first.clone();
        second.filters.tags.reverse();

        assert_eq!(
            request_signature(&scope, &first),
            request_signature(&scope, &second)
        );

        first.filters.table = Some("fact_sales".to_string());
        assert_ne!(
            request_signature(&scope, &first),
            request_signature(&scope, &second)
        );
    }

    #[hegel::test]
    fn cursor_roundtrip_law(tc: hegel::TestCase) {
        let offset = tc.draw(hegel::generators::integers::<usize>().max_value(10_000));
        let signature: String = tc.draw(hegel::generators::text().min_size(1).max_size(32));

        let cursor = encode_offset(offset, &signature);

        assert_eq!(decode_offset(Some(&cursor), &signature).unwrap(), offset);
        assert!(decode_offset(Some(&cursor), "different-signature").is_err());
    }

    #[hegel::test]
    fn request_signature_is_stable_for_same_logical_request_law(tc: hegel::TestCase) {
        let scope = draw_scope(&tc);
        let mut first = draw_request(&tc);
        let mut second = first.clone();
        second.limit = second.limit.saturating_add(17);
        second.cursor = Some(encode_offset(25, "prior-signature"));

        assert_eq!(
            request_signature(&scope, &first),
            request_signature(&scope, &second),
            "cursor signatures must ignore pagination state"
        );

        first.filters.tags.reverse();
        first.kinds.reverse();
        assert_eq!(
            request_signature(&scope, &first),
            request_signature(&scope, &second),
            "cursor signatures must be stable for unordered kind/tag filters"
        );
    }

    #[hegel::test]
    fn request_signature_changes_when_scope_or_filters_change_law(tc: hegel::TestCase) {
        let scope = draw_scope(&tc);
        let request = draw_request(&tc);
        let original = request_signature(&scope, &request);

        let changed_scope = CatalogScope::new(
            TenantId::new_unchecked(format!("{}-other", scope.tenant_id.as_str())),
            WorkspaceId::new_unchecked(scope.workspace_id.as_str().to_string()),
        );
        assert_ne!(original, request_signature(&changed_scope, &request));

        let mut changed_request = request.clone();
        changed_request.query.push_str(" changed");
        assert_ne!(original, request_signature(&scope, &changed_request));

        let mut changed_filter = request;
        changed_filter.filters.table = Some("changed_table".to_string());
        assert_ne!(original, request_signature(&scope, &changed_filter));
    }

    fn draw_scope(tc: &hegel::TestCase) -> CatalogScope {
        let tenant_suffix = tc.draw(hegel::generators::integers::<u16>());
        let workspace_suffix = tc.draw(hegel::generators::integers::<u16>());
        CatalogScope::new(
            TenantId::new_unchecked(format!("tenant-{tenant_suffix}")),
            WorkspaceId::new_unchecked(format!("workspace-{workspace_suffix}")),
        )
    }

    fn draw_request(tc: &hegel::TestCase) -> CatalogSearchRequest {
        let query_suffix = tc.draw(hegel::generators::integers::<u16>());
        let tag_suffix = tc.draw(hegel::generators::integers::<u16>());
        let table_suffix = tc.draw(hegel::generators::integers::<u16>());
        let limit = tc.draw(
            hegel::generators::integers::<usize>()
                .min_value(1)
                .max_value(100),
        );
        CatalogSearchRequest {
            query: format!("query {query_suffix}"),
            kinds: vec![SemanticEntityKind::Table, SemanticEntityKind::Column],
            filters: CatalogSearchFilters {
                table: Some(format!("table_{table_suffix}")),
                tags: vec![
                    format!("tag_{tag_suffix}"),
                    format!("tag_{}", tag_suffix.wrapping_add(1)),
                ],
                ..CatalogSearchFilters::default()
            },
            limit,
            cursor: None,
        }
    }
}
