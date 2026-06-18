use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use agent_fw_catalog::{
    relation_kind, CatalogEntry, CatalogKind, DataCatalog, RelationshipMetadata, SemanticEntity,
};

use crate::CatalogToolError;

use super::entity_output::CatalogEntityAssembler;
use super::filters::is_allowed_relation_kind;
use super::ref_resolution::CatalogRefResolver;
use super::types::{
    CatalogEntity, CatalogEntityKind, CatalogGraphEdge, CatalogRelationsForRef,
    GetCatalogRelationsInput, GetCatalogRelationsOutput, Pagination, RelationDirection,
};

pub(crate) const RELATIONSHIP_SCAN_LIMIT: usize = 10_000;
const MAX_RELATION_REFS: usize = 20;

pub struct CatalogGraphService<'a> {
    catalog: &'a dyn DataCatalog,
    resolver: CatalogRefResolver<'a>,
    assembler: CatalogEntityAssembler,
    /// Optional precomputed relationship vertices. When present,
    /// `relationship_vertex_edges` iterates this slice instead of issuing a
    /// fresh `list_by_type(Relationship, ..)` scan per call — letting a BFS
    /// caller load the set once and reuse it across every node.
    relationships: Option<Arc<Vec<CatalogEntry>>>,
}

impl<'a> CatalogGraphService<'a> {
    pub fn new(
        catalog: &'a dyn DataCatalog,
        resolver: CatalogRefResolver<'a>,
        assembler: CatalogEntityAssembler,
    ) -> Self {
        Self {
            catalog,
            resolver,
            assembler,
            relationships: None,
        }
    }

    /// Like [`CatalogGraphService::new`], but reuses a precomputed relationship
    /// vertex set instead of scanning the catalog on every `get_relations`
    /// call. Used by path search to avoid an O(nodes * relationships) rescan.
    pub fn new_with_relationships(
        catalog: &'a dyn DataCatalog,
        resolver: CatalogRefResolver<'a>,
        assembler: CatalogEntityAssembler,
        relationships: Arc<Vec<CatalogEntry>>,
    ) -> Self {
        Self {
            catalog,
            resolver,
            assembler,
            relationships: Some(relationships),
        }
    }

    pub async fn get_relations(
        &self,
        input: GetCatalogRelationsInput,
    ) -> Result<GetCatalogRelationsOutput, CatalogToolError> {
        if input.refs.is_empty() || input.refs.len() > MAX_RELATION_REFS {
            return Err(CatalogToolError::Validation(format!(
                "get_catalog_relations.refs must contain 1..={MAX_RELATION_REFS} refs"
            )));
        }
        let direction = input.direction.unwrap_or_default();
        let limit = input.limit_per_ref.unwrap_or(25).clamp(1, 100);
        let relation_filter_active = !input.relation_kinds.is_empty();
        let mut relation_filter = HashSet::new();
        let mut warnings = Vec::new();
        for relation_kind in input.relation_kinds {
            if is_allowed_relation_kind(&relation_kind) {
                relation_filter.insert(relation_kind);
            } else {
                warnings.push(format!("unknown relation_kind filter: {relation_kind}"));
            }
        }
        let target_filter: HashSet<CatalogEntityKind> = input.target_kinds.into_iter().collect();
        let mut results = Vec::new();
        let relationships = match &self.relationships {
            Some(precomputed) => precomputed.clone(),
            None => Arc::new(self.load_relationship_vertices(&mut warnings).await?),
        };

        for reference in input.refs {
            let resolved = self.resolver.resolve(&reference).await?;
            let Some(source_ref) = resolved.resolved else {
                warnings.push(format!(
                    "could not resolve graph source {}",
                    reference.display_input()
                ));
                continue;
            };
            let Some(source_entry) = self.catalog.get_by_id(&source_ref.id).await? else {
                warnings.push(format!(
                    "resolved graph source {} disappeared",
                    source_ref.id
                ));
                continue;
            };
            let source_assembly = self.assembler.assemble(source_entry.clone());
            let Some(source) = source_assembly.entity else {
                warnings.extend(source_assembly.warnings);
                continue;
            };

            let mut edges = Vec::new();
            if matches!(
                direction,
                RelationDirection::Outgoing | RelationDirection::Both
            ) {
                edges.extend(
                    self.outgoing_edges(
                        &source_entry,
                        &relation_filter,
                        relation_filter_active,
                        &target_filter,
                    )
                    .await?,
                );
            }
            if matches!(
                direction,
                RelationDirection::Incoming | RelationDirection::Both
            ) {
                edges.extend(
                    self.incoming_edges(
                        &source_entry,
                        &relation_filter,
                        relation_filter_active,
                        &target_filter,
                    )
                    .await?,
                );
            }
            edges.extend(
                self.relationship_vertex_edges(
                    &source_entry,
                    direction,
                    relationships.as_slice(),
                    &relation_filter,
                    relation_filter_active,
                    &target_filter,
                )
                .await?,
            );
            dedupe_edges(&mut edges);

            let has_more = edges.len() > limit;
            edges.truncate(limit);
            let returned = edges.len();
            let mut source_warnings = source_assembly.warnings;
            if has_more {
                source_warnings.push(format!(
                    "relations truncated to limit_per_ref={limit}; apply relation_kinds or target_kinds filters for complete edge classes"
                ));
            }
            results.push(CatalogRelationsForRef {
                source,
                relations: edges,
                pagination: Pagination {
                    limit,
                    returned,
                    has_more,
                    next_cursor: None,
                },
                warnings: source_warnings,
            });
        }

        Ok(GetCatalogRelationsOutput { results, warnings })
    }

    async fn outgoing_edges(
        &self,
        source: &CatalogEntry,
        relation_filter: &HashSet<String>,
        relation_filter_active: bool,
        target_filter: &HashSet<CatalogEntityKind>,
    ) -> Result<Vec<CatalogGraphEdge>, CatalogToolError> {
        let ids: Vec<String> = source
            .links
            .iter()
            .map(|link| link.target_id.clone())
            .collect();
        let targets = self.catalog.get_by_ids(&ids).await?;
        let target_map: HashMap<&str, &CatalogEntry> = targets
            .iter()
            .map(|entry| (entry.id.as_str(), entry))
            .collect();
        let mut edges = Vec::new();
        for link in &source.links {
            if !relation_matches(relation_filter, relation_filter_active, &link.kind) {
                continue;
            }
            let Some(target) = target_map.get(link.target_id.as_str()) else {
                continue;
            };
            if !target_kind_matches(target_filter, target) {
                continue;
            }
            if let Some(target_entity) = self.assemble_target((*target).clone()) {
                edges.push(CatalogGraphEdge {
                    relation_kind: link.kind.clone(),
                    direction: RelationDirection::Outgoing,
                    target: target_entity,
                    description: link.description.clone(),
                    relationship: None,
                });
            }
        }
        Ok(edges)
    }

    async fn incoming_edges(
        &self,
        source: &CatalogEntry,
        relation_filter: &HashSet<String>,
        relation_filter_active: bool,
        target_filter: &HashSet<CatalogEntityKind>,
    ) -> Result<Vec<CatalogGraphEdge>, CatalogToolError> {
        let incoming = self.catalog.get_related_reverse(&source.id, None).await?;
        let mut edges = Vec::new();
        for target in incoming {
            if !target_kind_matches(target_filter, &target) {
                continue;
            }
            for link in target
                .links
                .iter()
                .filter(|link| link.target_id == source.id)
            {
                if !relation_matches(relation_filter, relation_filter_active, &link.kind) {
                    continue;
                }
                if let Some(target_entity) = self.assemble_target(target.clone()) {
                    edges.push(CatalogGraphEdge {
                        relation_kind: link.kind.clone(),
                        direction: RelationDirection::Incoming,
                        target: target_entity,
                        description: link.description.clone(),
                        relationship: None,
                    });
                }
            }
        }
        Ok(edges)
    }

    async fn relationship_vertex_edges(
        &self,
        source: &CatalogEntry,
        direction: RelationDirection,
        relationships: &[CatalogEntry],
        relation_filter: &HashSet<String>,
        relation_filter_active: bool,
        target_filter: &HashSet<CatalogEntityKind>,
    ) -> Result<Vec<CatalogGraphEdge>, CatalogToolError> {
        let mut edges = Vec::new();
        for relationship in relationships {
            let Ok(SemanticEntity::Relationship { metadata, .. }) =
                SemanticEntity::try_from(relationship.clone())
            else {
                continue;
            };
            if metadata.source_table_id == source.id
                && matches!(
                    direction,
                    RelationDirection::Outgoing | RelationDirection::Both
                )
            {
                self.push_relationship_edge(
                    relation_kind::REFERENCES_TABLE,
                    RelationDirection::Outgoing,
                    metadata.target_table_id.clone(),
                    &metadata,
                    relationship.clone(),
                    relation_filter,
                    relation_filter_active,
                    target_filter,
                    &mut edges,
                )
                .await?;
            }
            if metadata.target_table_id == source.id
                && matches!(
                    direction,
                    RelationDirection::Incoming | RelationDirection::Both
                )
            {
                self.push_relationship_edge(
                    relation_kind::REFERENCED_BY_TABLE,
                    RelationDirection::Incoming,
                    metadata.source_table_id.clone(),
                    &metadata,
                    relationship.clone(),
                    relation_filter,
                    relation_filter_active,
                    target_filter,
                    &mut edges,
                )
                .await?;
            }
        }
        Ok(edges)
    }

    async fn load_relationship_vertices(
        &self,
        warnings: &mut Vec<String>,
    ) -> Result<Vec<CatalogEntry>, CatalogToolError> {
        let relationships = self
            .catalog
            .list_by_type(CatalogKind::Relationship, RELATIONSHIP_SCAN_LIMIT)
            .await?;
        if relationships.len() == RELATIONSHIP_SCAN_LIMIT {
            warnings.push(format!(
                "relationship vertex scan reached limit {RELATIONSHIP_SCAN_LIMIT}; results may be incomplete"
            ));
        }
        Ok(relationships)
    }

    async fn push_relationship_edge(
        &self,
        relation_kind: &str,
        direction: RelationDirection,
        target_id: String,
        metadata: &RelationshipMetadata,
        relationship: CatalogEntry,
        relation_filter: &HashSet<String>,
        relation_filter_active: bool,
        target_filter: &HashSet<CatalogEntityKind>,
        edges: &mut Vec<CatalogGraphEdge>,
    ) -> Result<(), CatalogToolError> {
        if !relation_matches(relation_filter, relation_filter_active, relation_kind) {
            return Ok(());
        }
        let Some(target) = self.catalog.get_by_id(&target_id).await? else {
            return Ok(());
        };
        if !target_kind_matches(target_filter, &target) {
            return Ok(());
        }
        let Some(target_entity) = self.assemble_target(target) else {
            return Ok(());
        };
        let relationship_entity = self.assembler.assemble(relationship).entity;
        edges.push(CatalogGraphEdge {
            relation_kind: relation_kind.to_string(),
            direction,
            target: target_entity,
            description: Some(format!(
                "{}.{} -> {}.{}",
                metadata.source_table,
                metadata.source_column,
                metadata.target_table,
                metadata.target_column
            )),
            relationship: relationship_entity,
        });
        Ok(())
    }

    fn assemble_target(&self, entry: CatalogEntry) -> Option<CatalogEntity> {
        self.assembler.assemble(entry).entity
    }
}

fn relation_matches(filter: &HashSet<String>, filter_active: bool, relation_kind: &str) -> bool {
    !filter_active || filter.contains(relation_kind)
}

fn target_kind_matches(filter: &HashSet<CatalogEntityKind>, entry: &CatalogEntry) -> bool {
    filter.is_empty() || filter.contains(&CatalogEntityKind::from(entry.kind))
}

/// Collapse edges that describe the same logical relation.
///
/// In production the interpreters materialize a table→table FK as a
/// `references_table` link on the source table (loaded into `links`, so
/// `outgoing_edges` emits it with `relationship: None`) AND store a
/// relationship vertex, which `relationship_vertex_edges` emits as the same
/// `(direction, relation_kind, target)` edge carrying `relationship: Some(..)`.
/// Keying dedup on the relationship id would treat the `None`/`Some` pair as
/// distinct edges and keep both, so we key only on the logical relation and
/// merge the relationship vertex (and its richer join description) onto the
/// first-seen survivor. `outgoing_edges`/`incoming_edges` are appended before
/// `relationship_vertex_edges`, so the link copy is the survivor and the vertex
/// copy is the merge source — order-stable and deterministic.
fn dedupe_edges(edges: &mut Vec<CatalogGraphEdge>) {
    let mut index: HashMap<(RelationDirection, String, String), usize> = HashMap::new();
    let mut collapsed: Vec<CatalogGraphEdge> = Vec::with_capacity(edges.len());
    for edge in edges.drain(..) {
        let key = (
            edge.direction,
            edge.relation_kind.clone(),
            edge.target.id.clone(),
        );
        match index.get(&key) {
            Some(&position) => merge_edge(&mut collapsed[position], edge),
            None => {
                index.insert(key, collapsed.len());
                collapsed.push(edge);
            }
        }
    }
    *edges = collapsed;
}

/// Fold a duplicate edge into the surviving one, preferring the relationship
/// vertex and its description when the survivor lacks them.
fn merge_edge(survivor: &mut CatalogGraphEdge, incoming: CatalogGraphEdge) {
    if survivor.relationship.is_none() && incoming.relationship.is_some() {
        survivor.relationship = incoming.relationship;
        if incoming.description.is_some() {
            survivor.description = incoming.description;
        }
    } else if survivor.description.is_none() {
        survivor.description = incoming.description;
    }
}
