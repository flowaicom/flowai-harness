//! Canonical CLI runner for Flow AI harness data commands.
//!
//! This crate is intentionally thin. It owns command parsing, output rendering,
//! and host-side dependency resolution, while delegating actual profiling work
//! to `flowai-runtime::data`.

mod catalog_graph;
mod mcp;

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_fw_catalog::{
    CatalogRelationDiagnostic, CatalogRelationIssue, CatalogScope, EnrichmentError,
    EnrichmentResult, IngestionEvent, IngestionSummary, KnowledgeExtractionRequest, KnowledgeItem,
    SemanticEnricher, TableEnrichmentRequest,
};
use agent_fw_core::{TenantId, WorkspaceId};
use agent_fw_ingest::builder;
use agent_fw_interpreter::AnthropicEnricher;
use async_trait::async_trait;
use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand, ValueEnum};
use flowai_runtime::data::{
    estimate_profiling, export_catalog, ingest_knowledge, profile_database, profile_table,
    ExportCatalogCommand, IngestKnowledgeCommand, KnowledgeCommandDeps, KnowledgeIngestEvent,
    KnowledgeSourceSpec, ProfileDatabaseCommand, ProfileTableCommand, ProfilingCommandDeps,
    ProfilingEstimateCommand,
};
use flowai_runtime::storage::{
    build_catalog_for_scope, doctor_catalog_search_index_from_environment,
    rebuild_catalog_search_index_from_environment, DataEnvironmentConfig,
};
use thiserror::Error;

const DEFAULT_DATA_TENANT_ID: &str = flowai_runtime::data::DEFAULT_DATA_TENANT_ID;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("{0}")]
    Parse(String),
    #[error("failed to read data-environment file '{path}': {source}")]
    ReadConfig { path: PathBuf, source: io::Error },
    #[error("invalid data-environment file '{path}': {message}")]
    InvalidConfig { path: PathBuf, message: String },
    #[error(transparent)]
    Runtime(#[from] flowai_runtime::data::DataCommandError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Execution(String),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputMode {
    Text,
    Json,
    Ndjson,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Parser)]
#[command(name = "flowai-harness", version, about = "Flow AI harness CLI")]
struct Cli {
    #[command(flatten)]
    common: CommonArgs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Args)]
struct CommonArgs {
    #[arg(long, global = true)]
    data_environment: Option<PathBuf>,
    #[arg(long, global = true, value_enum, default_value = "text")]
    output: OutputMode,
    #[arg(long, global = true)]
    quiet: bool,
    #[arg(long, global = true)]
    verbose: bool,
    #[arg(long, global = true, value_enum)]
    log_level: Option<LogLevel>,
    #[arg(long, global = true)]
    no_color: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    Data {
        #[command(subcommand)]
        command: DataCommand,
    },
    Mcp {
        #[command(subcommand)]
        command: mcp::McpCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DataCommand {
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    Knowledge {
        #[command(subcommand)]
        command: KnowledgeCommand,
    },
    Catalog {
        #[command(subcommand)]
        command: CatalogCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ProfileCommand {
    Estimate(EstimateArgs),
    Table(ProfileTableArgs),
    Database(ProfileDatabaseArgs),
}

#[derive(Debug, Subcommand)]
enum KnowledgeCommand {
    Ingest(IngestKnowledgeArgs),
}

#[derive(Debug, Subcommand)]
enum CatalogCommand {
    Graph(CatalogGraphArgs),
    Export(CatalogExportArgs),
    Index {
        #[command(subcommand)]
        command: CatalogIndexCommand,
    },
}

#[derive(Debug, Subcommand)]
enum CatalogIndexCommand {
    Rebuild(CatalogIndexArgs),
    Doctor(CatalogIndexArgs),
}

#[derive(Debug, Clone, Args)]
struct EstimateArgs {
    #[command(flatten)]
    scope: ScopeArgs,
    #[arg(long)]
    database_id: String,
    #[arg(long)]
    schema: Option<String>,
    #[arg(long = "table")]
    tables: Vec<String>,
    #[arg(long = "model")]
    model_id: Option<String>,
    #[arg(long)]
    sample_size: Option<usize>,
}

#[derive(Debug, Clone, Args)]
struct ProfileTableArgs {
    #[command(flatten)]
    scope: ScopeArgs,
    #[arg(long)]
    database_id: String,
    #[arg(long)]
    table: String,
    #[arg(long)]
    schema: Option<String>,
    #[arg(long = "model")]
    model_id: Option<String>,
    #[arg(long)]
    sample_size: Option<usize>,
    #[command(flatten)]
    enrichment: EnrichmentArgs,
}

#[derive(Debug, Clone, Args)]
struct ProfileDatabaseArgs {
    #[command(flatten)]
    scope: ScopeArgs,
    #[arg(long)]
    database_id: String,
    #[arg(long)]
    schema: Option<String>,
    #[arg(long = "table")]
    tables: Vec<String>,
    #[arg(long = "model")]
    model_id: Option<String>,
    #[arg(long)]
    sample_size: Option<usize>,
    #[command(flatten)]
    enrichment: EnrichmentArgs,
}

#[derive(Debug, Clone, Args)]
struct EnrichmentArgs {
    #[arg(long)]
    schema_only: bool,
    #[arg(long)]
    anthropic_api_key: Option<String>,
    #[arg(long)]
    anthropic_model: Option<String>,
    #[arg(long)]
    anthropic_base_url: Option<String>,
}

#[derive(Debug, Clone, Args)]
struct IngestKnowledgeArgs {
    #[arg(long)]
    local_dir: PathBuf,
    #[arg(long = "ext")]
    extensions: Vec<String>,
    #[arg(long)]
    database_id: String,
    #[arg(long)]
    extract_knowledge: bool,
    #[command(flatten)]
    extraction: KnowledgeExtractionArgs,
    #[command(flatten)]
    scope: ScopeArgs,
}

#[derive(Debug, Clone, Args)]
struct CatalogGraphArgs {
    #[command(flatten)]
    scope: ScopeArgs,
    #[arg(long, value_enum, default_value = "html")]
    format: CatalogGraphFormat,
    #[arg(long)]
    output_file: Option<PathBuf>,
    #[arg(long)]
    include_columns: bool,
    #[arg(long, default_value_t = 750)]
    max_nodes: usize,
}

#[derive(Debug, Clone, Args)]
struct CatalogIndexArgs {
    #[command(flatten)]
    scope: ScopeArgs,
}

#[derive(Debug, Clone, Args)]
struct CatalogExportArgs {
    #[command(flatten)]
    scope: ScopeArgs,
    /// Destination path for the exported `catalog.entries.json` artifact.
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CatalogGraphFormat {
    Html,
    Json,
}

#[derive(Debug, Clone, Default, Args)]
struct ScopeArgs {
    #[arg(long)]
    tenant_id: Option<String>,
    #[arg(long)]
    workspace_id: Option<String>,
}

#[derive(Debug, Clone, Args)]
struct KnowledgeExtractionArgs {
    #[arg(long)]
    anthropic_api_key: Option<String>,
    #[arg(long)]
    anthropic_model: Option<String>,
    #[arg(long)]
    anthropic_base_url: Option<String>,
}

pub async fn run<I, T>(args: I) -> Result<(), CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    run_with_io(args, &mut stdout, &mut stderr).await
}

pub async fn run_with_io<I, T>(
    args: I,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            write!(stdout, "{err}")?;
            return Ok(());
        }
        Err(err) => return Err(CliError::Parse(err.to_string())),
    };
    validate_common_args(&cli.common)?;
    init_tracing(&cli.common);

    match cli.command {
        Command::Data { command } => {
            let data_environment = load_required_data_environment(&cli.common)?;
            match command {
                DataCommand::Profile { command } => match command {
                    ProfileCommand::Estimate(args) => {
                        run_profile_estimate(&cli.common, &data_environment, args, stdout).await
                    }
                    ProfileCommand::Table(args) => {
                        run_profile_table(&cli.common, &data_environment, args, stdout).await
                    }
                    ProfileCommand::Database(args) => {
                        run_profile_database(&cli.common, &data_environment, args, stdout).await
                    }
                },
                DataCommand::Knowledge { command } => match command {
                    KnowledgeCommand::Ingest(args) => {
                        run_knowledge_ingest(&cli.common, &data_environment, args, stdout).await
                    }
                },
                DataCommand::Catalog { command } => match command {
                    CatalogCommand::Graph(args) => {
                        run_catalog_graph(&cli.common, &data_environment, args, stdout).await
                    }
                    CatalogCommand::Export(args) => {
                        run_catalog_export(&cli.common, &data_environment, args, stdout).await
                    }
                    CatalogCommand::Index { command } => match command {
                        CatalogIndexCommand::Rebuild(args) => {
                            run_catalog_index_rebuild(&cli.common, &data_environment, args, stdout)
                                .await
                        }
                        CatalogIndexCommand::Doctor(args) => {
                            run_catalog_index_doctor(&cli.common, &data_environment, args, stdout)
                                .await
                        }
                    },
                },
            }
        }
        Command::Mcp { command } => mcp::run_mcp_command(&cli.common, command, stderr).await,
    }
    .map_err(|err| {
        let _ = writeln!(stderr, "{err}");
        err
    })
}

fn validate_common_args(common: &CommonArgs) -> Result<(), CliError> {
    if common.quiet && common.verbose {
        return Err(CliError::Parse(
            "--quiet and --verbose cannot be used together".to_string(),
        ));
    }
    Ok(())
}

fn init_tracing(common: &CommonArgs) {
    let level = if let Some(level) = &common.log_level {
        match level {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    } else if common.quiet {
        "error"
    } else if common.verbose {
        "debug"
    } else {
        "info"
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| level.into()),
        )
        .with_ansi(!common.no_color)
        .with_writer(io::stderr)
        .with_target(false)
        .try_init();
}

fn load_required_data_environment(common: &CommonArgs) -> Result<DataEnvironmentConfig, CliError> {
    let data_environment_path = common
        .data_environment
        .as_ref()
        .ok_or_else(|| CliError::Parse("--data-environment <path> is required".to_string()))?;
    load_data_environment(data_environment_path)
}

fn load_data_environment(path: &Path) -> Result<DataEnvironmentConfig, CliError> {
    let raw = fs::read_to_string(path).map_err(|source| CliError::ReadConfig {
        path: path.to_path_buf(),
        source,
    })?;

    match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => serde_json::from_str(&raw).map_err(|err| CliError::InvalidConfig {
            path: path.to_path_buf(),
            message: err.to_string(),
        }),
        Some("toml") => toml::from_str(&raw).map_err(|err| CliError::InvalidConfig {
            path: path.to_path_buf(),
            message: err.to_string(),
        }),
        _ => serde_json::from_str(&raw)
            .or_else(|_| toml::from_str(&raw))
            .map_err(|err| CliError::InvalidConfig {
                path: path.to_path_buf(),
                message: err.to_string(),
            }),
    }
}

fn resolve_scope(
    data_environment: &DataEnvironmentConfig,
    scope_args: &ScopeArgs,
    default_tenant_id: &str,
) -> Result<CatalogScope, CliError> {
    let tenant_id = match &scope_args.tenant_id {
        Some(value) => TenantId::new(value.clone())
            .ok_or_else(|| CliError::Parse("--tenant-id must not be blank".to_string()))?,
        None => data_environment
            .tenant_id
            .clone()
            .unwrap_or_else(|| TenantId::new_unchecked(default_tenant_id)),
    };
    let workspace_id = match &scope_args.workspace_id {
        Some(value) => WorkspaceId::new(value.clone())
            .ok_or_else(|| CliError::Parse("--workspace-id must not be blank".to_string()))?,
        None => data_environment
            .workspace_id
            .clone()
            .unwrap_or_else(WorkspaceId::default_workspace),
    };
    Ok(CatalogScope::new(tenant_id, workspace_id))
}

async fn run_profile_estimate(
    common: &CommonArgs,
    data_environment: &DataEnvironmentConfig,
    args: EstimateArgs,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let deps = ProfilingCommandDeps::new(Arc::new(SchemaOnlyEnricher));
    let scope = resolve_scope(data_environment, &args.scope, DEFAULT_DATA_TENANT_ID)?;
    let result = estimate_profiling(
        &ProfilingEstimateCommand {
            data_environment: data_environment.clone(),
            tenant_id: Some(scope.tenant_id),
            workspace_id: Some(scope.workspace_id),
            database_id: args.database_id,
            schema_name: args.schema,
            tables: args.tables,
            model_id: args.model_id,
            sample_size: args.sample_size,
        },
        &deps,
    )
    .await?;

    match common.output {
        OutputMode::Text => {
            writeln!(
                stdout,
                "model_id: {}",
                result.model_id.as_deref().unwrap_or("unpriced")
            )?;
            writeln!(stdout, "tables: {}", result.table_count)?;
            writeln!(stdout, "columns: {}", result.column_count)?;
            writeln!(
                stdout,
                "estimated_input_tokens: {}",
                result.estimated_input_tokens
            )?;
            writeln!(
                stdout,
                "estimated_output_tokens: {}",
                result.estimated_output_tokens
            )?;
            writeln!(
                stdout,
                "estimated_cached_tokens: {}",
                result.estimated_cached_tokens
            )?;
            match result.estimated_cost_usd {
                Some(cost) => writeln!(stdout, "estimated_cost_usd: {cost:.6}")?,
                None => writeln!(stdout, "estimated_cost_usd: unavailable")?,
            }
            writeln!(
                stdout,
                "estimated_duration_secs: {}",
                result.estimated_duration_secs
            )?;
        }
        OutputMode::Json => {
            serde_json::to_writer_pretty(&mut *stdout, &result)?;
            writeln!(stdout)?;
        }
        OutputMode::Ndjson => {
            serde_json::to_writer(&mut *stdout, &result)?;
            writeln!(stdout)?;
        }
    }

    Ok(())
}

async fn run_profile_table(
    common: &CommonArgs,
    data_environment: &DataEnvironmentConfig,
    args: ProfileTableArgs,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    log_json_event_buffering(common, "table profiling");
    let scope = resolve_scope(data_environment, &args.scope, DEFAULT_DATA_TENANT_ID)?;
    tracing::info!(
        tenant_id = %scope.tenant_id.as_str(),
        workspace_id = %scope.workspace_id.as_str(),
        database_id = %args.database_id,
        schema = args.schema.as_deref().unwrap_or("<default>"),
        table = %args.table,
        sample_size = args.sample_size.unwrap_or(10),
        enrichment = profile_enrichment_mode(&args.enrichment),
        "starting profile table command"
    );
    let enrichment = enrichment_args_with_profile_model(args.enrichment, args.model_id.clone());
    let deps = ProfilingCommandDeps::new(build_enricher(&enrichment)?);
    let handle = profile_table(
        ProfileTableCommand {
            data_environment: data_environment.clone(),
            tenant_id: Some(scope.tenant_id),
            workspace_id: Some(scope.workspace_id),
            database_id: args.database_id,
            schema_name: args.schema,
            table_name: args.table,
            model_id: args.model_id,
            sample_size: args.sample_size,
        },
        deps,
    )
    .await?;
    drain_profiling_events(common, handle.events, stdout).await
}

async fn run_profile_database(
    common: &CommonArgs,
    data_environment: &DataEnvironmentConfig,
    args: ProfileDatabaseArgs,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    log_json_event_buffering(common, "database profiling");
    let scope = resolve_scope(data_environment, &args.scope, DEFAULT_DATA_TENANT_ID)?;
    tracing::info!(
        tenant_id = %scope.tenant_id.as_str(),
        workspace_id = %scope.workspace_id.as_str(),
        database_id = %args.database_id,
        schema = args.schema.as_deref().unwrap_or("<default>"),
        selected_tables = args.tables.len(),
        all_tables = args.tables.is_empty(),
        sample_size = args.sample_size.unwrap_or(10),
        enrichment = profile_enrichment_mode(&args.enrichment),
        "starting profile database command"
    );
    let enrichment = enrichment_args_with_profile_model(args.enrichment, args.model_id.clone());
    let deps = ProfilingCommandDeps::new(build_enricher(&enrichment)?);
    let handle = profile_database(
        ProfileDatabaseCommand {
            data_environment: data_environment.clone(),
            tenant_id: Some(scope.tenant_id),
            workspace_id: Some(scope.workspace_id),
            database_id: args.database_id,
            schema_name: args.schema,
            tables: args.tables,
            model_id: args.model_id,
            sample_size: args.sample_size,
        },
        deps,
    )
    .await?;
    drain_profiling_events(common, handle.events, stdout).await
}

fn log_json_event_buffering(common: &CommonArgs, operation: &str) {
    if matches!(common.output, OutputMode::Json) && !common.quiet {
        tracing::info!(
            operation,
            "stdout profiling events are buffered until completion because --output json was selected; use --output ndjson for live event output"
        );
    }
}

fn profile_enrichment_mode(args: &EnrichmentArgs) -> &'static str {
    if args.schema_only {
        "schema-only"
    } else {
        "anthropic"
    }
}

fn enrichment_args_with_profile_model(
    mut enrichment: EnrichmentArgs,
    profile_model_id: Option<String>,
) -> EnrichmentArgs {
    if enrichment.anthropic_model.is_none() {
        enrichment.anthropic_model = profile_model_id;
    }
    enrichment
}

async fn run_knowledge_ingest(
    common: &CommonArgs,
    data_environment: &DataEnvironmentConfig,
    args: IngestKnowledgeArgs,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let scope = resolve_scope(data_environment, &args.scope, DEFAULT_DATA_TENANT_ID)?;
    let deps = if args.extract_knowledge {
        KnowledgeCommandDeps::new().with_enricher(build_knowledge_enricher(&args.extraction)?)
    } else {
        KnowledgeCommandDeps::new()
    };
    let handle = ingest_knowledge(
        IngestKnowledgeCommand {
            data_environment: data_environment.clone(),
            tenant_id: scope.tenant_id.to_string(),
            workspace_id: Some(scope.workspace_id),
            database_id: args.database_id,
            source: KnowledgeSourceSpec::LocalDirectory {
                path: args.local_dir,
                extensions: args.extensions,
            },
            extract_knowledge: args.extract_knowledge,
        },
        deps,
    )
    .await?;
    drain_knowledge_events(common, handle.events, stdout).await
}

async fn run_catalog_graph(
    _common: &CommonArgs,
    data_environment: &DataEnvironmentConfig,
    args: CatalogGraphArgs,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    if args.max_nodes == 0 {
        return Err(CliError::Parse(
            "--max-nodes must be greater than 0".to_string(),
        ));
    }

    let Some(catalog_config) = data_environment.catalog.clone() else {
        return Err(CliError::Execution(
            "catalog graph requires data_environment.catalog".to_string(),
        ));
    };
    let scope = resolve_scope(data_environment, &args.scope, DEFAULT_DATA_TENANT_ID)?;
    let catalog = build_catalog_for_scope(catalog_config, scope.clone())
        .await
        .map_err(|err| CliError::Execution(err.to_string()))?;
    let graph = catalog_graph::build_catalog_graph(
        catalog.as_ref(),
        &scope,
        catalog_graph::GraphBuildOptions {
            include_columns: args.include_columns,
            max_nodes: args.max_nodes,
        },
    )
    .await?;
    let rendered = match args.format {
        CatalogGraphFormat::Html => catalog_graph::render_html(&graph)?,
        CatalogGraphFormat::Json => catalog_graph::render_json(&graph)?,
    };

    if let Some(path) = args.output_file {
        fs::write(path, rendered)?;
    } else {
        write!(stdout, "{rendered}")?;
        if !rendered.ends_with('\n') {
            writeln!(stdout)?;
        }
    }

    Ok(())
}

async fn run_catalog_export(
    common: &CommonArgs,
    data_environment: &DataEnvironmentConfig,
    args: CatalogExportArgs,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let scope = resolve_scope(data_environment, &args.scope, DEFAULT_DATA_TENANT_ID)?;
    let export = export_catalog(ExportCatalogCommand {
        data_environment: data_environment.clone(),
        tenant_id: Some(scope.tenant_id),
        workspace_id: Some(scope.workspace_id),
    })
    .await?;

    if let Some(parent) = args.out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let json = serde_json::to_vec_pretty(&export.entries)?;
    fs::write(&args.out, json)?;

    let summary = &export.summary;
    match common.output {
        OutputMode::Text => {
            writeln!(stdout, "tenant_id: {}", summary.tenant_id)?;
            writeln!(stdout, "workspace_id: {}", summary.workspace_id)?;
            writeln!(stdout, "output_path: {}", args.out.display())?;
            writeln!(stdout, "entries_written: {}", summary.entries_written)?;
            for (kind, count) in &summary.counts_by_kind {
                writeln!(stdout, "  {kind}: {count}")?;
            }
            if summary.entries_written == 0 {
                writeln!(stdout, "warning: catalog is empty; wrote an empty artifact")?;
            }
        }
        OutputMode::Json => {
            serde_json::to_writer_pretty(&mut *stdout, summary)?;
            writeln!(stdout)?;
        }
        OutputMode::Ndjson => {
            serde_json::to_writer(&mut *stdout, summary)?;
            writeln!(stdout)?;
        }
    }

    Ok(())
}

async fn run_catalog_index_rebuild(
    common: &CommonArgs,
    data_environment: &DataEnvironmentConfig,
    args: CatalogIndexArgs,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let scope = resolve_scope(data_environment, &args.scope, DEFAULT_DATA_TENANT_ID)?;
    let summary = rebuild_catalog_search_index_from_environment(data_environment, scope)
        .await
        .map_err(|err| CliError::Execution(err.to_string()))?;
    match common.output {
        OutputMode::Text => {
            writeln!(stdout, "tenant_id: {}", summary.tenant_id)?;
            writeln!(stdout, "workspace_id: {}", summary.workspace_id)?;
            writeln!(stdout, "indexed_entries: {}", summary.indexed_entries)?;
            writeln!(stdout, "skipped_entries: {}", summary.skipped_entries)?;
            for warning in summary.warnings {
                writeln!(stdout, "warning: {warning}")?;
            }
        }
        OutputMode::Json => {
            serde_json::to_writer_pretty(&mut *stdout, &summary)?;
            writeln!(stdout)?;
        }
        OutputMode::Ndjson => {
            serde_json::to_writer(&mut *stdout, &summary)?;
            writeln!(stdout)?;
        }
    }
    Ok(())
}

async fn run_catalog_index_doctor(
    common: &CommonArgs,
    data_environment: &DataEnvironmentConfig,
    args: CatalogIndexArgs,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let scope = resolve_scope(data_environment, &args.scope, DEFAULT_DATA_TENANT_ID)?;
    let report = doctor_catalog_search_index_from_environment(data_environment, scope)
        .await
        .map_err(|err| CliError::Execution(err.to_string()))?;
    match common.output {
        OutputMode::Text => {
            writeln!(stdout, "tenant_id: {}", report.tenant_id)?;
            writeln!(stdout, "workspace_id: {}", report.workspace_id)?;
            writeln!(stdout, "index_path: {}", report.index_path)?;
            writeln!(stdout, "health: {}", catalog_health_label(&report.health))?;
            if let Some(diagnostics) = &report.relation_diagnostics {
                writeln!(stdout, "relation_total: {}", diagnostics.total_relations)?;
                writeln!(
                    stdout,
                    "relation_orphaned: {}",
                    diagnostics.orphaned_relations
                )?;
                writeln!(
                    stdout,
                    "relation_database_mismatched: {}",
                    diagnostics.database_mismatched_relations
                )?;
                for sample in &diagnostics.samples {
                    writeln!(
                        stdout,
                        "relation_sample: {}",
                        render_relation_diagnostic_sample(sample)
                    )?;
                }
            }
        }
        OutputMode::Json => {
            serde_json::to_writer_pretty(&mut *stdout, &report)?;
            writeln!(stdout)?;
        }
        OutputMode::Ndjson => {
            serde_json::to_writer(&mut *stdout, &report)?;
            writeln!(stdout)?;
        }
    }
    Ok(())
}

fn catalog_health_label(health: &agent_fw_catalog::CatalogSearchHealth) -> &'static str {
    match health {
        agent_fw_catalog::CatalogSearchHealth::Ready { .. } => "ready",
        agent_fw_catalog::CatalogSearchHealth::Stale { .. } => "stale",
        agent_fw_catalog::CatalogSearchHealth::Unavailable { .. } => "unavailable",
    }
}

fn render_relation_diagnostic_sample(sample: &CatalogRelationDiagnostic) -> String {
    format!(
        "source_id={} target_id={} relation_kind={} issue={}",
        sample.source_id,
        sample.target_id,
        sample.relation_kind,
        render_relation_issue(&sample.issue)
    )
}

fn render_relation_issue(issue: &CatalogRelationIssue) -> String {
    match issue {
        CatalogRelationIssue::MissingSource => "missing_source".to_string(),
        CatalogRelationIssue::MissingTarget => "missing_target".to_string(),
        CatalogRelationIssue::InvalidRelationshipMetadata(error) => {
            format!("invalid_relationship_metadata({error})")
        }
        CatalogRelationIssue::RelationshipTargetDatabaseMismatch {
            expected_database_id,
            actual_database_id,
        } => format!(
            "relationship_target_database_mismatch(expected_database_id={}, actual_database_id={})",
            expected_database_id,
            actual_database_id.as_deref().unwrap_or("<missing>")
        ),
        CatalogRelationIssue::RelationEndpointDatabaseMismatch {
            source_database_id,
            target_database_id,
        } => format!(
            "relation_endpoint_database_mismatch(source_database_id={}, target_database_id={})",
            source_database_id, target_database_id
        ),
    }
}

fn build_enricher(args: &EnrichmentArgs) -> Result<Arc<dyn SemanticEnricher>, CliError> {
    if args.schema_only {
        tracing::info!("using schema-only enrichment; Anthropic API will not be called");
        return Ok(Arc::new(SchemaOnlyEnricher));
    }

    build_anthropic_enricher(
        args.anthropic_api_key.clone(),
        args.anthropic_model.clone(),
        args.anthropic_base_url.clone(),
        "ANTHROPIC_API_KEY is required for LLM enrichment; pass --schema-only for deterministic fallback",
        "FLOWAI_PROFILE_ANTHROPIC_MODEL",
    )
}

fn build_knowledge_enricher(
    args: &KnowledgeExtractionArgs,
) -> Result<Arc<dyn SemanticEnricher>, CliError> {
    build_anthropic_enricher(
        args.anthropic_api_key.clone(),
        args.anthropic_model.clone(),
        args.anthropic_base_url.clone(),
        "ANTHROPIC_API_KEY is required when --extract-knowledge is enabled",
        "FLOWAI_KNOWLEDGE_ANTHROPIC_MODEL",
    )
}

fn build_anthropic_enricher(
    api_key: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    missing_help: &str,
    model_env_var: &str,
) -> Result<Arc<dyn SemanticEnricher>, CliError> {
    let api_key = api_key
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .ok_or_else(|| CliError::Execution(missing_help.to_string()))?;

    let mut enricher = AnthropicEnricher::new(api_key);
    if let Some(model) = model.or_else(|| std::env::var(model_env_var).ok()) {
        tracing::info!(model = %model, "using Anthropic enrichment model");
        enricher = enricher.with_model(model);
    } else {
        tracing::info!("using default Anthropic enrichment model");
    }
    if let Some(base_url) = base_url.or_else(|| std::env::var("ANTHROPIC_BASE_URL").ok()) {
        tracing::info!(base_url = %base_url, "using custom Anthropic base URL");
        enricher = enricher.with_base_url(base_url);
    }

    Ok(Arc::new(enricher))
}

async fn drain_knowledge_events(
    common: &CommonArgs,
    mut events: tokio::sync::mpsc::Receiver<KnowledgeIngestEvent>,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let mut collected = Vec::new();
    let mut summary = None;
    let mut errors = Vec::new();

    while let Some(event) = events.recv().await {
        match common.output {
            OutputMode::Text => render_text_knowledge_event(common, &event, stdout)?,
            OutputMode::Ndjson => {
                serde_json::to_writer(&mut *stdout, &event)?;
                writeln!(stdout)?;
            }
            OutputMode::Json => collected.push(event.clone()),
        }

        match event {
            KnowledgeIngestEvent::Completed(completed_summary) => {
                if !completed_summary.errors.is_empty() {
                    errors.extend(completed_summary.errors.clone());
                }
                summary = Some(completed_summary);
            }
            KnowledgeIngestEvent::Error { message } => errors.push(message),
            _ => {}
        }
    }

    if matches!(common.output, OutputMode::Json) {
        serde_json::to_writer_pretty(&mut *stdout, &collected)?;
        writeln!(stdout)?;
    }

    if !errors.is_empty() {
        return Err(CliError::Execution(errors.join("; ")));
    }

    if matches!(common.output, OutputMode::Text) {
        if let Some(summary) = summary {
            render_text_knowledge_summary(stdout, &summary)?;
        }
    }

    Ok(())
}

async fn drain_profiling_events(
    common: &CommonArgs,
    mut events: tokio::sync::mpsc::Receiver<IngestionEvent>,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let mut collected = Vec::new();
    let mut summary = None;
    let mut errors = Vec::new();

    while let Some(event) = events.recv().await {
        match common.output {
            OutputMode::Text => render_text_event(common, &event, stdout)?,
            OutputMode::Ndjson => {
                serde_json::to_writer(&mut *stdout, &event)?;
                writeln!(stdout)?;
            }
            OutputMode::Json => collected.push(event.clone()),
        }

        match event {
            IngestionEvent::Completed {
                summary: completed_summary,
            } => summary = Some(completed_summary),
            IngestionEvent::TableFailed { table_name, error } => {
                errors.push(format!("{table_name}: {error}"));
            }
            IngestionEvent::Error { message } => errors.push(message),
            _ => {}
        }
    }

    if matches!(common.output, OutputMode::Json) {
        serde_json::to_writer_pretty(&mut *stdout, &collected)?;
        writeln!(stdout)?;
    }

    if !errors.is_empty() {
        return Err(CliError::Execution(errors.join("; ")));
    }

    if matches!(common.output, OutputMode::Text) {
        if let Some(summary) = summary {
            render_text_summary(stdout, &summary)?;
        }
    }

    Ok(())
}

fn render_text_event(
    common: &CommonArgs,
    event: &IngestionEvent,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    if common.quiet {
        return Ok(());
    }

    match event {
        IngestionEvent::Started { job_id } => {
            writeln!(stdout, "started profiling job {job_id}")?;
        }
        IngestionEvent::Progress { status } => {
            writeln!(stdout, "progress: {status:?}")?;
        }
        IngestionEvent::TableProfiled {
            table_name,
            columns,
            duration_ms,
        } => {
            writeln!(
                stdout,
                "profiled {table_name} ({columns} columns in {duration_ms} ms)"
            )?;
        }
        IngestionEvent::TableEnriched {
            table_name,
            source,
            fallback_reason,
        } => {
            if let Some(reason) = fallback_reason {
                writeln!(stdout, "enriched {table_name} ({source:?}: {reason})")?;
            } else {
                writeln!(stdout, "enriched {table_name} ({source:?})")?;
            }
        }
        IngestionEvent::TableCompleted {
            table_name,
            summary,
        } => {
            writeln!(
                stdout,
                "indexed {table_name} ({} catalog items)",
                summary.catalog_items_indexed
            )?;
        }
        IngestionEvent::TableFailed { table_name, error } => {
            writeln!(stdout, "table failed: {table_name}: {error}")?;
        }
        IngestionEvent::Completed { .. } => {}
        IngestionEvent::Error { message } => {
            writeln!(stdout, "error: {message}")?;
        }
    }
    Ok(())
}

fn render_text_summary(stdout: &mut dyn Write, summary: &IngestionSummary) -> Result<(), CliError> {
    writeln!(stdout, "completed profiling run")?;
    writeln!(stdout, "tables_discovered: {}", summary.tables_discovered)?;
    writeln!(stdout, "columns_profiled: {}", summary.columns_profiled)?;
    writeln!(stdout, "enums_extracted: {}", summary.enums_extracted)?;
    writeln!(
        stdout,
        "relationships_found: {}",
        summary.relationships_found
    )?;
    writeln!(
        stdout,
        "catalog_items_indexed: {}",
        summary.catalog_items_indexed
    )?;
    writeln!(stdout, "duration_ms: {}", summary.duration_ms)?;
    Ok(())
}

fn render_text_knowledge_event(
    common: &CommonArgs,
    event: &KnowledgeIngestEvent,
    stdout: &mut dyn Write,
) -> Result<(), CliError> {
    if common.quiet {
        return Ok(());
    }

    match event {
        KnowledgeIngestEvent::Discovered { total } => {
            writeln!(stdout, "discovered {total} documents")?;
        }
        KnowledgeIngestEvent::Ingesting {
            current,
            total,
            name,
        } => {
            writeln!(stdout, "ingesting {current}/{total}: {name}")?;
        }
        KnowledgeIngestEvent::Completed(_) => {}
        KnowledgeIngestEvent::Error { message } => {
            writeln!(stdout, "error: {message}")?;
        }
    }

    Ok(())
}

fn render_text_knowledge_summary(
    stdout: &mut dyn Write,
    summary: &flowai_runtime::data::KnowledgeIngestSummary,
) -> Result<(), CliError> {
    writeln!(stdout, "completed knowledge ingest")?;
    writeln!(stdout, "scanned: {}", summary.scanned)?;
    writeln!(stdout, "new: {}", summary.new)?;
    writeln!(stdout, "skipped_duplicate: {}", summary.skipped_duplicate)?;
    writeln!(stdout, "errors: {}", summary.errors.len())?;
    Ok(())
}

struct SchemaOnlyEnricher;

#[async_trait]
impl SemanticEnricher for SchemaOnlyEnricher {
    async fn enrich_table(
        &self,
        request: TableEnrichmentRequest,
    ) -> Result<EnrichmentResult, EnrichmentError> {
        Ok(
            EnrichmentResult::fallback(builder::fallback_semantic_profile(
                &request.table,
                &request.table.schema_name,
                &request.table.table_name,
            ))
            .with_fallback_reason("schema-only enrichment requested"),
        )
    }

    async fn extract_knowledge(
        &self,
        _request: KnowledgeExtractionRequest,
    ) -> Result<Vec<KnowledgeItem>, EnrichmentError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_catalog::{CatalogEntry, CatalogKind, CatalogRelation};
    use flowai_runtime::storage::{
        build_catalog_for_scope, build_kv_store_from_environment,
        open_catalog_for_writes_for_scope, CatalogSearchConfig, CatalogStorageConfig,
        KvStorageConfig, TargetDatabaseStorageConfig,
    };
    use rusqlite::Connection;
    use serde_json::json;
    use uuid::Uuid;

    #[tokio::test]
    async fn estimate_command_renders_json() {
        let target_path = temp_sqlite_path("cli-estimate-target");
        seed_target_database(&target_path);
        let env_path = temp_json_path("cli-estimate-env");
        write_data_environment_file(
            &env_path,
            &DataEnvironmentConfig {
                tenant_id: None,
                workspace_id: None,
                kv: None,
                catalog: None,
                catalog_search: None,
                target_database: Some(TargetDatabaseStorageConfig::Sqlite {
                    url: sqlite_url(&target_path),
                }),
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "--output",
                "json",
                "data",
                "profile",
                "estimate",
                "--tenant-id",
                "tenant-a",
                "--workspace-id",
                "workspace-a",
                "--database-id",
                "acme",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();

        let rendered = String::from_utf8(stdout).unwrap();
        assert!(rendered.contains("\"tableCount\": 2"));
        assert!(stderr.is_empty());

        let _ = std::fs::remove_file(&target_path);
        let _ = std::fs::remove_file(&env_path);
    }

    #[tokio::test]
    async fn version_flag_renders_package_version_without_data_environment() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        run_with_io(["flowai-harness", "--version"], &mut stdout, &mut stderr)
            .await
            .unwrap();

        let rendered = String::from_utf8(stdout).unwrap();
        assert!(rendered.starts_with("flowai-harness "));
        assert!(rendered.contains(env!("CARGO_PKG_VERSION")));
        assert!(stderr.is_empty());
    }

    #[test]
    fn profile_model_argument_feeds_anthropic_model_when_not_overridden() {
        let enrichment = enrichment_args_with_profile_model(
            EnrichmentArgs {
                schema_only: false,
                anthropic_api_key: None,
                anthropic_model: None,
                anthropic_base_url: None,
            },
            Some("claude-profile-model".to_string()),
        );

        assert_eq!(
            enrichment.anthropic_model.as_deref(),
            Some("claude-profile-model")
        );
    }

    #[test]
    fn explicit_anthropic_model_overrides_profile_model_argument() {
        let enrichment = enrichment_args_with_profile_model(
            EnrichmentArgs {
                schema_only: false,
                anthropic_api_key: None,
                anthropic_model: Some("claude-explicit-model".to_string()),
                anthropic_base_url: None,
            },
            Some("claude-profile-model".to_string()),
        );

        assert_eq!(
            enrichment.anthropic_model.as_deref(),
            Some("claude-explicit-model")
        );
    }

    #[tokio::test]
    async fn profile_table_command_persists_catalog_with_schema_only_enrichment() {
        let target_path = temp_sqlite_path("cli-table-target");
        let catalog_path = temp_sqlite_path("cli-table-catalog");
        let env_path = temp_json_path("cli-table-env");
        seed_target_database(&target_path);
        write_data_environment_file(
            &env_path,
            &DataEnvironmentConfig {
                tenant_id: None,
                workspace_id: None,
                kv: None,
                catalog: Some(CatalogStorageConfig::Sqlite {
                    url: sqlite_url(&catalog_path),
                    ensure_schema: true,
                }),
                catalog_search: None,
                target_database: Some(TargetDatabaseStorageConfig::Sqlite {
                    url: sqlite_url(&target_path),
                }),
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "--output",
                "ndjson",
                "data",
                "profile",
                "table",
                "--database-id",
                "acme",
                "--table",
                "products",
                "--schema-only",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();

        let rendered = String::from_utf8(stdout).unwrap();
        assert!(rendered.contains("\"type\":\"completed\""));
        assert!(stderr.is_empty());

        let catalog = build_catalog_for_scope(
            CatalogStorageConfig::Sqlite {
                url: sqlite_url(&catalog_path),
                ensure_schema: true,
            },
            default_data_scope(),
        )
        .await
        .unwrap();
        let tables = catalog.list_tables().await.unwrap();
        let products = tables
            .iter()
            .find(|entry| entry.name == "products")
            .expect("products table should be persisted");
        let metadata: agent_fw_catalog::TableMetadata =
            serde_json::from_value(products.metadata.clone()).unwrap();
        assert!(metadata.source.profiling_run_id.is_some());
        assert_eq!(
            metadata.source.enrichment_source.as_deref(),
            Some("fallback")
        );
        assert_eq!(metadata.source.model_id, None);
        assert_eq!(
            metadata.source.fallback_reason.as_deref(),
            Some("schema-only enrichment requested")
        );
        assert_eq!(metadata.source.schema_snapshot_at, None);
        assert_eq!(metadata.source.target_fingerprint, None);

        let _ = std::fs::remove_file(&target_path);
        let _ = std::fs::remove_file(&catalog_path);
        let _ = std::fs::remove_file(&env_path);
    }

    #[tokio::test]
    async fn profile_table_command_writes_under_cli_scope_flags() {
        let target_path = temp_sqlite_path("cli-table-scope-target");
        let catalog_path = temp_sqlite_path("cli-table-scope-catalog");
        let env_path = temp_json_path("cli-table-scope-env");
        seed_target_database(&target_path);
        write_data_environment_file(
            &env_path,
            &DataEnvironmentConfig {
                tenant_id: None,
                workspace_id: None,
                kv: None,
                catalog: Some(CatalogStorageConfig::Sqlite {
                    url: sqlite_url(&catalog_path),
                    ensure_schema: true,
                }),
                catalog_search: None,
                target_database: Some(TargetDatabaseStorageConfig::Sqlite {
                    url: sqlite_url(&target_path),
                }),
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "--output",
                "ndjson",
                "data",
                "profile",
                "table",
                "--tenant-id",
                "tenant-a",
                "--workspace-id",
                "workspace-a",
                "--database-id",
                "acme",
                "--table",
                "products",
                "--schema-only",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();

        let rendered = String::from_utf8(stdout).unwrap();
        assert!(rendered.contains("\"type\":\"completed\""));
        assert!(stderr.is_empty());

        let scoped_catalog = build_catalog_for_scope(
            CatalogStorageConfig::Sqlite {
                url: sqlite_url(&catalog_path),
                ensure_schema: true,
            },
            CatalogScope::new(
                TenantId::new_unchecked("tenant-a"),
                WorkspaceId::new_unchecked("workspace-a"),
            ),
        )
        .await
        .unwrap();
        assert!(scoped_catalog
            .list_tables()
            .await
            .unwrap()
            .iter()
            .any(|entry| entry.name == "products"));

        let wrong_scope_catalog = build_catalog_for_scope(
            CatalogStorageConfig::Sqlite {
                url: sqlite_url(&catalog_path),
                ensure_schema: true,
            },
            CatalogScope::new(
                TenantId::new_unchecked("tenant-b"),
                WorkspaceId::new_unchecked("workspace-a"),
            ),
        )
        .await
        .unwrap();
        assert!(wrong_scope_catalog.list_tables().await.unwrap().is_empty());

        let _ = std::fs::remove_file(&target_path);
        let _ = std::fs::remove_file(&catalog_path);
        let _ = std::fs::remove_file(&env_path);
    }

    #[tokio::test]
    async fn profile_database_command_profiles_requested_subset_and_renders_ndjson() {
        let target_path = temp_sqlite_path("cli-db-target");
        let catalog_path = temp_sqlite_path("cli-db-catalog");
        let env_path = temp_json_path("cli-db-env");
        seed_target_database(&target_path);
        write_data_environment_file(
            &env_path,
            &DataEnvironmentConfig {
                tenant_id: None,
                workspace_id: None,
                kv: None,
                catalog: Some(CatalogStorageConfig::Sqlite {
                    url: sqlite_url(&catalog_path),
                    ensure_schema: true,
                }),
                catalog_search: None,
                target_database: Some(TargetDatabaseStorageConfig::Sqlite {
                    url: sqlite_url(&target_path),
                }),
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "--output",
                "ndjson",
                "data",
                "profile",
                "database",
                "--database-id",
                "acme",
                "--table",
                "products",
                "--sample-size",
                "1",
                "--schema-only",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();

        let rendered = String::from_utf8(stdout).unwrap();
        assert!(rendered.contains("\"type\":\"completed\""));
        assert!(rendered.contains("\"tableName\":\"products\""));
        assert!(!rendered.contains("\"tableName\":\"orders\""));
        assert!(stderr.is_empty());

        let catalog = build_catalog_for_scope(
            CatalogStorageConfig::Sqlite {
                url: sqlite_url(&catalog_path),
                ensure_schema: true,
            },
            default_data_scope(),
        )
        .await
        .unwrap();
        let tables = catalog.list_tables().await.unwrap();
        assert!(tables.iter().any(|entry| entry.name == "products"));
        assert!(!tables.iter().any(|entry| entry.name == "orders"));

        let _ = std::fs::remove_file(&target_path);
        let _ = std::fs::remove_file(&catalog_path);
        let _ = std::fs::remove_file(&env_path);
    }

    #[tokio::test]
    async fn knowledge_ingest_command_persists_documents_into_sqlite_kv() {
        let kv_path = temp_sqlite_path("cli-knowledge-kv");
        let env_path = temp_json_path("cli-knowledge-env");
        let knowledge_dir = temp_directory("cli-knowledge-docs");
        std::fs::write(
            knowledge_dir.join("metrics.md"),
            "# Metrics\nRevenue comes from orders.",
        )
        .unwrap();
        std::fs::write(knowledge_dir.join("ignored.rs"), "fn main() {}").unwrap();
        write_data_environment_file(
            &env_path,
            &DataEnvironmentConfig {
                tenant_id: None,
                workspace_id: None,
                kv: Some(KvStorageConfig::Sqlite {
                    url: sqlite_url(&kv_path),
                    ensure_schema: true,
                }),
                catalog: None,
                catalog_search: None,
                target_database: None,
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "--output",
                "ndjson",
                "data",
                "knowledge",
                "ingest",
                "--tenant-id",
                "acme",
                "--workspace-id",
                "workspace-a",
                "--database-id",
                "warehouse",
                "--local-dir",
                knowledge_dir.to_str().unwrap(),
                "--ext",
                "md",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();

        let rendered = String::from_utf8(stdout).unwrap();
        assert!(rendered.contains("\"type\":\"completed\""));
        assert!(stderr.is_empty());

        let kv = build_kv_store_from_environment(&DataEnvironmentConfig {
            tenant_id: None,
            workspace_id: None,
            kv: Some(KvStorageConfig::Sqlite {
                url: sqlite_url(&kv_path),
                ensure_schema: true,
            }),
            catalog: None,
            catalog_search: None,
            target_database: None,
            legacy_target_database_url: None,
            legacy_target_database_schema: None,
        })
        .await
        .unwrap();
        assert!(kv
            .list_keys("acme", "data:document:")
            .await
            .unwrap()
            .is_empty());
        let doc_keys = kv
            .list_keys("acme::workspace:workspace-a", "data:document:")
            .await
            .unwrap();
        assert_eq!(doc_keys.len(), 1);

        let _ = std::fs::remove_file(&kv_path);
        let _ = std::fs::remove_file(&env_path);
        let _ = std::fs::remove_dir_all(&knowledge_dir);
    }

    #[tokio::test]
    async fn knowledge_ingest_command_requires_database_id() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let err = run_with_io(
            [
                "flowai-harness",
                "data",
                "knowledge",
                "ingest",
                "--local-dir",
                "/tmp",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("--database-id <DATABASE_ID>"));
        assert!(stdout.is_empty());
    }

    #[tokio::test]
    async fn missing_data_environment_returns_parse_error() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let err = run_with_io(
            [
                "flowai-harness",
                "data",
                "profile",
                "estimate",
                "--database-id",
                "acme",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("--data-environment <path> is required"));
    }

    #[tokio::test]
    async fn mcp_help_does_not_require_data_environment() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        run_with_io(
            ["flowai-harness", "mcp", "--help"],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();

        let rendered = String::from_utf8(stdout).unwrap();
        assert!(rendered.contains("toolkit"));
        assert!(stderr.is_empty());
    }

    #[tokio::test]
    async fn mcp_toolkit_help_does_not_require_data_environment() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        run_with_io(
            ["flowai-harness", "mcp", "toolkit", "--help"],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();

        let rendered = String::from_utf8(stdout).unwrap();
        assert!(rendered.contains("--toolkit"));
        assert!(rendered.contains("--tenant-id"));
        assert!(rendered.contains("--transport"));
        assert!(stderr.is_empty());
    }

    #[tokio::test]
    async fn catalog_graph_help_does_not_require_data_environment() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        run_with_io(
            ["flowai-harness", "data", "catalog", "graph", "--help"],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();

        let rendered = String::from_utf8(stdout).unwrap();
        assert!(rendered.contains("--format"));
        assert!(rendered.contains("--include-columns"));
        assert!(rendered.contains("--output-file"));
        assert!(stderr.is_empty());
    }

    #[tokio::test]
    async fn catalog_index_rebuild_and_doctor_render_json_and_text_samples() {
        let catalog_path = temp_sqlite_path("cli-catalog-index-catalog");
        let index_root = temp_directory("cli-catalog-index-root");
        let env_path = temp_json_path("cli-catalog-index-env");
        let scope = CatalogScope::new(
            TenantId::new_unchecked("tenant-a"),
            WorkspaceId::new_unchecked("workspace-a"),
        );
        let opened = open_catalog_for_writes_for_scope(
            CatalogStorageConfig::Sqlite {
                url: sqlite_url(&catalog_path),
                ensure_schema: true,
            },
            scope,
        )
        .await
        .unwrap();
        let mut orders = catalog_graph_table("orders");
        orders.links.push(CatalogRelation {
            target_id: "table:public.missing_orders".to_string(),
            kind: "has_column".to_string(),
            description: None,
        });
        orders.links.push(CatalogRelation {
            target_id: "table:public.products".to_string(),
            kind: "references_table".to_string(),
            description: None,
        });
        opened
            .writer
            .save_items(vec![
                orders,
                catalog_graph_table_with_database("products", "other_warehouse"),
            ])
            .await
            .unwrap();
        Connection::open(&catalog_path)
            .unwrap()
            .execute(
                "INSERT INTO catalog_relations
                 (tenant_id, workspace_id, source_id, target_id, kind, description)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    "tenant-a",
                    "workspace-a",
                    "table:public.missing_source",
                    "table:public.orders",
                    "depends_on",
                    Option::<String>::None,
                ],
            )
            .unwrap();
        write_data_environment_file(
            &env_path,
            &DataEnvironmentConfig {
                tenant_id: Some(TenantId::new_unchecked("tenant-a")),
                workspace_id: Some(WorkspaceId::new_unchecked("workspace-a")),
                kv: None,
                catalog: Some(CatalogStorageConfig::Sqlite {
                    url: sqlite_url(&catalog_path),
                    ensure_schema: true,
                }),
                catalog_search: Some(CatalogSearchConfig {
                    index_path: index_root.clone(),
                    rebuild_on_start: false,
                    write_through: false,
                }),
                target_database: None,
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "--output",
                "json",
                "data",
                "catalog",
                "index",
                "rebuild",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();
        let rebuild: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(rebuild["indexedEntries"], 2);
        assert!(stderr.is_empty());

        stdout.clear();
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "--output",
                "json",
                "data",
                "catalog",
                "index",
                "doctor",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();
        let doctor: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(doctor["health"]["status"], "ready");
        assert_eq!(doctor["relationDiagnostics"]["orphanedRelations"], 2);
        assert_eq!(
            doctor["relationDiagnostics"]["orphanedCountsByKind"]["has_column"],
            1
        );
        assert_eq!(
            doctor["relationDiagnostics"]["orphanedCountsByKind"]["depends_on"],
            1
        );
        assert_eq!(
            doctor["relationDiagnostics"]["databaseMismatchedRelations"],
            1
        );
        assert_eq!(
            doctor["relationDiagnostics"]["databaseMismatchCountsByKind"]["references_table"],
            1
        );
        assert_eq!(
            doctor["relationDiagnostics"]["samples"][0]["targetId"],
            "table:public.missing_orders"
        );
        assert!(doctor["relationDiagnostics"]["samples"]
            .as_array()
            .unwrap()
            .iter()
            .any(|sample| sample["sourceId"] == "table:public.missing_source"
                && sample["issue"] == "missingSource"));
        assert!(doctor["relationDiagnostics"]["samples"]
            .as_array()
            .unwrap()
            .iter()
            .any(|sample| sample["sourceId"] == "table:public.orders"
                && sample["targetId"] == "table:public.products"
                && sample["relationKind"] == "references_table"
                && sample["issue"]["relationEndpointDatabaseMismatch"]["sourceDatabaseId"]
                    == "warehouse"
                && sample["issue"]["relationEndpointDatabaseMismatch"]["targetDatabaseId"]
                    == "other_warehouse"));
        assert!(stderr.is_empty());

        stdout.clear();
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "data",
                "catalog",
                "index",
                "doctor",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();
        let rendered = String::from_utf8(stdout).unwrap();
        assert!(rendered.contains("relation_orphaned: 2"));
        assert!(rendered.contains("relation_database_mismatched: 1"));
        assert!(rendered.contains(
            "relation_sample: source_id=table:public.orders target_id=table:public.missing_orders relation_kind=has_column issue=missing_target"
        ));
        assert!(rendered.contains(
            "relation_sample: source_id=table:public.orders target_id=table:public.products relation_kind=references_table issue=relation_endpoint_database_mismatch(source_database_id=warehouse, target_database_id=other_warehouse)"
        ));
        assert!(rendered.contains(
            "relation_sample: source_id=table:public.missing_source target_id=table:public.orders relation_kind=depends_on issue=missing_source"
        ));
        assert!(stderr.is_empty());

        let _ = std::fs::remove_file(&catalog_path);
        let _ = std::fs::remove_file(&env_path);
        let _ = std::fs::remove_dir_all(&index_root);
    }

    #[tokio::test]
    async fn catalog_graph_command_renders_json_from_inline_catalog() {
        let env_path = temp_json_path("cli-catalog-graph-env");
        write_data_environment_file(
            &env_path,
            &DataEnvironmentConfig {
                tenant_id: Some(TenantId::new_unchecked("tenant-a")),
                workspace_id: Some(WorkspaceId::new_unchecked("workspace-a")),
                kv: None,
                catalog: Some(CatalogStorageConfig::Inline {
                    entries: vec![catalog_graph_table("orders")],
                }),
                catalog_search: None,
                target_database: None,
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "data",
                "catalog",
                "graph",
                "--format",
                "json",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();

        let rendered: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(rendered["summary"]["tenantId"], "tenant-a");
        assert_eq!(rendered["summary"]["workspaceId"], "workspace-a");
        assert_eq!(rendered["nodes"][0]["id"], "table:public.orders");
        assert!(stderr.is_empty());

        let _ = std::fs::remove_file(&env_path);
    }

    #[tokio::test]
    async fn catalog_graph_command_writes_html_output_file() {
        let env_path = temp_json_path("cli-catalog-graph-html-env");
        let output_path =
            std::env::temp_dir().join(format!("cli-catalog-graph-{}.html", Uuid::new_v4()));
        write_data_environment_file(
            &env_path,
            &DataEnvironmentConfig {
                tenant_id: Some(TenantId::new_unchecked("tenant-a")),
                workspace_id: Some(WorkspaceId::new_unchecked("workspace-a")),
                kv: None,
                catalog: Some(CatalogStorageConfig::Inline {
                    entries: vec![catalog_graph_table("orders")],
                }),
                catalog_search: None,
                target_database: None,
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "data",
                "catalog",
                "graph",
                "--output-file",
                output_path.to_str().unwrap(),
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();

        let rendered = std::fs::read_to_string(&output_path).unwrap();
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
        assert!(rendered.contains("id=\"catalog-graph-data\""));
        assert!(rendered.contains("3d-force-graph"));
        assert!(rendered.contains("table:public.orders"));

        let _ = std::fs::remove_file(&env_path);
        let _ = std::fs::remove_file(&output_path);
    }

    #[tokio::test]
    async fn catalog_graph_requires_catalog_config() {
        let env_path = temp_json_path("cli-catalog-graph-missing-catalog-env");
        write_data_environment_file(
            &env_path,
            &DataEnvironmentConfig {
                tenant_id: Some(TenantId::new_unchecked("tenant-a")),
                workspace_id: Some(WorkspaceId::new_unchecked("workspace-a")),
                kv: None,
                catalog: None,
                catalog_search: None,
                target_database: None,
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let err = run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "data",
                "catalog",
                "graph",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .expect_err("catalog graph should fail without catalog config");

        assert!(err.to_string().contains("data_environment.catalog"));
        assert!(stdout.is_empty());

        let _ = std::fs::remove_file(&env_path);
    }

    #[tokio::test]
    async fn catalog_graph_rejects_zero_max_nodes_before_catalog_build() {
        let env_path = temp_json_path("cli-catalog-graph-zero-max-env");
        let catalog_path = std::env::temp_dir()
            .join(format!("cli-catalog-graph-missing-{}", Uuid::new_v4()))
            .join("catalog.db");
        write_data_environment_file(
            &env_path,
            &DataEnvironmentConfig {
                tenant_id: Some(TenantId::new_unchecked("tenant-a")),
                workspace_id: Some(WorkspaceId::new_unchecked("workspace-a")),
                kv: None,
                catalog: Some(CatalogStorageConfig::Sqlite {
                    url: sqlite_url(&catalog_path),
                    ensure_schema: false,
                }),
                catalog_search: None,
                target_database: None,
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
        );

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let err = run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "data",
                "catalog",
                "graph",
                "--max-nodes",
                "0",
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .expect_err("catalog graph should reject zero max_nodes before opening catalog");

        assert_eq!(err.to_string(), "--max-nodes must be greater than 0");
        assert!(stdout.is_empty());

        let _ = std::fs::remove_file(&env_path);
    }

    #[tokio::test]
    async fn catalog_export_command_writes_sorted_deterministic_artifact() {
        let target_path = temp_sqlite_path("cli-export-target");
        let catalog_path = temp_sqlite_path("cli-export-catalog");
        let env_path = temp_json_path("cli-export-env");
        let out_path = temp_json_path("cli-export-artifact");
        seed_target_database(&target_path);
        write_data_environment_file(
            &env_path,
            &DataEnvironmentConfig {
                tenant_id: None,
                workspace_id: None,
                kv: None,
                catalog: Some(CatalogStorageConfig::Sqlite {
                    url: sqlite_url(&catalog_path),
                    ensure_schema: true,
                }),
                catalog_search: None,
                target_database: Some(TargetDatabaseStorageConfig::Sqlite {
                    url: sqlite_url(&target_path),
                }),
                legacy_target_database_url: None,
                legacy_target_database_schema: None,
            },
        );

        // Profile the whole database into the sqlite catalog.
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "--output",
                "ndjson",
                "data",
                "profile",
                "database",
                "--database-id",
                "acme",
                "--schema-only",
            ],
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .await
        .unwrap();

        // Export the catalog to a JSON artifact.
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "--output",
                "json",
                "data",
                "catalog",
                "export",
                "--out",
                out_path.to_str().unwrap(),
            ],
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();

        let summary = String::from_utf8(stdout).unwrap();
        assert!(summary.contains("\"entriesWritten\""));
        assert!(stderr.is_empty());

        let artifact = std::fs::read(&out_path).unwrap();
        let entries: Vec<CatalogEntry> = serde_json::from_slice(&artifact).unwrap();
        assert!(!entries.is_empty());
        assert!(entries.iter().all(|e| e.kind != CatalogKind::Enum));
        assert!(entries
            .iter()
            .any(|e| e.kind == CatalogKind::Table && e.name == "products"));
        // Tables sort ahead of every non-table entry.
        if let Some(idx) = entries.iter().position(|e| e.kind != CatalogKind::Table) {
            assert!(entries[..idx].iter().all(|e| e.kind == CatalogKind::Table));
        }

        // Determinism: a second export of the same catalog is byte-identical.
        let out_path_2 = temp_json_path("cli-export-artifact-2");
        run_with_io(
            [
                "flowai-harness",
                "--data-environment",
                env_path.to_str().unwrap(),
                "data",
                "catalog",
                "export",
                "--out",
                out_path_2.to_str().unwrap(),
            ],
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(&out_path).unwrap(),
            std::fs::read(&out_path_2).unwrap()
        );

        let _ = std::fs::remove_file(&target_path);
        let _ = std::fs::remove_file(&catalog_path);
        let _ = std::fs::remove_file(&env_path);
        let _ = std::fs::remove_file(&out_path);
        let _ = std::fs::remove_file(&out_path_2);
    }

    fn write_data_environment_file(path: &Path, config: &DataEnvironmentConfig) {
        let value = json!({
            "tenantId": config.tenant_id.as_ref().map(|id| id.as_str()),
            "workspaceId": config.workspace_id.as_ref().map(|id| id.as_str()),
            "kv": match &config.kv {
                Some(KvStorageConfig::Memory) => json!({"kind": "memory"}),
                Some(KvStorageConfig::Sqlite { url, ensure_schema }) => {
                    json!({"kind": "sqlite", "url": url, "ensureSchema": ensure_schema})
                }
                Some(KvStorageConfig::Postgres { url, url_env, table, ensure_schema }) => {
                    json!({"kind": "postgres", "url": url, "urlEnv": url_env, "table": table, "ensureSchema": ensure_schema})
                }
                Some(KvStorageConfig::Redis { url, url_env, prefix }) => {
                    json!({"kind": "redis", "url": url, "urlEnv": url_env, "prefix": prefix})
                }
                None => serde_json::Value::Null,
            },
            "catalog": match &config.catalog {
                Some(CatalogStorageConfig::Sqlite { url, ensure_schema }) => {
                    json!({"kind": "sqlite", "url": url, "ensureSchema": ensure_schema})
                }
                Some(CatalogStorageConfig::Postgres { url, url_env, ensure_schema }) => {
                    json!({"kind": "postgres", "url": url, "urlEnv": url_env, "ensureSchema": ensure_schema})
                }
                Some(CatalogStorageConfig::Inline { entries }) => {
                    json!({"kind": "inline", "entries": entries})
                }
                Some(CatalogStorageConfig::Empty) => json!({"kind": "empty"}),
                None => serde_json::Value::Null,
            },
            "catalogSearch": match &config.catalog_search {
                Some(search) => {
                    json!({
                        "indexPath": search.index_path.to_string_lossy(),
                        "rebuildOnStart": search.rebuild_on_start,
                        "writeThrough": search.write_through
                    })
                }
                None => serde_json::Value::Null,
            },
            "targetDatabase": match &config.target_database {
                Some(TargetDatabaseStorageConfig::Sqlite { url }) => {
                    json!({"kind": "sqlite", "url": url})
                }
                Some(TargetDatabaseStorageConfig::Postgres { url, url_env, schema }) => {
                    json!({"kind": "postgres", "url": url, "urlEnv": url_env, "schema": schema})
                }
                None => serde_json::Value::Null,
            },
        });
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }

    fn default_data_scope() -> CatalogScope {
        CatalogScope::new(
            TenantId::new_unchecked(DEFAULT_DATA_TENANT_ID),
            WorkspaceId::default_workspace(),
        )
    }

    fn sqlite_url(path: &Path) -> String {
        format!("sqlite:{}", path.display())
    }

    fn temp_sqlite_path(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}.db", Uuid::new_v4()))
    }

    fn temp_json_path(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}.json", Uuid::new_v4()))
    }

    fn catalog_graph_table(name: &str) -> CatalogEntry {
        catalog_graph_table_with_database(name, "warehouse")
    }

    fn catalog_graph_table_with_database(name: &str, database_id: &str) -> CatalogEntry {
        CatalogEntry {
            id: format!("table:public.{name}"),
            kind: CatalogKind::Table,
            name: name.to_string(),
            qualified_name: Some(format!("public.{name}")),
            content: format!("{name} table"),
            tags: Vec::new(),
            links: Vec::new(),
            metadata: json!({
                "databaseId": database_id,
                "schemaName": "public",
                "tableName": name,
            }),
        }
    }

    fn temp_directory(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn seed_target_database(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE products (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                price REAL NOT NULL
            );
            INSERT INTO products (id, name, price) VALUES
                (1, 'Tea', 4.5),
                (2, 'Coffee', 7.0);

            CREATE TABLE orders (
                id INTEGER PRIMARY KEY,
                product_id INTEGER NOT NULL,
                quantity INTEGER NOT NULL,
                FOREIGN KEY(product_id) REFERENCES products(id)
            );
            INSERT INTO orders (id, product_id, quantity) VALUES
                (1, 1, 2),
                (2, 2, 1);
            "#,
        )
        .unwrap();
    }
}
