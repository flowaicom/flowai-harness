//! Ingestion orchestrator — composes all layers into the profiling pipeline.
//!
//! Pipeline per table:
//! 1. Introspect → PhysicalTable
//! 2. Sample → Vec<Value>
//! 3. Profile → TableProfile
//! 4. Enrich → SemanticTableProfile (with retry + timeout)
//! 5. Extract enums
//! 6. Build catalog entries (pure)
//! 7. Save → CatalogWriter::save_in_transaction
//!
//! # Framework-level behavior
//!
//! - Uses `retry_when_observed` for retry policy
//! - Uses `with_timeout` + `CancellationToken::run()` instead of `timeout_or_cancel`
//! - KV key functions are local (no `kv_keys` module dependency)

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;
use tracing::Instrument;

use agent_fw_algebra::{
    retry::retry_when_observed, retry::RetryPolicy, timeout::with_timeout, CancellationToken,
    KVStore, KVStoreExt, TargetDatabase,
};
use agent_fw_catalog::{
    provenance_origin, CatalogProvenance, CatalogWriter, EnrichmentError, EnrichmentResult,
    EnrichmentSource, ForeignKeyEdge, IngestionEvent, IngestionStatus, IngestionSummary,
    PhysicalTable, QualityNote, SemanticEnricher, TableEnrichmentRequest, TableInfo,
};

use crate::builder;
use crate::introspection::IntrospectionService;
use crate::profiling::{self, ProfilingService};

/// Standard TTL for ephemeral KV entries (24 hours).
const EPHEMERAL_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Timeout for LLM enrichment calls.
const ENRICHMENT_TIMEOUT: Duration = Duration::from_secs(60);

/// Retry count for enrichment after the first attempt.
const ENRICHMENT_MAX_RETRIES: u32 = 2;

/// Max table-level concurrency during database-wide profiling.
const TABLE_CONCURRENCY: usize = 4;

/// Default KV key prefix for ingestion job status.
const INGEST_STATUS_PREFIX: &str = "ingest:status:";

/// KV persistence policy for ingestion orchestration artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestionPersistencePolicy {
    status_key_prefix: String,
}

impl Default for IngestionPersistencePolicy {
    fn default() -> Self {
        Self {
            status_key_prefix: INGEST_STATUS_PREFIX.to_string(),
        }
    }
}

impl IngestionPersistencePolicy {
    /// Override the status-key prefix.
    pub fn with_status_key_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.status_key_prefix = prefix.into();
        self
    }

    pub fn status_key(&self, job_id: &str) -> String {
        format!("{}{}", self.status_key_prefix, job_id)
    }
}

/// Retry policy for enrichment: exponential backoff with jitter, 2 retries.
fn enrichment_retry_policy() -> RetryPolicy {
    RetryPolicy::exponential_backoff_jitter(ENRICHMENT_MAX_RETRIES, Duration::from_millis(500))
        .with_max_delay(Duration::from_secs(10))
}

fn enrichment_source_value(source: EnrichmentSource) -> String {
    match source {
        EnrichmentSource::Fresh => "fresh",
        EnrichmentSource::Cached => "cached",
        EnrichmentSource::Fallback => "fallback",
    }
    .to_string()
}

fn relationship_origin_value(source: EnrichmentSource) -> &'static str {
    match source {
        EnrichmentSource::Fresh => provenance_origin::LLM_ENRICHMENT,
        EnrichmentSource::Cached => provenance_origin::CACHED_ENRICHMENT,
        EnrichmentSource::Fallback => provenance_origin::FALLBACK,
    }
}

/// Send an event, returning `false` if the receiver has disconnected.
async fn emit(tx: &mpsc::Sender<IngestionEvent>, event: IngestionEvent) -> bool {
    tx.send(event).await.is_ok()
}

// =============================================================================
// PipelineCtx — factors out cancel-check + KV-update + SSE-emit
// =============================================================================

struct PipelineCtx<'a> {
    tenant_id: &'a str,
    status_key: &'a str,
    kv: &'a dyn KVStore,
    tx: &'a mpsc::Sender<IngestionEvent>,
    cancel: &'a CancellationToken,
}

impl<'a> PipelineCtx<'a> {
    /// Run a stage: check cancellation, update KV status, emit progress, execute.
    /// Returns `None` if cancelled or client disconnected.
    async fn run<T, F, Fut>(&self, status: IngestionStatus, op: F) -> Option<Result<T, String>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, String>>,
    {
        if self.cancel.is_cancelled() {
            let _ = emit(
                self.tx,
                IngestionEvent::Error {
                    message: "Cancelled".into(),
                },
            )
            .await;
            return None;
        }
        let _ = self
            .kv
            .put(
                self.tenant_id,
                self.status_key,
                &status,
                Some(EPHEMERAL_TTL),
            )
            .await;
        if !emit(self.tx, IngestionEvent::Progress { status }).await {
            return None;
        }
        let result = op().await;
        if self.cancel.is_cancelled() {
            let _ = emit(
                self.tx,
                IngestionEvent::Error {
                    message: "Cancelled".into(),
                },
            )
            .await;
            return None;
        }
        Some(result)
    }

    async fn is_alive(&self) -> bool {
        if self.cancel.is_cancelled() {
            let _ = emit(
                self.tx,
                IngestionEvent::Error {
                    message: "Cancelled".into(),
                },
            )
            .await;
            return false;
        }
        true
    }

    async fn emit_progress(&self, status: IngestionStatus) -> bool {
        let _ = self
            .kv
            .put(
                self.tenant_id,
                self.status_key,
                &status,
                Some(EPHEMERAL_TTL),
            )
            .await;
        emit(self.tx, IngestionEvent::Progress { status }).await
    }
}

// =============================================================================
// Named parameter types
// =============================================================================

/// Parameters for profiling a single table.
pub struct ProfileTableParams<'a> {
    pub tenant_id: &'a str,
    pub job_id: &'a str,
    pub profiling_run_id: Option<&'a str>,
    pub schema: &'a str,
    pub table: &'a str,
    pub database_id: &'a str,
    pub sample_size: usize,
    pub tx: &'a mpsc::Sender<IngestionEvent>,
    pub cancel: &'a CancellationToken,
    pub database_context: Option<&'a str>,
    pub fk_edges: &'a [ForeignKeyEdge],
}

/// Parameters for profiling a database or a selected table subset.
pub struct ProfileDatabaseParams<'a> {
    pub tenant_id: &'a str,
    pub job_id: &'a str,
    pub schema: &'a str,
    pub database_id: &'a str,
    pub tx: &'a mpsc::Sender<IngestionEvent>,
    pub cancel: &'a CancellationToken,
    pub selected_tables: Option<&'a [String]>,
    pub sample_size: usize,
}

/// Result of Phase 1 schema discovery.
struct SchemaDiscovery {
    tables: Vec<TableInfo>,
    database_context: Option<String>,
    fk_edges: Vec<ForeignKeyEdge>,
}

/// Result of profiling one table in the parallel pipeline.
struct TableProfileResult {
    table_name: String,
    summary: Option<IngestionSummary>,
    error: Option<String>,
}

/// Configuration for spawning a per-table profiler task.
struct TableProfilerConfig {
    sub_job_id: String,
    root_job_id: String,
    table_schema: String,
    table_name: String,
    database_id: String,
    sample_size: usize,
    tenant_id: String,
    db: Arc<dyn TargetDatabase>,
    enricher: Arc<dyn SemanticEnricher>,
    writer: Arc<dyn CatalogWriter>,
    kv: Arc<dyn KVStore>,
    database_context: Option<String>,
    /// FK edges involving this table, so the enricher sees cross-table context.
    fk_edges: Vec<ForeignKeyEdge>,
    persistence: IngestionPersistencePolicy,
    cancel: CancellationToken,
    main_tx: mpsc::Sender<IngestionEvent>,
}

// =============================================================================
// IngestionOrchestrator
// =============================================================================

/// The ingestion orchestrator composes all service layers.
pub struct IngestionOrchestrator {
    db: Arc<dyn TargetDatabase>,
    enricher: Arc<dyn SemanticEnricher>,
    writer: Arc<dyn CatalogWriter>,
    kv: Arc<dyn KVStore>,
    persistence: IngestionPersistencePolicy,
}

impl IngestionOrchestrator {
    pub fn new(
        db: Arc<dyn TargetDatabase>,
        enricher: Arc<dyn SemanticEnricher>,
        writer: Arc<dyn CatalogWriter>,
        kv: Arc<dyn KVStore>,
    ) -> Self {
        Self::new_with_persistence(
            db,
            enricher,
            writer,
            kv,
            IngestionPersistencePolicy::default(),
        )
    }

    pub fn new_with_persistence(
        db: Arc<dyn TargetDatabase>,
        enricher: Arc<dyn SemanticEnricher>,
        writer: Arc<dyn CatalogWriter>,
        kv: Arc<dyn KVStore>,
        persistence: IngestionPersistencePolicy,
    ) -> Self {
        Self {
            db,
            enricher,
            writer,
            kv,
            persistence,
        }
    }

    /// Profile a single table and emit SSE events.
    pub async fn profile_single_table(&self, req: ProfileTableParams<'_>) {
        let ProfileTableParams {
            tenant_id,
            job_id,
            profiling_run_id,
            schema,
            table,
            database_id,
            sample_size,
            tx,
            cancel,
            database_context,
            fk_edges,
        } = req;
        let durable_profiling_run_id = profiling_run_id.unwrap_or(job_id);

        tracing::info!(
            job_id,
            database_id,
            schema,
            table,
            sample_size,
            "starting table profiling"
        );

        if !emit(
            tx,
            IngestionEvent::Started {
                job_id: job_id.to_string(),
            },
        )
        .await
        {
            return;
        }

        let status_key = self.persistence.status_key(job_id);
        let _ = self
            .kv
            .put(
                tenant_id,
                &status_key,
                &IngestionStatus::Queued,
                Some(EPHEMERAL_TTL),
            )
            .await;

        let ctx = PipelineCtx {
            tenant_id,
            status_key: &status_key,
            kv: self.kv.as_ref(),
            tx,
            cancel,
        };

        let introspection = IntrospectionService::new(Arc::clone(&self.db));
        let profiling_svc = ProfilingService::new(Arc::clone(&self.db));

        // 1. Introspect
        tracing::info!(schema, table, "introspecting table");
        let physical = match ctx
            .run(IngestionStatus::Discovering { tables_found: 1 }, || async {
                introspection
                    .introspect_table(schema, table)
                    .await
                    .map_err(|e| format!("Introspection failed: {}", e))
            })
            .await
        {
            Some(Ok(p)) => p,
            Some(Err(e)) => {
                let _ = emit(
                    tx,
                    IngestionEvent::TableFailed {
                        table_name: table.to_string(),
                        error: e.clone(),
                    },
                )
                .await;
                let _ = emit(tx, IngestionEvent::Error { message: e }).await;
                return;
            }
            None => return,
        };
        tracing::info!(
            schema,
            table,
            columns = physical.columns.len(),
            "table introspection complete"
        );

        // 2. Sample (soft failure)
        tracing::info!(schema, table, sample_size, "sampling table rows");
        let (samples, sample_failed) =
            match introspection.sample_rows(schema, table, sample_size).await {
                Ok(s) => {
                    tracing::info!(schema, table, rows = s.len(), "table row sampling complete");
                    (s, false)
                }
                Err(e) => {
                    tracing::warn!(schema, table, "Failed to sample: {}", e);
                    (vec![], true)
                }
            };
        if !ctx.is_alive().await {
            return;
        }

        // 3. Profile
        let total_columns = physical.columns.len() as u32;
        let start = std::time::Instant::now();
        tracing::info!(
            schema,
            table,
            columns = total_columns,
            "profiling table columns"
        );
        let profile = match ctx
            .run(
                IngestionStatus::Profiling {
                    tables_found: 1,
                    columns_profiled: 0,
                    total_columns,
                },
                || async {
                    profiling_svc
                        .profile_table(&physical, &samples)
                        .await
                        .map_err(|e| format!("Profiling failed: {}", e))
                },
            )
            .await
        {
            Some(Ok(p)) => p,
            Some(Err(e)) => {
                let _ = emit(
                    tx,
                    IngestionEvent::TableFailed {
                        table_name: table.to_string(),
                        error: e.clone(),
                    },
                )
                .await;
                let _ = emit(tx, IngestionEvent::Error { message: e }).await;
                return;
            }
            None => return,
        };
        let profile_duration = start.elapsed().as_millis() as u64;
        tracing::info!(
            schema,
            table,
            columns = total_columns,
            duration_ms = profile_duration,
            "table column profiling complete"
        );

        if !emit(
            tx,
            IngestionEvent::TableProfiled {
                table_name: table.to_string(),
                columns: total_columns,
                duration_ms: profile_duration,
            },
        )
        .await
        {
            return;
        }

        // 4. Enrich via LLM
        if !ctx
            .emit_progress(IngestionStatus::Enriching {
                tables_enriched: 0,
                total_tables: 1,
            })
            .await
        {
            return;
        }

        let enrichment_request = TableEnrichmentRequest {
            table: physical.clone(),
            sample_rows: samples,
            profile: profile.clone(),
            database_context: database_context.map(String::from),
            fk_edges: fk_edges.to_vec(),
        };

        let enrichment_start = std::time::Instant::now();
        tracing::info!(schema, table, "enriching table semantics");
        let enrichment = self
            .enrich_with_retry_and_timeout(enrichment_request, cancel, schema, table, &physical)
            .await;

        let enrichment_source = enrichment.source;
        let enrichment_model_id = enrichment.model_id.clone();
        let enrichment_fallback_reason = if enrichment_source == EnrichmentSource::Fallback {
            enrichment
                .fallback_reason
                .clone()
                .or_else(|| Some("enricher reported fallback".to_string()))
        } else {
            None
        };
        let enrichment_degraded = enrichment_source == EnrichmentSource::Fallback;
        let mut semantic = enrichment.profile;
        tracing::info!(
            schema,
            table,
            source = ?enrichment_source,
            duration_ms = enrichment_start.elapsed().as_millis() as u64,
            "table enrichment complete"
        );

        if sample_failed {
            semantic.quality_notes.push(QualityNote {
                column_name: "*".to_string(),
                notes: "Sample rows unavailable; enrichment relied on schema only".to_string(),
                typical_value_range: None,
                validation_rules: vec![],
            });
        }

        if !emit(
            tx,
            IngestionEvent::TableEnriched {
                table_name: table.to_string(),
                source: enrichment_source,
                fallback_reason: enrichment_fallback_reason.clone(),
            },
        )
        .await
        {
            return;
        }

        if !ctx.is_alive().await {
            return;
        }

        // 5. Extract enums (pure)
        if !ctx
            .emit_progress(IngestionStatus::Extracting { enums_extracted: 0 })
            .await
        {
            return;
        }

        let enums = profiling::extract_enums_from_profile(
            &profile,
            &physical.columns,
            builder::LOW_CARDINALITY_ENUM_THRESHOLD,
        );
        let enums_extracted = enums.len() as u32;

        if !ctx.is_alive().await {
            return;
        }

        // 6. Build catalog entries (pure)
        let provenance = CatalogProvenance {
            origin: None,
            profiling_run_id: Some(durable_profiling_run_id.to_string()),
            enrichment_source: Some(enrichment_source_value(enrichment_source)),
            model_id: enrichment_model_id,
            fallback_reason: enrichment_fallback_reason,
            schema_snapshot_at: None,
            target_fingerprint: None,
        };
        let mut catalog_items = vec![builder::build_table_entry_with_provenance(
            &physical,
            &semantic,
            database_id,
            provenance.clone(),
        )];
        catalog_items.extend(builder::build_column_entries(
            &physical,
            &semantic,
            &profile,
            database_id,
        ));
        let physical_relationship_provenance = CatalogProvenance {
            origin: Some(provenance_origin::PHYSICAL_SCHEMA.to_string()),
            enrichment_source: None,
            model_id: None,
            fallback_reason: None,
            ..provenance.clone()
        };
        let physical_relationships = builder::build_physical_relationship_entries_with_provenance(
            &physical,
            database_id,
            physical_relationship_provenance,
        );
        let physical_relationship_count = physical_relationships.len();
        catalog_items.extend(physical_relationships);
        let semantic_relationship_provenance = CatalogProvenance {
            origin: Some(relationship_origin_value(enrichment_source).to_string()),
            ..provenance.clone()
        };
        catalog_items.extend(builder::build_relationship_entries_with_provenance(
            &semantic,
            schema,
            database_id,
            semantic_relationship_provenance,
        ));
        catalog_items.extend(builder::build_enum_entries_with_detection(
            &enums,
            table,
            schema,
            database_id,
        ));
        catalog_items.extend(builder::build_quality_note_entries(
            &physical,
            &semantic,
            database_id,
            provenance,
        ));
        let items_count = catalog_items.len() as u32;

        // 7. Save via CatalogWriter
        tracing::info!(
            schema,
            table,
            catalog_items = items_count,
            "saving catalog entries"
        );
        match ctx
            .run(IngestionStatus::Indexing { items_indexed: 0 }, || async {
                self.writer
                    .save_in_transaction(catalog_items)
                    .await
                    .map_err(|e| format!("Failed to save catalog entries: {}", e))
            })
            .await
        {
            Some(Ok(_)) => {
                tracing::info!(
                    schema,
                    table,
                    catalog_items = items_count,
                    "catalog entries saved"
                );
                let summary = IngestionSummary {
                    tables_discovered: 1,
                    columns_profiled: total_columns,
                    enums_extracted,
                    relationships_found: (semantic.relationships.len()
                        + physical_relationship_count)
                        as u32,
                    catalog_items_indexed: items_count,
                    duration_ms: start.elapsed().as_millis() as u64,
                    enrichment_degraded,
                    enrichment_cache_hits: if enrichment_source == EnrichmentSource::Cached {
                        1
                    } else {
                        0
                    },
                    enrichment_fallbacks: if enrichment_degraded { 1 } else { 0 },
                    enrichment_fresh: if enrichment_source == EnrichmentSource::Fresh {
                        1
                    } else {
                        0
                    },
                };
                let _ = self
                    .kv
                    .put(
                        tenant_id,
                        &status_key,
                        &IngestionStatus::Completed {
                            summary: summary.clone(),
                        },
                        Some(EPHEMERAL_TTL),
                    )
                    .await;
                let _ = emit(
                    tx,
                    IngestionEvent::TableCompleted {
                        table_name: table.to_string(),
                        summary: summary.clone(),
                    },
                )
                .await;
                let _ = emit(tx, IngestionEvent::Completed { summary }).await;
            }
            Some(Err(e)) => {
                let _ = emit(
                    tx,
                    IngestionEvent::TableFailed {
                        table_name: table.to_string(),
                        error: e.clone(),
                    },
                )
                .await;
                let _ = emit(tx, IngestionEvent::Error { message: e }).await;
            }
            None => return,
        }
    }

    /// Profile all tables in a schema, accumulating summaries via the monoid combine.
    pub async fn profile_database(
        &self,
        tenant_id: &str,
        job_id: &str,
        schema: &str,
        database_id: &str,
        tx: &mpsc::Sender<IngestionEvent>,
        cancel: &CancellationToken,
    ) {
        self.profile_database_with_params(ProfileDatabaseParams {
            tenant_id,
            job_id,
            schema,
            database_id,
            tx,
            cancel,
            selected_tables: None,
            sample_size: 10,
        })
        .await;
    }

    /// Profile a database with optional table filtering and configurable row sampling.
    pub async fn profile_database_with_params(&self, req: ProfileDatabaseParams<'_>) {
        let ProfileDatabaseParams {
            tenant_id,
            job_id,
            schema,
            database_id,
            tx,
            cancel,
            selected_tables,
            sample_size,
        } = req;
        let start = std::time::Instant::now();
        let selected_table_count = selected_tables.map(|tables| tables.len()).unwrap_or(0);
        tracing::info!(
            job_id,
            database_id,
            schema,
            selected_table_count,
            all_tables = selected_tables.is_none(),
            sample_size,
            "starting database profiling"
        );
        if !emit(
            tx,
            IngestionEvent::Started {
                job_id: job_id.to_string(),
            },
        )
        .await
        {
            return;
        }

        // Phase 1: Discover schema structure
        let discovery = match self.discover_schema(schema, tx).await {
            Some(d) => d,
            None => return,
        };

        let filtered_tables = match Self::filter_requested_tables(discovery.tables, selected_tables)
        {
            Ok(tables) => tables,
            Err(message) => {
                let _ = emit(tx, IngestionEvent::Error { message }).await;
                return;
            }
        };

        let total_tables = filtered_tables.len();
        let total_columns_estimate = Self::estimate_total_columns(&filtered_tables);
        tracing::info!(
            job_id,
            schema,
            tables = total_tables,
            estimated_columns = total_columns_estimate,
            "database profiling scope resolved"
        );

        // Phase 2: Profile tables in parallel (with per-table FK edges)
        tracing::info!(
            job_id,
            tables = total_tables,
            concurrency = TABLE_CONCURRENCY,
            "spawning table profilers"
        );
        let results = self.profile_tables_parallel(
            &filtered_tables,
            job_id,
            database_id,
            tenant_id,
            discovery.database_context,
            &discovery.fk_edges,
            sample_size,
            tx,
            cancel,
        );

        // Phase 3: Accumulate results and emit completion
        let status_key = self.persistence.status_key(job_id);
        let mut accumulated = self
            .accumulate_results(
                results,
                tenant_id,
                &status_key,
                total_tables as u32,
                total_columns_estimate,
                cancel,
                tx,
            )
            .await;

        accumulated.duration_ms = start.elapsed().as_millis() as u64;
        self.emit_completion(tenant_id, &status_key, accumulated, tx)
            .await;
        tracing::info!(
            job_id,
            duration_ms = start.elapsed().as_millis() as u64,
            "database profiling complete"
        );
    }

    // =========================================================================
    // Private helpers
    // =========================================================================

    async fn discover_schema(
        &self,
        schema: &str,
        tx: &mpsc::Sender<IngestionEvent>,
    ) -> Option<SchemaDiscovery> {
        let introspection = IntrospectionService::new(Arc::clone(&self.db));

        tracing::info!(schema, "discovering schema tables");
        let tables = match introspection.list_tables(schema).await {
            Ok(t) => t,
            Err(e) => {
                let _ = emit(
                    tx,
                    IngestionEvent::Error {
                        message: format!("Failed to list tables: {}", e),
                    },
                )
                .await;
                return None;
            }
        };
        tracing::info!(
            schema,
            tables_found = tables.len(),
            "schema table discovery complete"
        );

        if !emit(
            tx,
            IngestionEvent::Progress {
                status: IngestionStatus::Discovering {
                    tables_found: tables.len() as u32,
                },
            },
        )
        .await
        {
            return None;
        }

        tracing::info!(schema, "discovering foreign keys");
        let fk_edges = match introspection.list_foreign_keys(schema).await {
            Ok(edges) => edges,
            Err(e) => {
                tracing::warn!(schema, error = %e, "FK discovery failed");
                vec![]
            }
        };
        tracing::info!(
            schema,
            foreign_keys = fk_edges.len(),
            "foreign key discovery complete"
        );

        let database_context = Self::build_database_context(&tables, &fk_edges);
        Some(SchemaDiscovery {
            tables,
            database_context,
            fk_edges,
        })
    }

    fn build_database_context(tables: &[TableInfo], fk_edges: &[ForeignKeyEdge]) -> Option<String> {
        if tables.is_empty() {
            return None;
        }

        let table_list: Vec<String> = tables
            .iter()
            .map(|t| {
                let row_info = t
                    .row_count
                    .map(|r| format!(" (~{} rows)", r))
                    .unwrap_or_default();
                let col_info = t
                    .column_count
                    .map(|c| format!(", {} cols", c))
                    .unwrap_or_default();
                format!(
                    "- {}.{}{}{}",
                    t.schema_name, t.table_name, row_info, col_info
                )
            })
            .collect();

        let mut ctx = format!(
            "Database contains {} tables:\n{}",
            tables.len(),
            table_list.join("\n")
        );

        if !fk_edges.is_empty() {
            ctx.push_str("\n\nForeign key relationships:");
            for edge in fk_edges {
                ctx.push_str(&format!(
                    "\n- {}.{} -> {}.{}",
                    edge.source_table, edge.source_column, edge.target_table, edge.target_column
                ));
            }
        }

        Some(ctx)
    }

    fn estimate_total_columns(tables: &[TableInfo]) -> u32 {
        let from_metadata: u32 = tables
            .iter()
            .filter_map(|t| t.column_count)
            .map(|c| c as u32)
            .sum();
        if from_metadata > 0 {
            from_metadata
        } else {
            tables.len() as u32 * 12
        }
    }

    fn filter_requested_tables(
        tables: Vec<TableInfo>,
        selected_tables: Option<&[String]>,
    ) -> Result<Vec<TableInfo>, String> {
        let Some(selected_tables) = selected_tables else {
            return Ok(tables);
        };

        let requested: std::collections::BTreeSet<&str> =
            selected_tables.iter().map(String::as_str).collect();
        let filtered: Vec<_> = tables
            .into_iter()
            .filter(|table| requested.contains(table.table_name.as_str()))
            .collect();
        let found: std::collections::BTreeSet<&str> = filtered
            .iter()
            .map(|table| table.table_name.as_str())
            .collect();
        let missing: Vec<_> = requested
            .difference(&found)
            .copied()
            .map(str::to_string)
            .collect();

        if !missing.is_empty() {
            return Err(format!(
                "requested tables not found in schema: {}",
                missing.join(", ")
            ));
        }

        Ok(filtered)
    }

    fn profile_tables_parallel(
        &self,
        tables: &[TableInfo],
        job_id: &str,
        database_id: &str,
        tenant_id: &str,
        database_context: Option<String>,
        all_fk_edges: &[ForeignKeyEdge],
        sample_size: usize,
        tx: &mpsc::Sender<IngestionEvent>,
        cancel: &CancellationToken,
    ) -> JoinSet<TableProfileResult> {
        let semaphore = Arc::new(Semaphore::new(TABLE_CONCURRENCY));
        let mut join_set: JoinSet<TableProfileResult> = JoinSet::new();

        for (i, table_info) in tables.iter().enumerate() {
            // Filter FK edges relevant to this table (inbound + outbound)
            let table_fk_edges: Vec<ForeignKeyEdge> = all_fk_edges
                .iter()
                .filter(|e| {
                    e.source_table == table_info.table_name
                        || e.target_table == table_info.table_name
                })
                .cloned()
                .collect();

            let span = tracing::info_span!("profile_table", table = %table_info.table_name);
            join_set.spawn(
                spawn_table_profiler(
                    Arc::clone(&semaphore),
                    TableProfilerConfig {
                        sub_job_id: format!("{}-table-{}", job_id, i),
                        root_job_id: job_id.to_string(),
                        table_schema: table_info.schema_name.clone(),
                        table_name: table_info.table_name.clone(),
                        database_id: database_id.to_string(),
                        sample_size,
                        tenant_id: tenant_id.to_string(),
                        db: Arc::clone(&self.db),
                        enricher: Arc::clone(&self.enricher),
                        writer: Arc::clone(&self.writer),
                        kv: Arc::clone(&self.kv),
                        database_context: database_context.clone(),
                        fk_edges: table_fk_edges,
                        persistence: self.persistence.clone(),
                        cancel: cancel.child(),
                        main_tx: tx.clone(),
                    },
                )
                .instrument(span),
            );
        }

        join_set
    }

    async fn accumulate_results(
        &self,
        mut join_set: JoinSet<TableProfileResult>,
        tenant_id: &str,
        status_key: &str,
        total_tables: u32,
        total_columns_estimate: u32,
        cancel: &CancellationToken,
        tx: &mpsc::Sender<IngestionEvent>,
    ) -> IngestionSummary {
        let mut accumulated = IngestionSummary::ZERO;
        let mut completed_tables = 0u32;

        while let Some(result) = join_set.join_next().await {
            if cancel.is_cancelled() {
                let _ = emit(
                    tx,
                    IngestionEvent::Error {
                        message: "Cancelled".to_string(),
                    },
                )
                .await;
                return accumulated;
            }

            match result {
                Ok(TableProfileResult {
                    table_name,
                    summary,
                    error,
                }) => {
                    if let Some(s) = summary {
                        accumulated = accumulated.combine(&s);
                    } else if let Some(ref err) = error {
                        tracing::warn!(table = %table_name, error = %err, "Table profiling failed");
                    } else {
                        tracing::warn!(table = %table_name, "Table profiling failed");
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Table profiling task panicked");
                }
            }
            completed_tables += 1;

            let status = IngestionStatus::Profiling {
                tables_found: total_tables,
                columns_profiled: accumulated.columns_profiled,
                total_columns: total_columns_estimate,
            };
            let _ = self
                .kv
                .put(tenant_id, status_key, &status, Some(EPHEMERAL_TTL))
                .await;
            tracing::info!(
                completed_tables,
                total_tables,
                columns_profiled = accumulated.columns_profiled,
                total_columns = total_columns_estimate,
                "database profiling progress"
            );
        }

        accumulated
    }

    async fn emit_completion(
        &self,
        tenant_id: &str,
        status_key: &str,
        summary: IngestionSummary,
        tx: &mpsc::Sender<IngestionEvent>,
    ) {
        if let Err(e) = self
            .kv
            .put(
                tenant_id,
                status_key,
                &IngestionStatus::Completed {
                    summary: summary.clone(),
                },
                Some(EPHEMERAL_TTL),
            )
            .await
        {
            tracing::warn!(status_key, "Failed to update completion status: {}", e);
        }
        let _ = emit(tx, IngestionEvent::Completed { summary }).await;
    }

    /// Enrich a table with retry, timeout, and cancellation.
    async fn enrich_with_retry_and_timeout(
        &self,
        request: TableEnrichmentRequest,
        cancel: &CancellationToken,
        schema: &str,
        table: &str,
        physical: &PhysicalTable,
    ) -> EnrichmentResult {
        let enricher = Arc::clone(&self.enricher);
        let policy = enrichment_retry_policy();
        let max_attempts = ENRICHMENT_MAX_RETRIES + 1;
        let mut next_attempt = 1u32;

        let result = retry_when_observed(
            &policy,
            || {
                let attempt = next_attempt;
                next_attempt += 1;
                let e = enricher.clone();
                let r = request.clone();
                let cancel = cancel.clone();
                async move {
                    tracing::info!(
                        schema,
                        table,
                        attempt,
                        max_attempts,
                        timeout_ms = ENRICHMENT_TIMEOUT.as_millis() as u64,
                        "starting enrichment attempt"
                    );
                    // Combine timeout + cancellation
                    let enrichment_fut = e.enrich_table(r);
                    let timed = with_timeout(ENRICHMENT_TIMEOUT, enrichment_fut).await;
                    match timed {
                        Ok(Ok(result)) => {
                            if cancel.is_cancelled() {
                                Err(EnrichmentError::Cancelled)
                            } else {
                                Ok(result)
                            }
                        }
                        Ok(Err(e)) => Err(e),
                        Err(_) => Err(EnrichmentError::Timeout {
                            duration_ms: ENRICHMENT_TIMEOUT.as_millis() as u64,
                        }),
                    }
                }
            },
            |e: &EnrichmentError| e.is_retryable(),
            |ctx| {
                tracing::warn!(
                    schema,
                    table,
                    attempt = ctx.attempt + 1,
                    max_attempts,
                    retry_delay_ms = ctx.delay.as_millis() as u64,
                    elapsed_ms = ctx.elapsed.as_millis() as u64,
                    error = %ctx.last_error,
                    "enrichment attempt failed; retrying"
                );
            },
        )
        .await;

        match result {
            Ok(r) => {
                tracing::info!(
                    schema,
                    table,
                    source = ?r.source,
                    "enrichment succeeded"
                );
                r
            }
            Err(e) => {
                tracing::warn!(schema, table, error = %e, "Enrichment failed, using fallback");
                EnrichmentResult::fallback(builder::fallback_semantic_profile(
                    physical, schema, table,
                ))
                .with_fallback_reason(format!("enrichment failed: {e}"))
            }
        }
    }
}

// =============================================================================
// Spawned table profiler
// =============================================================================

/// Profile a single table in a spawned task with semaphore-bounded concurrency.
///
/// Event filtering law: only forward events that carry table provenance.
/// Anonymous events (Started, Progress, Completed, Error) are absorbed.
async fn spawn_table_profiler(
    sem: Arc<Semaphore>,
    config: TableProfilerConfig,
) -> TableProfileResult {
    let TableProfilerConfig {
        sub_job_id,
        root_job_id,
        table_schema,
        table_name,
        database_id,
        sample_size,
        tenant_id,
        db,
        enricher,
        writer,
        kv,
        database_context,
        fk_edges,
        persistence,
        cancel: child_cancel,
        main_tx,
    } = config;

    let _permit = match sem.acquire_owned().await {
        Ok(p) => p,
        Err(_) => {
            let _ = main_tx
                .send(IngestionEvent::TableFailed {
                    table_name: table_name.clone(),
                    error: "Concurrency semaphore closed".to_string(),
                })
                .await;
            return TableProfileResult {
                table_name,
                summary: None,
                error: Some("Concurrency semaphore closed".to_string()),
            };
        }
    };

    if child_cancel.is_cancelled() {
        let _ = main_tx
            .send(IngestionEvent::TableFailed {
                table_name: table_name.clone(),
                error: "Cancelled".to_string(),
            })
            .await;
        return TableProfileResult {
            table_name,
            summary: None,
            error: Some("Cancelled".to_string()),
        };
    }

    tracing::info!(
        schema = %table_schema,
        table = %table_name,
        "table profiler slot acquired"
    );

    let orchestrator = IngestionOrchestrator {
        db,
        enricher,
        writer,
        kv,
        persistence,
    };
    let (table_tx, mut table_rx) = mpsc::channel::<IngestionEvent>(256);

    let worker_cancel = child_cancel.clone();
    let worker_table_name = table_name.clone();
    let worker = tokio::spawn(async move {
        orchestrator
            .profile_single_table(ProfileTableParams {
                tenant_id: &tenant_id,
                job_id: &sub_job_id,
                profiling_run_id: Some(&root_job_id),
                schema: &table_schema,
                table: &worker_table_name,
                database_id: &database_id,
                sample_size,
                tx: &table_tx,
                cancel: &worker_cancel,
                database_context: database_context.as_deref(),
                fk_edges: &fk_edges,
            })
            .await;
    });

    // Forwarder: drain table_rx → main_tx.
    let mut summary: Option<IngestionSummary> = None;
    let mut error: Option<String> = None;
    let mut client_gone = false;
    while let Some(event) = table_rx.recv().await {
        match event {
            // Absorb: no table provenance
            IngestionEvent::Started { .. } | IngestionEvent::Progress { .. } => {}
            // Absorb: extract summary
            IngestionEvent::Completed { summary: s } => {
                summary = Some(s);
            }
            // Absorb: extract error
            IngestionEvent::Error { message } => {
                error = Some(message);
            }
            // Forward: carries table_name
            named_event => {
                if !client_gone && main_tx.send(named_event).await.is_err() {
                    client_gone = true;
                    child_cancel.cancel();
                }
            }
        }
    }

    if let Err(e) = worker.await {
        tracing::error!(table = %table_name, error = %e, "Table profiling worker panicked");
        if error.is_none() {
            error = Some(format!("Worker panicked: {}", e));
        }
    }

    // Emit TableCompleted or TableFailed
    if !client_gone {
        match &summary {
            Some(s) => {
                let _ = main_tx
                    .send(IngestionEvent::TableCompleted {
                        table_name: table_name.clone(),
                        summary: s.clone(),
                    })
                    .await;
            }
            None => {
                let err_msg = error.as_deref().unwrap_or("Unknown error");
                let _ = main_tx
                    .send(IngestionEvent::TableFailed {
                        table_name: table_name.clone(),
                        error: err_msg.to_string(),
                    })
                    .await;
            }
        }
    }

    TableProfileResult {
        table_name,
        summary,
        error,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_catalog::{
        CatalogEntry, CatalogError, ColumnDescriptions, DataQualityFindingMetadata,
        EnrichmentResult, EnrichmentSource, ForeignKeyEdge, IngestionEvent,
        KnowledgeExtractionRequest, KnowledgeItem, SemanticTableProfile, TableEnrichmentRequest,
        TableMetadata,
    };
    use agent_fw_interpreter::{
        DashMapKVStore, MockEnricher, MockTargetDatabase, SqliteTargetDatabase,
    };

    const TEST_TABLE: &str = "test_table";
    const TEST_SCHEMA: &str = "public";
    const TEST_DB_ID: &str = "test-db";
    const TEST_TENANT: &str = "test-tenant";

    // ── Protocol checker ────────────────────────────────────────────

    fn assert_protocol(events: &[IngestionEvent], table: &str, expect_success: bool) {
        assert!(!events.is_empty(), "Protocol violation: zero events");

        // L1: first event is Started
        assert!(
            matches!(&events[0], IngestionEvent::Started { .. }),
            "L1 violated: first event is {:?}, expected Started",
            events[0]
        );

        let len = events.len();

        if expect_success {
            assert!(
                len >= 3,
                "L2 violated: need >=3 events for success, got {}",
                len
            );
            assert!(
                matches!(
                    &events[len - 2],
                    IngestionEvent::TableCompleted { table_name, .. } if table_name == table
                ),
                "L2 violated: second-to-last event is {:?}, expected TableCompleted({})",
                events[len - 2],
                table
            );
            assert!(
                matches!(&events[len - 1], IngestionEvent::Completed { .. }),
                "L2 violated: last event is {:?}, expected Completed",
                events[len - 1]
            );
        } else {
            assert!(
                len >= 3,
                "L3 violated: need >=3 events for failure, got {}",
                len
            );
            assert!(
                matches!(
                    &events[len - 2],
                    IngestionEvent::TableFailed { table_name, .. } if table_name == table
                ),
                "L3 violated: second-to-last event is {:?}, expected TableFailed({})",
                events[len - 2],
                table
            );
            assert!(
                matches!(&events[len - 1], IngestionEvent::Error { .. }),
                "L3 violated: last event is {:?}, expected Error",
                events[len - 1]
            );
        }

        // L4: exclusion
        let has_completed = events
            .iter()
            .any(|e| matches!(e, IngestionEvent::TableCompleted { .. }));
        let has_failed = events
            .iter()
            .any(|e| matches!(e, IngestionEvent::TableFailed { .. }));
        assert!(
            !(has_completed && has_failed),
            "L4 violated: both TableCompleted and TableFailed present"
        );
    }

    // ── KVCatalogWriter (test impl) ────────────────────────────────

    struct KVCatalogWriter {
        kv: Arc<dyn KVStore>,
        tenant: String,
    }

    impl KVCatalogWriter {
        fn new(kv: Arc<dyn KVStore>, tenant: &str) -> Self {
            Self {
                kv,
                tenant: tenant.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl CatalogWriter for KVCatalogWriter {
        async fn save_items(&self, items: Vec<CatalogEntry>) -> Result<Vec<String>, CatalogError> {
            let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
            for item in &items {
                let key = format!("catalog:{}", item.id);
                self.kv
                    .put(&self.tenant, &key, item, Some(EPHEMERAL_TTL))
                    .await
                    .map_err(|e| CatalogError::Unavailable(format!("{}", e)))?;
            }
            Ok(ids)
        }

        async fn delete_items(&self, _ids: &[String]) -> Result<u32, CatalogError> {
            Ok(0)
        }

        async fn save_in_transaction(
            &self,
            items: Vec<CatalogEntry>,
        ) -> Result<Vec<String>, CatalogError> {
            self.save_items(items).await
        }
    }

    // ── FailingCatalogWriter ────────────────────────────────────────

    struct FailingCatalogWriter;

    #[async_trait::async_trait]
    impl CatalogWriter for FailingCatalogWriter {
        async fn save_items(&self, _items: Vec<CatalogEntry>) -> Result<Vec<String>, CatalogError> {
            Err(CatalogError::Unavailable("test: always fails".into()))
        }

        async fn delete_items(&self, _ids: &[String]) -> Result<u32, CatalogError> {
            Err(CatalogError::Unavailable("test: always fails".into()))
        }

        async fn save_in_transaction(
            &self,
            _items: Vec<CatalogEntry>,
        ) -> Result<Vec<String>, CatalogError> {
            Err(CatalogError::Unavailable("test: always fails".into()))
        }
    }

    // ── Test infrastructure ─────────────────────────────────────────

    fn make_orchestrator(
        db: Arc<dyn TargetDatabase>,
        writer: Arc<dyn CatalogWriter>,
    ) -> IngestionOrchestrator {
        let enricher: Arc<dyn SemanticEnricher> = Arc::new(MockEnricher::new());
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        IngestionOrchestrator::new(db, enricher, writer, kv)
    }

    fn make_orchestrator_with_enricher(
        db: Arc<dyn TargetDatabase>,
        writer: Arc<dyn CatalogWriter>,
        enricher: Arc<dyn SemanticEnricher>,
    ) -> IngestionOrchestrator {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        IngestionOrchestrator::new(db, enricher, writer, kv)
    }

    struct ModelIdEnricher;

    #[async_trait::async_trait]
    impl SemanticEnricher for ModelIdEnricher {
        async fn enrich_table(
            &self,
            request: TableEnrichmentRequest,
        ) -> Result<EnrichmentResult, EnrichmentError> {
            let table_name = request.table.table_name;
            let mut col_descs = ColumnDescriptions::new();
            for col in request.table.columns {
                col_descs.insert(
                    col.column_name.clone(),
                    format!("{} description", col.column_name),
                );
            }

            Ok(EnrichmentResult {
                profile: SemanticTableProfile {
                    description: format!("{table_name} table"),
                    short_description: table_name,
                    column_descriptions: col_descs,
                    relationships: vec![],
                    quality_notes: vec![],
                },
                source: EnrichmentSource::Fresh,
                model_id: Some("claude-test-model".to_string()),
                fallback_reason: None,
            })
        }

        async fn extract_knowledge(
            &self,
            _request: KnowledgeExtractionRequest,
        ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
            Ok(vec![])
        }
    }

    struct FailingEnricher;

    #[async_trait::async_trait]
    impl SemanticEnricher for FailingEnricher {
        async fn enrich_table(
            &self,
            _request: TableEnrichmentRequest,
        ) -> Result<EnrichmentResult, EnrichmentError> {
            Err(EnrichmentError::LlmFailed(
                "provider unavailable".to_string(),
            ))
        }

        async fn extract_knowledge(
            &self,
            _request: KnowledgeExtractionRequest,
        ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
            Ok(vec![])
        }
    }

    struct QualityNoteEnricher;

    #[async_trait::async_trait]
    impl SemanticEnricher for QualityNoteEnricher {
        async fn enrich_table(
            &self,
            request: TableEnrichmentRequest,
        ) -> Result<EnrichmentResult, EnrichmentError> {
            let mut col_descs = ColumnDescriptions::new();
            for col in request.table.columns {
                col_descs.insert(
                    col.column_name.clone(),
                    format!("{} description", col.column_name),
                );
            }

            Ok(EnrichmentResult::fresh(SemanticTableProfile {
                description: format!("{} table", request.table.table_name),
                short_description: request.table.table_name,
                column_descriptions: col_descs,
                relationships: vec![],
                quality_notes: vec![QualityNote {
                    column_name: "*".to_string(),
                    notes: "Rows with null identifiers should be investigated".to_string(),
                    typical_value_range: Some("non-null IDs".to_string()),
                    validation_rules: vec!["id IS NOT NULL".to_string()],
                }],
            }))
        }

        async fn extract_knowledge(
            &self,
            _request: KnowledgeExtractionRequest,
        ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
            Ok(vec![])
        }
    }

    #[derive(Clone, Default)]
    struct CapturingFkEnricher {
        requests: Arc<tokio::sync::Mutex<Vec<(String, Vec<ForeignKeyEdge>)>>>,
    }

    #[async_trait::async_trait]
    impl SemanticEnricher for CapturingFkEnricher {
        async fn enrich_table(
            &self,
            request: TableEnrichmentRequest,
        ) -> Result<EnrichmentResult, EnrichmentError> {
            self.requests
                .lock()
                .await
                .push((request.table.table_name.clone(), request.fk_edges.clone()));

            let mut col_descs = ColumnDescriptions::new();
            for col in request.table.columns {
                col_descs.insert(
                    col.column_name.clone(),
                    format!("{} description", col.column_name),
                );
            }

            Ok(EnrichmentResult::fresh(SemanticTableProfile {
                description: format!("{} table", request.table.table_name),
                short_description: request.table.table_name,
                column_descriptions: col_descs,
                relationships: vec![],
                quality_notes: vec![],
            }))
        }

        async fn extract_knowledge(
            &self,
            _request: KnowledgeExtractionRequest,
        ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
            Ok(vec![])
        }
    }

    async fn run_and_collect(
        orch: &IngestionOrchestrator,
        cancel: &CancellationToken,
    ) -> Vec<IngestionEvent> {
        let (tx, mut rx) = mpsc::channel(256);

        orch.profile_single_table(ProfileTableParams {
            tenant_id: TEST_TENANT,
            job_id: "test-job",
            profiling_run_id: None,
            schema: TEST_SCHEMA,
            table: TEST_TABLE,
            database_id: TEST_DB_ID,
            sample_size: 10,
            tx: &tx,
            cancel,
            database_context: None,
            fk_edges: &[],
        })
        .await;

        drop(tx);

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        events
    }

    // ── Tests ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn happy_path_satisfies_protocol() {
        let db: Arc<dyn TargetDatabase> = Arc::new(MockTargetDatabase::new());
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let writer: Arc<dyn CatalogWriter> =
            Arc::new(KVCatalogWriter::new(Arc::clone(&kv), TEST_TENANT));
        let orch = make_orchestrator(Arc::clone(&db), writer);
        let cancel = CancellationToken::new();

        let events = run_and_collect(&orch, &cancel).await;
        assert_protocol(&events, TEST_TABLE, true);

        // Interior events present
        assert!(events
            .iter()
            .any(|e| matches!(e, IngestionEvent::TableProfiled { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, IngestionEvent::TableEnriched { .. })));
    }

    #[tokio::test]
    async fn custom_persistence_policy_writes_status_under_configured_prefix() {
        let db: Arc<dyn TargetDatabase> = Arc::new(MockTargetDatabase::new());
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let writer: Arc<dyn CatalogWriter> =
            Arc::new(KVCatalogWriter::new(Arc::clone(&kv), TEST_TENANT));
        let enricher: Arc<dyn SemanticEnricher> = Arc::new(MockEnricher::new());
        let orch = IngestionOrchestrator::new_with_persistence(
            Arc::clone(&db),
            enricher,
            writer,
            Arc::clone(&kv),
            IngestionPersistencePolicy::default().with_status_key_prefix("data:ingestion:"),
        );
        let cancel = CancellationToken::new();

        let events = run_and_collect(&orch, &cancel).await;
        assert_protocol(&events, TEST_TABLE, true);

        let status: Option<IngestionStatus> = kv
            .get(TEST_TENANT, "data:ingestion:test-job")
            .await
            .expect("status lookup should succeed");
        assert!(matches!(status, Some(IngestionStatus::Completed { .. })));
        assert!(!kv
            .exists(TEST_TENANT, "ingest:status:test-job")
            .await
            .expect("default status-key lookup should succeed"));
    }

    #[tokio::test]
    async fn profile_single_table_persists_table_provenance() {
        let db: Arc<dyn TargetDatabase> = Arc::new(MockTargetDatabase::new());
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let writer: Arc<dyn CatalogWriter> =
            Arc::new(KVCatalogWriter::new(Arc::clone(&kv), TEST_TENANT));
        let orch = make_orchestrator(Arc::clone(&db), writer);
        let cancel = CancellationToken::new();

        let events = run_and_collect(&orch, &cancel).await;
        assert_protocol(&events, TEST_TABLE, true);

        let table_id = builder::generate_catalog_id(
            agent_fw_catalog::CatalogKind::Table,
            TEST_DB_ID,
            &[TEST_SCHEMA, TEST_TABLE],
        );
        let entry: CatalogEntry = kv
            .get(TEST_TENANT, &format!("catalog:{table_id}"))
            .await
            .expect("catalog lookup should succeed")
            .expect("table entry should be persisted");
        let metadata: TableMetadata =
            serde_json::from_value(entry.metadata).expect("table metadata should decode");

        assert_eq!(
            metadata.source.profiling_run_id.as_deref(),
            Some("test-job")
        );
        assert_eq!(metadata.source.enrichment_source.as_deref(), Some("fresh"));
        assert_eq!(metadata.source.model_id, None);
        assert_eq!(metadata.source.schema_snapshot_at, None);
        assert_eq!(metadata.source.target_fingerprint, None);
    }

    #[tokio::test]
    async fn profile_single_table_persists_enrichment_model_id_when_reported() {
        let db: Arc<dyn TargetDatabase> = Arc::new(MockTargetDatabase::new());
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let writer: Arc<dyn CatalogWriter> =
            Arc::new(KVCatalogWriter::new(Arc::clone(&kv), TEST_TENANT));
        let enricher: Arc<dyn SemanticEnricher> = Arc::new(ModelIdEnricher);
        let orch = make_orchestrator_with_enricher(Arc::clone(&db), writer, enricher);
        let cancel = CancellationToken::new();

        let events = run_and_collect(&orch, &cancel).await;
        assert_protocol(&events, TEST_TABLE, true);

        let table_id = builder::generate_catalog_id(
            agent_fw_catalog::CatalogKind::Table,
            TEST_DB_ID,
            &[TEST_SCHEMA, TEST_TABLE],
        );
        let entry: CatalogEntry = kv
            .get(TEST_TENANT, &format!("catalog:{table_id}"))
            .await
            .expect("catalog lookup should succeed")
            .expect("table entry should be persisted");
        let metadata: TableMetadata =
            serde_json::from_value(entry.metadata).expect("table metadata should decode");

        assert_eq!(metadata.source.enrichment_source.as_deref(), Some("fresh"));
        assert_eq!(
            metadata.source.model_id.as_deref(),
            Some("claude-test-model")
        );
    }

    #[tokio::test]
    async fn profile_database_persists_root_job_id_as_table_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("target.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT);
                 CREATE TABLE orders (
                     id INTEGER PRIMARY KEY,
                     product_id INTEGER,
                     status TEXT,
                     FOREIGN KEY(product_id) REFERENCES products(id)
                 );
                 INSERT INTO products (name) VALUES ('water');
                 INSERT INTO orders (product_id, status) VALUES (1, 'confirmed');",
            )
            .unwrap();
        }

        let db: Arc<dyn TargetDatabase> = Arc::new(SqliteTargetDatabase::open(&db_path).unwrap());
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let writer: Arc<dyn CatalogWriter> =
            Arc::new(KVCatalogWriter::new(Arc::clone(&kv), TEST_TENANT));
        let enricher: Arc<dyn SemanticEnricher> = Arc::new(MockEnricher::fallback());
        let orch = IngestionOrchestrator::new(db, enricher, writer, Arc::clone(&kv));
        let cancel = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel(256);

        orch.profile_database_with_params(ProfileDatabaseParams {
            tenant_id: TEST_TENANT,
            job_id: "root-profile-job",
            schema: "main",
            database_id: TEST_DB_ID,
            tx: &tx,
            cancel: &cancel,
            selected_tables: None,
            sample_size: 10,
        })
        .await;
        drop(tx);
        while rx.recv().await.is_some() {}

        let table_id = builder::generate_catalog_id(
            agent_fw_catalog::CatalogKind::Table,
            TEST_DB_ID,
            &["main", "orders"],
        );
        let entry: CatalogEntry = kv
            .get(TEST_TENANT, &format!("catalog:{table_id}"))
            .await
            .expect("catalog lookup should succeed")
            .expect("orders table entry should be persisted");
        let metadata_json = entry.metadata.clone();
        let metadata: TableMetadata =
            serde_json::from_value(entry.metadata).expect("table metadata should decode");

        assert_eq!(
            metadata.source.profiling_run_id.as_deref(),
            Some("root-profile-job")
        );
        assert_eq!(
            metadata.source.enrichment_source.as_deref(),
            Some("fallback")
        );
        assert_eq!(
            metadata_json
                .pointer("/source/fallbackReason")
                .and_then(|value| value.as_str()),
            Some("enricher reported fallback")
        );
    }

    #[tokio::test]
    async fn profile_database_passes_table_fk_edges_to_enricher() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("target.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT);
                 CREATE TABLE orders (
                     id INTEGER PRIMARY KEY,
                     product_id INTEGER,
                     status TEXT,
                     FOREIGN KEY(product_id) REFERENCES products(id)
                 );
                 INSERT INTO products (name) VALUES ('water');
                 INSERT INTO orders (product_id, status) VALUES (1, 'confirmed');",
            )
            .unwrap();
        }

        let db: Arc<dyn TargetDatabase> = Arc::new(SqliteTargetDatabase::open(&db_path).unwrap());
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let writer: Arc<dyn CatalogWriter> =
            Arc::new(KVCatalogWriter::new(Arc::clone(&kv), TEST_TENANT));
        let enricher = CapturingFkEnricher::default();
        let captured = Arc::clone(&enricher.requests);
        let orch = IngestionOrchestrator::new(db, Arc::new(enricher), writer, Arc::clone(&kv));
        let cancel = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel(256);

        orch.profile_database_with_params(ProfileDatabaseParams {
            tenant_id: TEST_TENANT,
            job_id: "fk-profile-job",
            schema: "main",
            database_id: TEST_DB_ID,
            tx: &tx,
            cancel: &cancel,
            selected_tables: None,
            sample_size: 10,
        })
        .await;
        drop(tx);
        while rx.recv().await.is_some() {}

        let requests = captured.lock().await.clone();
        let expected = ForeignKeyEdge {
            source_table: "orders".to_string(),
            source_column: "product_id".to_string(),
            target_table: "products".to_string(),
            target_column: "id".to_string(),
        };

        let orders_edges = requests
            .iter()
            .find(|(table, _)| table == "orders")
            .map(|(_, edges)| edges)
            .expect("orders table should be enriched");
        assert_eq!(orders_edges, &vec![expected.clone()]);

        let products_edges = requests
            .iter()
            .find(|(table, _)| table == "products")
            .map(|(_, edges)| edges)
            .expect("products table should be enriched");
        assert_eq!(products_edges, &vec![expected]);
    }

    #[tokio::test]
    async fn profile_single_table_persists_enrichment_failure_reason_on_fallback() {
        let db: Arc<dyn TargetDatabase> = Arc::new(MockTargetDatabase::new());
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let writer: Arc<dyn CatalogWriter> =
            Arc::new(KVCatalogWriter::new(Arc::clone(&kv), TEST_TENANT));
        let enricher: Arc<dyn SemanticEnricher> = Arc::new(FailingEnricher);
        let orch = make_orchestrator_with_enricher(Arc::clone(&db), writer, enricher);
        let cancel = CancellationToken::new();

        let events = run_and_collect(&orch, &cancel).await;
        assert_protocol(&events, TEST_TABLE, true);
        assert!(events.iter().any(|event| {
            matches!(
                event,
                IngestionEvent::TableEnriched {
                    source: EnrichmentSource::Fallback,
                    fallback_reason: Some(reason),
                    ..
                } if reason.contains("provider unavailable")
            )
        }));

        let table_id = builder::generate_catalog_id(
            agent_fw_catalog::CatalogKind::Table,
            TEST_DB_ID,
            &[TEST_SCHEMA, TEST_TABLE],
        );
        let entry: CatalogEntry = kv
            .get(TEST_TENANT, &format!("catalog:{table_id}"))
            .await
            .expect("catalog lookup should succeed")
            .expect("table entry should be persisted");
        let metadata_json = entry.metadata;

        assert_eq!(
            metadata_json
                .pointer("/source/enrichmentSource")
                .and_then(|value| value.as_str()),
            Some("fallback")
        );
        let reason = metadata_json
            .pointer("/source/fallbackReason")
            .and_then(|value| value.as_str())
            .expect("fallback reason should be persisted");
        assert!(reason.contains("provider unavailable"), "{reason}");
    }

    #[tokio::test]
    async fn profile_single_table_persists_quality_notes_as_data_quality_findings() {
        let db: Arc<dyn TargetDatabase> = Arc::new(MockTargetDatabase::new());
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let writer: Arc<dyn CatalogWriter> =
            Arc::new(KVCatalogWriter::new(Arc::clone(&kv), TEST_TENANT));
        let enricher: Arc<dyn SemanticEnricher> = Arc::new(QualityNoteEnricher);
        let orch = make_orchestrator_with_enricher(Arc::clone(&db), writer, enricher);
        let cancel = CancellationToken::new();

        let events = run_and_collect(&orch, &cancel).await;
        assert_protocol(&events, TEST_TABLE, true);

        let keys = kv
            .list_keys(TEST_TENANT, "catalog:")
            .await
            .expect("catalog keys should be listed");
        let mut quality_finding_entries = Vec::new();
        for key in keys {
            let Some(entry) = kv
                .get::<CatalogEntry>(TEST_TENANT, &key)
                .await
                .expect("catalog entry should decode")
            else {
                continue;
            };
            if entry.kind == agent_fw_catalog::CatalogKind::DataQualityFinding
                && entry.content.contains("Rows with null identifiers")
            {
                quality_finding_entries.push(entry);
            }
        }

        assert_eq!(quality_finding_entries.len(), 1);
        let entry = quality_finding_entries.pop().unwrap();
        let metadata: DataQualityFindingMetadata =
            serde_json::from_value(entry.metadata).expect("finding metadata should decode");
        assert_eq!(metadata.finding_type.as_deref(), Some("data_quality"));
        assert_eq!(
            metadata.scope_tables,
            vec![format!("{TEST_SCHEMA}.{TEST_TABLE}")]
        );
        assert!(metadata.scope_columns.is_empty());
        assert!(entry
            .links
            .iter()
            .any(|link| link.kind
                == agent_fw_catalog::relation_kind::DATA_QUALITY_FINDING_APPLIES_TO));
        assert!(entry.content.contains("id IS NOT NULL"));
        assert!(entry.content.contains("non-null IDs"));
    }

    #[tokio::test]
    async fn save_failure_satisfies_protocol() {
        let db: Arc<dyn TargetDatabase> = Arc::new(MockTargetDatabase::new());
        let writer: Arc<dyn CatalogWriter> = Arc::new(FailingCatalogWriter);
        let orch = make_orchestrator(db, writer);
        let cancel = CancellationToken::new();

        let events = run_and_collect(&orch, &cancel).await;
        assert_protocol(&events, TEST_TABLE, false);

        assert!(events
            .iter()
            .any(|e| matches!(e, IngestionEvent::TableProfiled { .. })));
    }

    #[tokio::test]
    async fn cancellation_emits_error() {
        let db: Arc<dyn TargetDatabase> = Arc::new(MockTargetDatabase::new());
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let writer: Arc<dyn CatalogWriter> =
            Arc::new(KVCatalogWriter::new(Arc::clone(&kv), TEST_TENANT));
        let orch = make_orchestrator(Arc::clone(&db), writer);
        let cancel = CancellationToken::new();
        cancel.cancel();

        let events = run_and_collect(&orch, &cancel).await;

        assert!(matches!(&events[0], IngestionEvent::Started { .. }));

        let last = events.last().expect("should have events");
        assert!(
            matches!(last, IngestionEvent::Error { message } if message == "Cancelled"),
            "Terminal event should be Error{{Cancelled}}, got {:?}",
            last
        );
    }
}
