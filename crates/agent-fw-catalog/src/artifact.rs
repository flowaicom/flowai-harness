//! Deterministic ordering for catalog artifact exports.
//!
//! Both the harness `data catalog export` command and the standalone
//! `catalog-export` utility serialize `CatalogEntry` values into a portable
//! `catalog.entries.json` artifact. That artifact is committed into reference
//! verticals and snapshot-tested, so its ordering must be byte-stable and
//! independent of how the entries were stored or read back. This module is the
//! single source of truth for that ordering.

use crate::entry::{CatalogEntry, CatalogKind};

/// Stable sort rank for catalog kinds in exported artifacts.
///
/// The numeric ordering is part of the artifact contract; do not reorder
/// existing kinds without intentionally re-baselining committed snapshots.
fn kind_rank(kind: CatalogKind) -> u8 {
    match kind {
        CatalogKind::Table => 0,
        CatalogKind::Column => 1,
        CatalogKind::Relationship => 2,
        CatalogKind::Enum => 3,
        CatalogKind::Metric => 4,
        CatalogKind::Knowledge => 5,
        CatalogKind::Document => 6,
        CatalogKind::DataQualityFinding => 7,
        CatalogKind::Special => 8,
    }
}

/// Filter and deterministically order catalog entries for artifact export.
///
/// `Enum` entries are dropped: their values are carried inline on the owning
/// column's metadata, so re-emitting them as top-level entries would duplicate
/// data and is not part of the artifact shape. The remaining entries are sorted
/// by `(kind, qualified_name, name, id)` so repeated exports of the same catalog
/// produce byte-identical output.
pub fn order_entries_for_artifact(entries: Vec<CatalogEntry>) -> Vec<CatalogEntry> {
    let mut entries: Vec<_> = entries
        .into_iter()
        .filter(|entry| entry.kind != CatalogKind::Enum)
        .collect();
    entries.sort_by(|left, right| {
        (
            kind_rank(left.kind),
            left.qualified_name.as_deref().unwrap_or(""),
            left.name.as_str(),
            left.id.as_str(),
        )
            .cmp(&(
                kind_rank(right.kind),
                right.qualified_name.as_deref().unwrap_or(""),
                right.name.as_str(),
                right.id.as_str(),
            ))
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::CatalogEntry;

    fn entry(id: &str, kind: CatalogKind, name: &str, qualified: Option<&str>) -> CatalogEntry {
        CatalogEntry {
            id: id.into(),
            kind,
            name: name.into(),
            qualified_name: qualified.map(str::to_string),
            content: String::new(),
            tags: Vec::new(),
            links: Vec::new(),
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn drops_enum_entries() {
        let ordered = order_entries_for_artifact(vec![
            entry("e1", CatalogKind::Enum, "status_enum", None),
            entry("t1", CatalogKind::Table, "orders", Some("public.orders")),
        ]);
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered[0].id, "t1");
    }

    #[test]
    fn orders_by_kind_then_qualified_name_then_name_then_id() {
        let ordered = order_entries_for_artifact(vec![
            entry(
                "c2",
                CatalogKind::Column,
                "amount",
                Some("public.orders.amount"),
            ),
            entry(
                "t2",
                CatalogKind::Table,
                "customers",
                Some("public.customers"),
            ),
            entry("t1", CatalogKind::Table, "orders", Some("public.orders")),
            entry("c1", CatalogKind::Column, "id", Some("public.orders.id")),
        ]);
        let ids: Vec<&str> = ordered.iter().map(|e| e.id.as_str()).collect();
        // Tables (rank 0) before columns (rank 1); within a kind, by qualified_name.
        assert_eq!(ids, vec!["t2", "t1", "c2", "c1"]);
    }

    #[test]
    fn is_deterministic_under_input_permutation() {
        let a = order_entries_for_artifact(vec![
            entry("t1", CatalogKind::Table, "orders", Some("public.orders")),
            entry(
                "t2",
                CatalogKind::Table,
                "customers",
                Some("public.customers"),
            ),
        ]);
        let b = order_entries_for_artifact(vec![
            entry(
                "t2",
                CatalogKind::Table,
                "customers",
                Some("public.customers"),
            ),
            entry("t1", CatalogKind::Table, "orders", Some("public.orders")),
        ]);
        let ids_a: Vec<&str> = a.iter().map(|e| e.id.as_str()).collect();
        let ids_b: Vec<&str> = b.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids_a, ids_b);
    }
}
