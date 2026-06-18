use serde::{Deserialize, Serialize};

use crate::CatalogKind;

/// User- or system-supplied reference to a catalog entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CatalogRef {
    Id(String),
    QualifiedName {
        kind: Option<CatalogKind>,
        qualified_name: String,
    },
    Name {
        kind: CatalogKind,
        name: String,
        schema: Option<String>,
    },
}

impl CatalogRef {
    pub fn parse_table(input: &str) -> Self {
        let input = input.trim();
        if is_direct_catalog_id(input) {
            Self::Id(input.to_string())
        } else if input.contains('.') {
            Self::QualifiedName {
                kind: Some(CatalogKind::Table),
                qualified_name: input.to_string(),
            }
        } else {
            Self::Name {
                kind: CatalogKind::Table,
                name: input.to_string(),
                schema: None,
            }
        }
    }

    pub fn parse_column(input: &str) -> Self {
        let input = input.trim();
        if is_direct_catalog_id(input) {
            Self::Id(input.to_string())
        } else if input.contains('.') {
            Self::QualifiedName {
                kind: Some(CatalogKind::Column),
                qualified_name: input.to_string(),
            }
        } else {
            Self::Name {
                kind: CatalogKind::Column,
                name: input.to_string(),
                schema: None,
            }
        }
    }
}

fn is_direct_catalog_id(input: &str) -> bool {
    (input.len() == 64 && input.chars().all(|c| c.is_ascii_hexdigit()))
        || input.starts_with("table:")
        || input.starts_with("column:")
        || input.starts_with("enum:")
        || input.starts_with("relationship:")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedCatalogRef {
    pub id: String,
    pub kind: CatalogKind,
    pub name: String,
    pub qualified_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_refs_parse_ids_and_qualified_names() {
        let id = "a".repeat(64);
        assert_eq!(CatalogRef::parse_table(&id), CatalogRef::Id(id));

        assert_eq!(
            CatalogRef::parse_table("public.fact_sales"),
            CatalogRef::QualifiedName {
                kind: Some(CatalogKind::Table),
                qualified_name: "public.fact_sales".to_string(),
            }
        );
        assert_eq!(
            CatalogRef::parse_column("order_status"),
            CatalogRef::Name {
                kind: CatalogKind::Column,
                name: "order_status".to_string(),
                schema: None,
            }
        );
    }

    #[test]
    fn semantic_refs_keep_prefixed_catalog_ids_as_ids() {
        assert_eq!(
            CatalogRef::parse_table("table:public.fact_sales"),
            CatalogRef::Id("table:public.fact_sales".to_string())
        );
        assert_eq!(
            CatalogRef::parse_column("column:public.fact_sales.order_status"),
            CatalogRef::Id("column:public.fact_sales.order_status".to_string())
        );
    }

    #[test]
    fn semantic_refs_normalize_surrounding_whitespace() {
        assert_eq!(
            CatalogRef::parse_table(" public.fact_sales "),
            CatalogRef::QualifiedName {
                kind: Some(CatalogKind::Table),
                qualified_name: "public.fact_sales".to_string(),
            }
        );
        assert_eq!(
            CatalogRef::parse_column(" order_status "),
            CatalogRef::Name {
                kind: CatalogKind::Column,
                name: "order_status".to_string(),
                schema: None,
            }
        );
    }
}
