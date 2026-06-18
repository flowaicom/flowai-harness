use std::collections::HashSet;
use std::sync::Arc;

use agent_fw_algebra::{CancellationToken, KVStore};
use agent_fw_catalog::{CatalogScope, IngestionEvent, SemanticEnricher};
use agent_fw_core::{DatabaseType, TenantId, WorkspaceId};
use agent_fw_eval::{estimate_profiling_cost, ModelPricing, ModelPricingResolver};
use agent_fw_ingest::ingestion::{
    IngestionOrchestrator, ProfileDatabaseParams, ProfileTableParams,
};
use agent_fw_ingest::introspection::IntrospectionService;
use agent_fw_interpreter::DashMapKVStore;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{
    DataCommandError, OpenedProfilingEnvironment, ProfileDatabaseCommand, ProfileTableCommand,
    ProfilingEstimateCommand,
};

const DEFAULT_SAMPLE_SIZE: usize = 10;
const EVENT_BUFFER: usize = 256;
const SECS_PER_COLUMN: u64 = 2;

/// Host-provided dependencies for harness profiling commands.
#[derive(Clone)]
pub struct ProfilingCommandDeps {
    pub enricher: Arc<dyn SemanticEnricher>,
    pub kv: Arc<dyn KVStore>,
    pub pricing_resolver: Option<Arc<dyn ModelPricingResolver>>,
}

impl ProfilingCommandDeps {
    pub fn new(enricher: Arc<dyn SemanticEnricher>) -> Self {
        Self {
            enricher,
            kv: Arc::new(DashMapKVStore::new()),
            pricing_resolver: None,
        }
    }

    pub fn with_kv(mut self, kv: Arc<dyn KVStore>) -> Self {
        self.kv = kv;
        self
    }

    pub fn with_pricing_resolver(
        mut self,
        pricing_resolver: Arc<dyn ModelPricingResolver>,
    ) -> Self {
        self.pricing_resolver = Some(pricing_resolver);
        self
    }
}

/// Serializable profiling estimate for CLI / Python adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilingEstimateResult {
    pub table_count: usize,
    pub column_count: usize,
    pub estimated_input_tokens: u64,
    pub estimated_output_tokens: u64,
    pub estimated_cached_tokens: u64,
    pub estimated_cost_usd: Option<f64>,
    pub model_id: Option<String>,
    pub estimated_duration_secs: u64,
}

/// Foreground profiling run handle.
pub struct ProfilingRunHandle {
    pub job_id: String,
    pub events: mpsc::Receiver<IngestionEvent>,
}

/// Estimate a profiling run from the configured target database.
pub async fn estimate_profiling(
    command: &ProfilingEstimateCommand,
    deps: &ProfilingCommandDeps,
) -> Result<ProfilingEstimateResult, DataCommandError> {
    let _database_id = profiling_database_id(&command.database_id)?;
    let target_database =
        crate::storage::build_target_database_from_environment(&command.data_environment).await?;
    let schema = command
        .schema_name
        .as_deref()
        .unwrap_or(default_schema_for_database(target_database.database_type()));
    let introspection = IntrospectionService::new(target_database);
    let tables = introspection
        .list_tables(schema)
        .await
        .map_err(|err| DataCommandError::Execution(format!("failed to list tables: {err}")))?;
    let filtered = filter_tables(&tables, &command.tables)?;
    let table_count = filtered.len();
    let column_count = filtered
        .iter()
        .filter_map(|table| table.column_count)
        .map(|count| count as usize)
        .sum::<usize>();

    let (pricing, model_id) = match command.model_id.as_deref() {
        Some(model_id) => (
            deps.pricing_resolver
                .as_ref()
                .and_then(|resolver| resolver.resolve_model_pricing(model_id)),
            Some(model_id.to_string()),
        ),
        None => (None, None),
    };

    let estimate = estimate_profiling_cost(
        table_count as u32,
        column_count as u32,
        &pricing
            .clone()
            .unwrap_or_else(|| ModelPricing::new("unpriced", 0.0, 0.0, 0.0)),
    );

    Ok(ProfilingEstimateResult {
        table_count,
        column_count,
        estimated_input_tokens: estimate.input_tokens,
        estimated_output_tokens: estimate.output_tokens,
        estimated_cached_tokens: estimate.cached_tokens,
        estimated_cost_usd: pricing.map(|_| estimate.cost.total_usd),
        model_id,
        estimated_duration_secs: (column_count as u64) * SECS_PER_COLUMN,
    })
}

/// Profile a single table and stream `IngestionEvent`s.
pub async fn profile_table(
    command: ProfileTableCommand,
    deps: ProfilingCommandDeps,
) -> Result<ProfilingRunHandle, DataCommandError> {
    let database_id = profiling_database_id(&command.database_id)?;
    let scope = profile_catalog_scope(
        &command.data_environment,
        command.tenant_id.clone(),
        command.workspace_id.clone(),
    );
    let tenant_id = scope.tenant_id.to_string();
    let env = OpenedProfilingEnvironment::open_for_scope(&command.data_environment, scope).await?;
    let schema = command.schema_name.clone().unwrap_or_else(|| {
        default_schema_for_database(env.target_database.database_type()).to_string()
    });
    let sample_size = command.sample_size.unwrap_or(DEFAULT_SAMPLE_SIZE);
    let orchestrator = IngestionOrchestrator::new(
        env.target_database,
        deps.enricher,
        env.catalog.writer,
        deps.kv,
    );
    let (tx, rx) = mpsc::channel(EVENT_BUFFER);
    let job_id = format!("profile-table-{}", Uuid::new_v4().simple());
    let job_id_for_task = job_id.clone();
    let table_name = command.table_name;

    tokio::spawn(async move {
        let cancel = CancellationToken::new();
        orchestrator
            .profile_single_table(ProfileTableParams {
                tenant_id: &tenant_id,
                job_id: &job_id_for_task,
                profiling_run_id: None,
                schema: &schema,
                table: &table_name,
                database_id: &database_id,
                sample_size,
                tx: &tx,
                cancel: &cancel,
                database_context: None,
                fk_edges: &[],
            })
            .await;
    });

    Ok(ProfilingRunHandle { job_id, events: rx })
}

/// Profile a full database and stream `IngestionEvent`s.
pub async fn profile_database(
    command: ProfileDatabaseCommand,
    deps: ProfilingCommandDeps,
) -> Result<ProfilingRunHandle, DataCommandError> {
    let database_id = profiling_database_id(&command.database_id)?;
    let scope = profile_catalog_scope(
        &command.data_environment,
        command.tenant_id.clone(),
        command.workspace_id.clone(),
    );
    let tenant_id = scope.tenant_id.to_string();
    let env = OpenedProfilingEnvironment::open_for_scope(&command.data_environment, scope).await?;
    let schema = command.schema_name.clone().unwrap_or_else(|| {
        default_schema_for_database(env.target_database.database_type()).to_string()
    });
    let selected_tables = if command.tables.is_empty() {
        None
    } else {
        let introspection = IntrospectionService::new(Arc::clone(&env.target_database));
        let tables = introspection
            .list_tables(&schema)
            .await
            .map_err(|err| DataCommandError::Execution(format!("failed to list tables: {err}")))?;
        let _ = filter_tables(&tables, &command.tables)?;
        Some(command.tables)
    };
    let sample_size = command.sample_size.unwrap_or(DEFAULT_SAMPLE_SIZE);
    let orchestrator = IngestionOrchestrator::new(
        env.target_database,
        deps.enricher,
        env.catalog.writer,
        deps.kv,
    );
    let (tx, rx) = mpsc::channel(EVENT_BUFFER);
    let job_id = format!("profile-db-{}", Uuid::new_v4().simple());
    let job_id_for_task = job_id.clone();

    tokio::spawn(async move {
        let cancel = CancellationToken::new();
        orchestrator
            .profile_database_with_params(ProfileDatabaseParams {
                tenant_id: &tenant_id,
                job_id: &job_id_for_task,
                schema: &schema,
                database_id: &database_id,
                tx: &tx,
                cancel: &cancel,
                selected_tables: selected_tables.as_deref(),
                sample_size,
            })
            .await;
    });

    Ok(ProfilingRunHandle { job_id, events: rx })
}

fn profiling_database_id(database_id: &str) -> Result<String, DataCommandError> {
    let database_id = database_id.trim();
    if database_id.is_empty() {
        return Err(DataCommandError::Invalid(
            "catalog profiling database_id must not be blank".to_string(),
        ));
    }
    Ok(database_id.to_string())
}

fn profile_catalog_scope(
    config: &crate::storage::DataEnvironmentConfig,
    tenant_id: Option<TenantId>,
    workspace_id: Option<WorkspaceId>,
) -> CatalogScope {
    CatalogScope::new(
        tenant_id
            .or_else(|| config.tenant_id.clone())
            .unwrap_or_else(|| TenantId::new_unchecked(super::DEFAULT_DATA_TENANT_ID)),
        workspace_id
            .or_else(|| config.workspace_id.clone())
            .unwrap_or_else(WorkspaceId::default_workspace),
    )
}

fn filter_tables<'a>(
    tables: &'a [agent_fw_catalog::TableInfo],
    requested_tables: &[String],
) -> Result<Vec<&'a agent_fw_catalog::TableInfo>, DataCommandError> {
    if requested_tables.is_empty() {
        return Ok(tables.iter().collect());
    }

    let requested: HashSet<&str> = requested_tables.iter().map(String::as_str).collect();
    let filtered: Vec<_> = tables
        .iter()
        .filter(|table| requested.contains(table.table_name.as_str()))
        .collect();
    let found: HashSet<&str> = filtered
        .iter()
        .map(|table| table.table_name.as_str())
        .collect();
    let missing: Vec<_> = requested
        .difference(&found)
        .copied()
        .map(str::to_string)
        .collect();

    if !missing.is_empty() {
        return Err(DataCommandError::Invalid(format!(
            "requested tables not found in schema: {}",
            missing.join(", ")
        )));
    }

    Ok(filtered)
}

fn default_schema_for_database(database_type: DatabaseType) -> &'static str {
    match database_type {
        DatabaseType::SQLite => "main",
        DatabaseType::PostgreSQL | DatabaseType::MySQL => "public",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_eval::{ModelPricing, StaticPricingResolver};

    #[test]
    fn filter_tables_allows_full_schema() {
        let tables = vec![agent_fw_catalog::TableInfo {
            schema_name: "public".to_string(),
            table_name: "products".to_string(),
            table_type: agent_fw_catalog::TableType::BaseTable,
            row_count: Some(10),
            column_count: Some(3),
            description: None,
        }];

        let filtered = filter_tables(&tables, &[]).unwrap();
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn filter_tables_rejects_missing_requested_tables() {
        let tables = vec![agent_fw_catalog::TableInfo {
            schema_name: "public".to_string(),
            table_name: "products".to_string(),
            table_type: agent_fw_catalog::TableType::BaseTable,
            row_count: Some(10),
            column_count: Some(3),
            description: None,
        }];

        let err = filter_tables(&tables, &[String::from("orders")]).unwrap_err();
        assert!(err.to_string().contains("requested tables not found"));
    }

    #[test]
    fn profiling_command_deps_accepts_pricing_resolver() {
        let resolver = StaticPricingResolver::new([ModelPricing::new("stub", 1.0, 2.0, 0.0)]);
        let deps = ProfilingCommandDeps::new(Arc::new(agent_fw_interpreter::MockEnricher::new()))
            .with_pricing_resolver(Arc::new(resolver));
        assert!(deps.pricing_resolver.is_some());
    }
}
