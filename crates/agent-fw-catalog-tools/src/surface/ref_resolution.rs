use agent_fw_catalog::{
    CatalogEntry, CatalogScope, CatalogSearchBackend, CatalogSearchFilters, CatalogSearchRequest,
    DataCatalog, SemanticEntityKind,
};

use crate::CatalogToolError;

use super::types::{CatalogEntityKind, CatalogRef, CatalogRefResolution, ResolvedCatalogRef};

const FUZZY_FALLBACK_LIMIT: usize = 5;

pub struct CatalogRefResolver<'a> {
    catalog: &'a dyn DataCatalog,
    fuzzy: Option<FuzzyFallback<'a>>,
}

struct FuzzyFallback<'a> {
    scope: CatalogScope,
    backend: &'a dyn CatalogSearchBackend,
}

impl<'a> CatalogRefResolver<'a> {
    pub fn new(catalog: &'a dyn DataCatalog) -> Self {
        Self {
            catalog,
            fuzzy: None,
        }
    }

    pub fn with_search_backend(
        mut self,
        scope: CatalogScope,
        backend: &'a dyn CatalogSearchBackend,
    ) -> Self {
        self.fuzzy = Some(FuzzyFallback { scope, backend });
        self
    }

    pub async fn resolve(
        &self,
        reference: &CatalogRef,
    ) -> Result<CatalogRefResolution, CatalogToolError> {
        self.resolve_inner(reference, true).await
    }

    pub async fn resolve_exact(
        &self,
        reference: &CatalogRef,
    ) -> Result<CatalogRefResolution, CatalogToolError> {
        self.resolve_inner(reference, false).await
    }

    async fn resolve_inner(
        &self,
        reference: &CatalogRef,
        allow_fuzzy_fallback: bool,
    ) -> Result<CatalogRefResolution, CatalogToolError> {
        if reference.provided_reference_count() != 1 {
            return Ok(CatalogRefResolution::unresolved(
                reference.clone(),
                "exactly_one_of_id_qualified_name_or_name_required",
            ));
        }

        if let Some(kind) = reference.kind {
            if !kind.is_public_searchable() {
                return Ok(CatalogRefResolution::unresolved(
                    reference.clone(),
                    "special_kind_is_reserved",
                ));
            }
        }

        if let Some(id) = reference.id.as_deref() {
            return self.resolve_id(reference, id).await;
        }
        if let Some(qualified_name) = reference.qualified_name.as_deref() {
            return self
                .resolve_qualified_name(reference, qualified_name.trim(), allow_fuzzy_fallback)
                .await;
        }
        if let Some(name) = reference.name.as_deref() {
            return self
                .resolve_name(reference, name.trim(), allow_fuzzy_fallback)
                .await;
        }

        Ok(CatalogRefResolution::unresolved(
            reference.clone(),
            "empty_reference",
        ))
    }

    async fn resolve_id(
        &self,
        reference: &CatalogRef,
        id: &str,
    ) -> Result<CatalogRefResolution, CatalogToolError> {
        let Some(entry) = self.catalog.get_by_id(id.trim()).await? else {
            return Ok(CatalogRefResolution::unresolved(
                reference.clone(),
                "no_matching_id",
            ));
        };
        Ok(self.resolution_for_candidates(reference, vec![entry]))
    }

    async fn resolve_qualified_name(
        &self,
        reference: &CatalogRef,
        qualified_name: &str,
        allow_fuzzy_fallback: bool,
    ) -> Result<CatalogRefResolution, CatalogToolError> {
        let kinds = public_kinds_for(reference.kind);
        let mut candidates = Vec::new();
        for kind in kinds {
            if let Some(entry) = self
                .catalog
                .get_by_qualified_name(kind.into(), qualified_name)
                .await?
            {
                candidates.push(entry);
            }
        }

        if candidates.is_empty() && allow_fuzzy_fallback {
            if let Some(fuzzy) = self.resolve_fuzzy(reference, qualified_name).await? {
                return Ok(fuzzy);
            }
            return Ok(CatalogRefResolution::unresolved(
                reference.clone(),
                "no_matching_qualified_name",
            ));
        }

        Ok(self.resolution_for_candidates(reference, candidates))
    }

    async fn resolve_name(
        &self,
        reference: &CatalogRef,
        name: &str,
        allow_fuzzy_fallback: bool,
    ) -> Result<CatalogRefResolution, CatalogToolError> {
        if reference.kind.is_none() {
            return Ok(CatalogRefResolution::unresolved(
                reference.clone(),
                "kind_required_for_name_reference",
            ));
        }

        let kinds = public_kinds_for(reference.kind);
        let mut candidates = Vec::new();
        for kind in kinds {
            candidates.extend(self.catalog.get_by_name(kind.into(), name).await?);
        }

        if candidates.is_empty() && allow_fuzzy_fallback {
            if let Some(fuzzy) = self.resolve_fuzzy(reference, name).await? {
                return Ok(fuzzy);
            }
            return Ok(CatalogRefResolution::unresolved(
                reference.clone(),
                "no_matching_name",
            ));
        }

        Ok(self.resolution_for_candidates(reference, candidates))
    }

    async fn resolve_fuzzy(
        &self,
        reference: &CatalogRef,
        query: &str,
    ) -> Result<Option<CatalogRefResolution>, CatalogToolError> {
        let Some(fuzzy) = &self.fuzzy else {
            return Ok(None);
        };

        let kinds: Vec<SemanticEntityKind> = public_kinds_for(reference.kind)
            .into_iter()
            .map(SemanticEntityKind::from)
            .collect();
        let results = fuzzy
            .backend
            .search(
                &fuzzy.scope,
                CatalogSearchRequest {
                    query: query.to_string(),
                    kinds,
                    filters: CatalogSearchFilters::default(),
                    limit: FUZZY_FALLBACK_LIMIT,
                    cursor: None,
                },
            )
            .await?;
        let ids: Vec<String> = results.hits.into_iter().map(|hit| hit.entry_id).collect();
        let entries = self.catalog.get_by_ids(&ids).await?;
        let mut candidates = Vec::new();
        for id in ids {
            if let Some(entry) = entries.iter().find(|entry| entry.id == id) {
                if entry.kind.is_public_searchable() && matches_kind(reference.kind, entry) {
                    candidates.push(entry.clone());
                }
            }
        }

        if candidates.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.resolution_for_candidates(reference, candidates)))
    }

    fn resolution_for_candidates(
        &self,
        reference: &CatalogRef,
        candidates: Vec<CatalogEntry>,
    ) -> CatalogRefResolution {
        let mut resolved: Vec<ResolvedCatalogRef> = candidates
            .into_iter()
            .filter(|entry| entry.kind.is_public_searchable())
            .filter(|entry| matches_kind(reference.kind, entry))
            .map(|entry| ResolvedCatalogRef::from_entry(&entry))
            .collect();
        resolved.sort_by(|left, right| left.id.cmp(&right.id));
        resolved.dedup_by(|left, right| left.id == right.id);

        match resolved.len() {
            0 => CatalogRefResolution::unresolved(reference.clone(), "no_matching_public_entity"),
            1 => CatalogRefResolution::resolved(reference.clone(), resolved.remove(0)),
            _ => CatalogRefResolution::ambiguous(reference.clone(), resolved),
        }
    }
}

fn public_kinds_for(kind: Option<CatalogEntityKind>) -> Vec<CatalogEntityKind> {
    match kind {
        Some(kind) if kind.is_public_searchable() => vec![kind],
        Some(_) => Vec::new(),
        None => CatalogEntityKind::PUBLIC_SEARCHABLE.to_vec(),
    }
}

fn matches_kind(kind: Option<CatalogEntityKind>, entry: &CatalogEntry) -> bool {
    kind.is_none_or(|kind| CatalogEntityKind::from(entry.kind) == kind)
}
