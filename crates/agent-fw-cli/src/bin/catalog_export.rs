use std::path::PathBuf;
use std::sync::Arc;

use agent_fw_algebra::{CancellationToken, TargetDatabase};
use agent_fw_catalog::{
    EnrichmentError, EnrichmentResult, IngestionEvent, IngestionSummary,
    KnowledgeExtractionRequest, KnowledgeItem, SemanticEnricher, TableEnrichmentRequest,
};
use agent_fw_cli::catalog_export::{write_catalog_entries_json, MemoryCatalogWriter};
use agent_fw_ingest::{builder, ingestion::IngestionOrchestrator};
use agent_fw_interpreter::{AnthropicEnricher, DashMapKVStore, SqlxTargetDatabase};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use clap::Parser;
use tokio::sync::mpsc;

#[derive(Parser, Debug)]
#[command(
    name = "catalog-export",
    about = "Export framework-generated catalog entries from a target database"
)]
struct Args {
    #[arg(long)]
    target_url: String,

    #[arg(long, default_value = "public")]
    schema: String,

    #[arg(long)]
    database_id: String,

    #[arg(long)]
    out: PathBuf,

    #[arg(long, default_value = "catalog-export")]
    tenant_id: String,

    #[arg(long, default_value = "catalog-export")]
    job_id: String,

    #[arg(long)]
    quiet: bool,

    /// Use deterministic schema-only enrichment instead of Anthropic.
    #[arg(long)]
    schema_only: bool,

    /// Anthropic API key. Defaults to ANTHROPIC_API_KEY.
    #[arg(long)]
    anthropic_api_key: Option<String>,

    /// Anthropic model for table enrichment.
    #[arg(long)]
    anthropic_model: Option<String>,

    /// Anthropic-compatible base URL.
    #[arg(long)]
    anthropic_base_url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "catalog_export=info,agent_fw_ingest=warn".into()),
        )
        .with_target(false)
        .init();

    let args = Args::parse();
    run(args).await
}

async fn run(args: Args) -> Result<()> {
    let db = SqlxTargetDatabase::connect(&args.target_url)
        .await
        .with_context(|| "failed to connect to target database")?
        .with_schema(args.schema.clone());
    db.health_check()
        .await
        .with_context(|| "target database health check failed")?;

    let db = Arc::new(db);
    let writer = Arc::new(MemoryCatalogWriter::default());
    let schema_only = args.schema_only;
    let enricher = build_enricher(&args)?;
    let orchestrator = IngestionOrchestrator::new(
        db,
        enricher,
        writer.clone(),
        Arc::new(DashMapKVStore::new()),
    );

    let (tx, rx) = mpsc::channel(1024);
    let event_task = tokio::spawn(drain_events(rx, args.quiet, schema_only));
    let cancel = CancellationToken::new();

    orchestrator
        .profile_database(
            &args.tenant_id,
            &args.job_id,
            &args.schema,
            &args.database_id,
            &tx,
            &cancel,
        )
        .await;
    drop(tx);

    let report = event_task
        .await
        .map_err(|e| anyhow!("catalog export event task failed: {e}"))?;
    if !report.errors.is_empty() {
        bail!("catalog export failed: {}", report.errors.join("; "));
    }

    let written_count = write_catalog_entries_json(&writer, &args.out)
        .await
        .with_context(|| format!("failed to write {}", args.out.display()))?;
    if written_count == 0 {
        bail!("catalog export produced no artifact-safe entries");
    }

    let summary = report.summary.unwrap_or(IngestionSummary::ZERO);
    if !args.quiet {
        eprintln!(
            "wrote {} catalog entries to {} (tables={}, columns={}, enums={})",
            written_count,
            args.out.display(),
            summary.tables_discovered,
            summary.columns_profiled,
            summary.enums_extracted,
        );
    }

    Ok(())
}

fn build_enricher(args: &Args) -> Result<Arc<dyn SemanticEnricher>> {
    if args.schema_only {
        return Ok(Arc::new(SchemaOnlyEnricher));
    }

    let api_key = args
        .anthropic_api_key
        .clone()
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .ok_or_else(|| {
            anyhow!(
                "ANTHROPIC_API_KEY is required for LLM enrichment; pass --schema-only for deterministic fallback"
            )
        })?;

    let mut enricher = AnthropicEnricher::new(api_key);
    if let Some(model) = args
        .anthropic_model
        .clone()
        .or_else(|| std::env::var("CATALOG_EXPORT_ANTHROPIC_MODEL").ok())
    {
        enricher = enricher.with_model(model);
    }
    if let Some(base_url) = args
        .anthropic_base_url
        .clone()
        .or_else(|| std::env::var("ANTHROPIC_BASE_URL").ok())
    {
        enricher = enricher.with_base_url(base_url);
    }

    Ok(Arc::new(enricher))
}

#[derive(Debug, Default)]
struct ExportReport {
    summary: Option<IngestionSummary>,
    errors: Vec<String>,
}

async fn drain_events(
    mut rx: mpsc::Receiver<IngestionEvent>,
    quiet: bool,
    allow_fallback: bool,
) -> ExportReport {
    let mut report = ExportReport::default();
    while let Some(event) = rx.recv().await {
        match event {
            IngestionEvent::Started { job_id } => {
                if !quiet {
                    eprintln!("started catalog export job {job_id}");
                }
            }
            IngestionEvent::TableProfiled {
                table_name,
                columns,
                ..
            } => {
                if !quiet {
                    eprintln!("profiled {table_name} ({columns} columns)");
                }
            }
            IngestionEvent::TableEnriched {
                table_name,
                source,
                fallback_reason,
            } => {
                if !quiet {
                    if let Some(reason) = fallback_reason.as_deref() {
                        eprintln!("enriched {table_name} ({source:?}: {reason})");
                    } else {
                        eprintln!("enriched {table_name} ({source:?})");
                    }
                }
                if !allow_fallback && source == agent_fw_catalog::EnrichmentSource::Fallback {
                    let reason = fallback_reason
                        .as_deref()
                        .unwrap_or("fallback reason unavailable");
                    report.errors.push(format!(
                        "{table_name}: LLM enrichment fell back to schema-only output ({reason})"
                    ));
                }
            }
            IngestionEvent::TableCompleted {
                table_name,
                summary,
            } => {
                if !quiet {
                    eprintln!(
                        "indexed {table_name} ({} entries)",
                        summary.catalog_items_indexed
                    );
                }
            }
            IngestionEvent::TableFailed { table_name, error } => {
                report.errors.push(format!("{table_name}: {error}"));
            }
            IngestionEvent::Completed { summary } => {
                report.summary = Some(summary);
            }
            IngestionEvent::Error { message } => {
                report.errors.push(message);
            }
            IngestionEvent::Progress { .. } => {}
        }
    }
    report
}

struct SchemaOnlyEnricher;

#[async_trait]
impl SemanticEnricher for SchemaOnlyEnricher {
    async fn enrich_table(
        &self,
        request: TableEnrichmentRequest,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        Ok(EnrichmentResult::fallback(
            builder::fallback_semantic_profile(
                &request.table,
                &request.table.schema_name,
                &request.table.table_name,
            ),
        ))
    }

    async fn extract_knowledge(
        &self,
        _request: KnowledgeExtractionRequest,
    ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
        Ok(vec![])
    }
}
