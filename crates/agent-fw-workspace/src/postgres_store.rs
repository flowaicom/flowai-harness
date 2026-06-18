//! PostgreSQL workspace store for framework workspace entities.
//!
//! This interpreter owns the canonical relational persistence for framework
//! workspace entities that are generic across applications:
//! - threads
//! - messages
//! - test cases
//! - eval runs and results
//! - data sources
//! - workspaces

use sqlx::PgPool;

use crate::data_source::DataSource;
use crate::store::{
    DataSourceStore, EvalStore, MessageStore, TestCaseStore, ThreadStore, WorkspaceEntityStore,
    WorkspaceError,
};
use crate::thread::{Message, Thread};
use crate::workspace::{DatabaseConfig, Workspace, WorkspaceModelConfig};
use agent_fw_core::{TenantId, WorkspaceId};
use agent_fw_eval::{
    AuthoredTestCase, EvalRun, EvalStatus, EvalThreadFork, GroundTruth, TestCaseResult,
    TestCaseSet, TestCaseStatus, TrajectorySource, TrajectoryStep,
};

trait SqlxWorkspaceResultExt<T> {
    fn ws_db(self) -> Result<T, WorkspaceError>;
}

impl<T> SqlxWorkspaceResultExt<T> for Result<T, sqlx::Error> {
    fn ws_db(self) -> Result<T, WorkspaceError> {
        self.map_err(|e| WorkspaceError::Db(e.to_string()))
    }
}

trait SerdeWorkspaceResultExt<T> {
    fn ws_serde(self) -> Result<T, WorkspaceError>;
}

impl<T> SerdeWorkspaceResultExt<T> for Result<T, serde_json::Error> {
    fn ws_serde(self) -> Result<T, WorkspaceError> {
        self.map_err(|e| WorkspaceError::Serde(e.to_string()))
    }
}

const TABLE_DEFINITIONS: &[&str] = &[
    r#"CREATE TABLE IF NOT EXISTS threads (
        id          TEXT PRIMARY KEY,
        tenant_id   TEXT NOT NULL,
        title       TEXT,
        source_id   TEXT,
        created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )"#,
    r#"CREATE TABLE IF NOT EXISTS messages (
        id                  TEXT PRIMARY KEY,
        thread_id           TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
        role                TEXT NOT NULL,
        content             TEXT NOT NULL DEFAULT '',
        tool_interactions   JSONB,
        parts               JSONB,
        created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )"#,
    r#"CREATE TABLE IF NOT EXISTS data_sources (
        id                      TEXT PRIMARY KEY,
        tenant_id               TEXT NOT NULL,
        name                    TEXT NOT NULL,
        database_type           TEXT NOT NULL,
        host                    TEXT NOT NULL DEFAULT '',
        port                    INTEGER NOT NULL DEFAULT 5432,
        database_name           TEXT NOT NULL DEFAULT '',
        schema_name             TEXT NOT NULL DEFAULT 'public',
        encrypted_credentials   TEXT,
        is_active               BOOLEAN NOT NULL DEFAULT TRUE,
        created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )"#,
    r#"CREATE TABLE IF NOT EXISTS workspaces (
        id                  TEXT NOT NULL,
        tenant_id           TEXT NOT NULL,
        name                TEXT NOT NULL,
        slug                TEXT NOT NULL,
        description         TEXT,
        database_config     JSONB NOT NULL DEFAULT '{"type":"default"}',
        model_config        JSONB NOT NULL DEFAULT '{}',
        created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        PRIMARY KEY (id),
        UNIQUE (tenant_id, slug)
    )"#,
    r#"CREATE TABLE IF NOT EXISTS test_cases (
        id                      TEXT PRIMARY KEY,
        tenant_id               TEXT NOT NULL,
        input                   TEXT NOT NULL,
        status                  TEXT NOT NULL DEFAULT 'draft',
        tags                    TEXT[] NOT NULL DEFAULT '{}',
        expected_trajectory     JSONB NOT NULL DEFAULT '[]',
        trajectory_sources      JSONB NOT NULL DEFAULT '[]',
        trajectory_mode         TEXT NOT NULL DEFAULT 'anyOrder',
        structured_ground_truth JSONB,
        source_thread_id        TEXT,
        source_session_id       TEXT,
        tag_warnings            TEXT[] NOT NULL DEFAULT '{}',
        created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )"#,
    r#"CREATE TABLE IF NOT EXISTS eval_runs (
        id                  TEXT PRIMARY KEY,
        tenant_id           TEXT NOT NULL,
        config              JSONB NOT NULL,
        status              JSONB NOT NULL DEFAULT '{"status":"queued"}',
        parent_run_id       TEXT,
        rerun_test_case_ids JSONB,
        created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )"#,
    r#"CREATE TABLE IF NOT EXISTS eval_results (
        id              BIGSERIAL PRIMARY KEY,
        eval_run_id     TEXT NOT NULL REFERENCES eval_runs(id) ON DELETE CASCADE,
        test_case_id    TEXT NOT NULL,
        result          JSONB NOT NULL,
        created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )"#,
    r#"CREATE TABLE IF NOT EXISTS eval_test_case_sets (
        id              TEXT PRIMARY KEY,
        tenant_id       TEXT NOT NULL,
        name            TEXT NOT NULL,
        description     TEXT NOT NULL DEFAULT '',
        test_cases      JSONB NOT NULL DEFAULT '[]',
        created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )"#,
    r#"CREATE TABLE IF NOT EXISTS eval_forks (
        id                      TEXT PRIMARY KEY,
        tenant_id               TEXT NOT NULL,
        eval_run_id             TEXT NOT NULL,
        test_case_id            TEXT NOT NULL,
        thread_id               TEXT NOT NULL,
        parent_thread_id        TEXT,
        fork_at_message_index   INTEGER,
        edited_content          TEXT,
        label                   TEXT,
        created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )"#,
];

const INDEX_DEFINITIONS: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS threads_tenant_idx ON threads(tenant_id, created_at DESC)",
    "CREATE INDEX IF NOT EXISTS messages_thread_idx ON messages(thread_id, created_at)",
    "CREATE INDEX IF NOT EXISTS data_sources_tenant_idx ON data_sources(tenant_id)",
    "CREATE INDEX IF NOT EXISTS workspaces_tenant_idx ON workspaces(tenant_id)",
    "CREATE INDEX IF NOT EXISTS test_cases_tenant_idx ON test_cases(tenant_id, created_at DESC)",
    "CREATE INDEX IF NOT EXISTS eval_runs_tenant_idx ON eval_runs(tenant_id, created_at DESC)",
    "CREATE INDEX IF NOT EXISTS eval_results_run_idx ON eval_results(eval_run_id, test_case_id)",
    "CREATE INDEX IF NOT EXISTS eval_test_case_sets_tenant_idx ON eval_test_case_sets(tenant_id, created_at DESC)",
    "CREATE INDEX IF NOT EXISTS eval_forks_lookup_idx ON eval_forks(tenant_id, eval_run_id, test_case_id, created_at)",
];

#[derive(Clone)]
pub struct PostgresWorkspaceStore {
    pool: PgPool,
}

impl PostgresWorkspaceStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn connect(url: &str) -> Result<Self, WorkspaceError> {
        let pool = PgPool::connect(url).await.ws_db()?;
        let store = Self { pool };
        store.ensure_schema().await?;
        Ok(store)
    }

    pub async fn ensure_schema(&self) -> Result<(), WorkspaceError> {
        for ddl in TABLE_DEFINITIONS {
            sqlx::query(ddl).execute(&self.pool).await.ws_db()?;
        }
        for ddl in INDEX_DEFINITIONS {
            sqlx::query(ddl).execute(&self.pool).await.ws_db()?;
        }
        self.run_column_migrations().await?;
        Ok(())
    }

    /// Additive column migrations for tables created before new columns were added.
    /// Each statement uses `IF NOT EXISTS` so it is safe to run repeatedly.
    async fn run_column_migrations(&self) -> Result<(), WorkspaceError> {
        let migrations: &[&str] = &[
            "ALTER TABLE threads ADD COLUMN IF NOT EXISTS source_id TEXT",
            "ALTER TABLE test_cases ADD COLUMN IF NOT EXISTS tag_warnings TEXT[] NOT NULL DEFAULT '{}'",
        ];
        for ddl in migrations {
            sqlx::query(ddl).execute(&self.pool).await.ws_db()?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl ThreadStore for PostgresWorkspaceStore {
    async fn list_threads(&self, tenant: &TenantId) -> Result<Vec<Thread>, WorkspaceError> {
        let rows = sqlx::query_as::<_, ThreadRow>(
            "SELECT id, tenant_id, title, source_id, created_at, updated_at FROM threads WHERE tenant_id = $1 ORDER BY updated_at DESC",
        )
        .bind(tenant.as_str())
        .fetch_all(&self.pool)
        .await
        .ws_db()?;
        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    async fn get_thread(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<Thread>, WorkspaceError> {
        let row = sqlx::query_as::<_, ThreadRow>(
            "SELECT id, tenant_id, title, source_id, created_at, updated_at FROM threads WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .ws_db()?;
        Ok(row.map(Into::into))
    }

    async fn upsert_thread(
        &self,
        tenant: &TenantId,
        thread: &Thread,
    ) -> Result<(), WorkspaceError> {
        sqlx::query(
            r#"
            INSERT INTO threads (id, tenant_id, title, source_id, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5::timestamptz, $6::timestamptz)
            ON CONFLICT (id) DO UPDATE SET
                title = EXCLUDED.title,
                source_id = EXCLUDED.source_id,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(&thread.id)
        .bind(tenant.as_str())
        .bind(&thread.title)
        .bind(&thread.source_id)
        .bind(&thread.created_at)
        .bind(&thread.updated_at)
        .execute(&self.pool)
        .await
        .ws_db()?;
        Ok(())
    }

    async fn delete_thread(&self, tenant: &TenantId, id: &str) -> Result<(), WorkspaceError> {
        sqlx::query("DELETE FROM threads WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant.as_str())
            .execute(&self.pool)
            .await
            .ws_db()?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl MessageStore for PostgresWorkspaceStore {
    async fn insert_message(
        &self,
        tenant: &TenantId,
        message: &Message,
        thread_id: &str,
    ) -> Result<(), WorkspaceError> {
        let tool_interactions = message
            .tool_interactions
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .ws_serde()?;
        let parts = message
            .parts
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .ws_serde()?;

        sqlx::query(
            r#"
            INSERT INTO messages (id, thread_id, role, content, tool_interactions, parts, created_at)
            SELECT $1, t.id, $3, $4, $5, $6, $7::timestamptz
            FROM threads t
            WHERE t.id = $2 AND t.tenant_id = $8
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(&message.id)
        .bind(thread_id)
        .bind(&message.role)
        .bind(&message.content)
        .bind(&tool_interactions)
        .bind(&parts)
        .bind(&message.created_at)
        .bind(tenant.as_str())
        .execute(&self.pool)
        .await
        .ws_db()?;
        Ok(())
    }

    async fn get_messages(
        &self,
        tenant: &TenantId,
        thread_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Message>, WorkspaceError> {
        let rows = sqlx::query_as::<_, MessageRow>(
            "SELECT m.id, m.role, m.content, m.tool_interactions, m.parts, m.created_at \
             FROM messages m \
             INNER JOIN threads t ON m.thread_id = t.id AND t.tenant_id = $4 \
             WHERE m.thread_id = $1 \
             ORDER BY m.created_at \
             LIMIT $2 OFFSET $3",
        )
        .bind(thread_id)
        .bind(limit as i64)
        .bind(offset as i64)
        .bind(tenant.as_str())
        .fetch_all(&self.pool)
        .await
        .ws_db()?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_all_messages(
        &self,
        tenant: &TenantId,
        thread_id: &str,
    ) -> Result<Vec<Message>, WorkspaceError> {
        let rows = sqlx::query_as::<_, MessageRow>(
            "SELECT m.id, m.role, m.content, m.tool_interactions, m.parts, m.created_at \
             FROM messages m \
             INNER JOIN threads t ON m.thread_id = t.id AND t.tenant_id = $2 \
             WHERE m.thread_id = $1 \
             ORDER BY m.created_at",
        )
        .bind(thread_id)
        .bind(tenant.as_str())
        .fetch_all(&self.pool)
        .await
        .ws_db()?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_recent_messages(
        &self,
        tenant: &TenantId,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<Message>, WorkspaceError> {
        let mut rows = sqlx::query_as::<_, MessageRow>(
            "SELECT m.id, m.role, m.content, m.tool_interactions, m.parts, m.created_at \
             FROM messages m \
             INNER JOIN threads t ON m.thread_id = t.id AND t.tenant_id = $3 \
             WHERE m.thread_id = $1 \
             ORDER BY m.created_at DESC \
             LIMIT $2",
        )
        .bind(thread_id)
        .bind(limit as i64)
        .bind(tenant.as_str())
        .fetch_all(&self.pool)
        .await
        .ws_db()?;
        rows.reverse();
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get_all_messages_batch(
        &self,
        tenant: &TenantId,
        thread_ids: &[String],
    ) -> Result<Vec<Vec<Message>>, WorkspaceError> {
        if thread_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query_as::<_, BatchMessageRow>(
            "SELECT m.thread_id, m.id, m.role, m.content, m.tool_interactions, m.parts, \
             m.created_at \
             FROM messages m \
             INNER JOIN threads t ON m.thread_id = t.id AND t.tenant_id = $2 \
             WHERE m.thread_id = ANY($1) \
             ORDER BY m.thread_id, m.created_at",
        )
        .bind(thread_ids)
        .bind(tenant.as_str())
        .fetch_all(&self.pool)
        .await
        .ws_db()?;

        let mut grouped: std::collections::HashMap<String, Vec<Message>> =
            std::collections::HashMap::with_capacity(thread_ids.len());
        for row in rows {
            grouped
                .entry(row.thread_id.clone())
                .or_default()
                .push(row.into_message());
        }

        Ok(thread_ids
            .iter()
            .map(|thread_id| grouped.remove(thread_id).unwrap_or_default())
            .collect())
    }
}

#[async_trait::async_trait]
impl TestCaseStore for PostgresWorkspaceStore {
    async fn list_test_cases(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<AuthoredTestCase>, WorkspaceError> {
        let rows = sqlx::query_as::<_, TestCaseRow>(
            "SELECT id, input, status, tags, expected_trajectory, trajectory_sources, \
             trajectory_mode, structured_ground_truth, source_thread_id, source_session_id, \
             tag_warnings, created_at, updated_at \
             FROM test_cases WHERE tenant_id = $1 ORDER BY created_at DESC",
        )
        .bind(tenant.as_str())
        .fetch_all(&self.pool)
        .await
        .ws_db()?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn get_test_case(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<AuthoredTestCase>, WorkspaceError> {
        let row = sqlx::query_as::<_, TestCaseRow>(
            "SELECT id, input, status, tags, expected_trajectory, trajectory_sources, \
             trajectory_mode, structured_ground_truth, source_thread_id, source_session_id, \
             tag_warnings, created_at, updated_at \
             FROM test_cases WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .ws_db()?;
        row.map(TryInto::try_into).transpose()
    }

    async fn get_test_cases_by_id(
        &self,
        tenant: &TenantId,
        ids: &[String],
    ) -> Result<Vec<Option<AuthoredTestCase>>, WorkspaceError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query_as::<_, TestCaseRow>(
            "SELECT id, input, status, tags, expected_trajectory, trajectory_sources, \
             trajectory_mode, structured_ground_truth, source_thread_id, source_session_id, \
             tag_warnings, created_at, updated_at \
             FROM test_cases WHERE tenant_id = $1 AND id = ANY($2)",
        )
        .bind(tenant.as_str())
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .ws_db()?;

        let mut cases_by_id = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            let id = row.id.clone();
            cases_by_id.insert(id, row.try_into()?);
        }

        Ok(ids.iter().map(|id| cases_by_id.remove(id)).collect())
    }

    async fn insert_test_case(
        &self,
        tenant: &TenantId,
        tc: &AuthoredTestCase,
    ) -> Result<(), WorkspaceError> {
        let expected_trajectory = serde_json::to_value(&tc.expected_trajectory).ws_serde()?;
        let trajectory_sources = serde_json::to_value(&tc.trajectory_sources).ws_serde()?;
        let structured_ground_truth = tc
            .structured_ground_truth
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .ws_serde()?;

        sqlx::query(
            r#"
            INSERT INTO test_cases (
                id, tenant_id, input, status, tags, expected_trajectory, trajectory_sources,
                trajectory_mode, structured_ground_truth, source_thread_id, source_session_id,
                tag_warnings, created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13::timestamptz, $14::timestamptz
            )
            ON CONFLICT (id) DO UPDATE SET
                input = EXCLUDED.input,
                status = EXCLUDED.status,
                tags = EXCLUDED.tags,
                expected_trajectory = EXCLUDED.expected_trajectory,
                trajectory_sources = EXCLUDED.trajectory_sources,
                trajectory_mode = EXCLUDED.trajectory_mode,
                structured_ground_truth = EXCLUDED.structured_ground_truth,
                source_thread_id = EXCLUDED.source_thread_id,
                source_session_id = EXCLUDED.source_session_id,
                tag_warnings = EXCLUDED.tag_warnings,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(tc.id.as_str())
        .bind(tenant.as_str())
        .bind(&tc.input)
        .bind(tc.status.as_str())
        .bind(&tc.tags)
        .bind(&expected_trajectory)
        .bind(&trajectory_sources)
        .bind(enum_to_str(&tc.trajectory_mode)?)
        .bind(&structured_ground_truth)
        .bind(&tc.source_thread_id)
        .bind(&tc.source_session_id)
        .bind(&tc.tag_warnings)
        .bind(&tc.created_at)
        .bind(&tc.updated_at)
        .execute(&self.pool)
        .await
        .ws_db()?;
        Ok(())
    }

    async fn update_test_case(
        &self,
        tenant: &TenantId,
        tc: &AuthoredTestCase,
    ) -> Result<(), WorkspaceError> {
        self.insert_test_case(tenant, tc).await
    }

    async fn delete_test_case(&self, tenant: &TenantId, id: &str) -> Result<(), WorkspaceError> {
        sqlx::query("DELETE FROM test_cases WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant.as_str())
            .execute(&self.pool)
            .await
            .ws_db()?;
        Ok(())
    }

    async fn batch_update_status(
        &self,
        tenant: &TenantId,
        ids: &[String],
        status: TestCaseStatus,
    ) -> Result<u64, WorkspaceError> {
        let result = sqlx::query(
            "UPDATE test_cases SET status = $1, updated_at = NOW() WHERE id = ANY($2) AND tenant_id = $3",
        )
        .bind(status.as_str())
        .bind(ids)
        .bind(tenant.as_str())
        .execute(&self.pool)
        .await
        .ws_db()?;
        Ok(result.rows_affected())
    }

    async fn batch_delete_test_cases(
        &self,
        tenant: &TenantId,
        ids: &[String],
    ) -> Result<u64, WorkspaceError> {
        let result = sqlx::query("DELETE FROM test_cases WHERE id = ANY($1) AND tenant_id = $2")
            .bind(ids)
            .bind(tenant.as_str())
            .execute(&self.pool)
            .await
            .ws_db()?;
        Ok(result.rows_affected())
    }
}

#[async_trait::async_trait]
impl EvalStore for PostgresWorkspaceStore {
    async fn insert_eval_run(
        &self,
        tenant: &TenantId,
        run: &EvalRun,
    ) -> Result<(), WorkspaceError> {
        let config = serde_json::to_value(&run.config).ws_serde()?;
        let status = serde_json::to_value(&run.status).ws_serde()?;
        let rerun_test_case_ids = run
            .rerun_test_case_ids
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .ws_serde()?;

        sqlx::query(
            r#"
            INSERT INTO eval_runs (id, tenant_id, config, status, parent_run_id, rerun_test_case_ids, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7::timestamptz, $8::timestamptz)
            ON CONFLICT (id) DO UPDATE SET
                status = EXCLUDED.status,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(run.id.as_str())
        .bind(tenant.as_str())
        .bind(&config)
        .bind(&status)
        .bind(run.parent_run_id.as_ref().map(|id| id.as_str()))
        .bind(&rerun_test_case_ids)
        .bind(&run.created_at)
        .bind(&run.updated_at)
        .execute(&self.pool)
        .await
        .ws_db()?;
        Ok(())
    }

    async fn get_eval_run(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<EvalRun>, WorkspaceError> {
        let row = sqlx::query_as::<_, EvalRunRow>(
            "SELECT id, config, status, parent_run_id, rerun_test_case_ids, created_at, updated_at \
             FROM eval_runs WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .ws_db()?;
        row.map(TryInto::try_into).transpose()
    }

    async fn update_eval_status(
        &self,
        tenant: &TenantId,
        id: &str,
        status: &EvalStatus,
    ) -> Result<(), WorkspaceError> {
        let status_json = serde_json::to_value(status).ws_serde()?;
        sqlx::query(
            "UPDATE eval_runs SET status = $1, updated_at = NOW() WHERE id = $2 AND tenant_id = $3",
        )
        .bind(&status_json)
        .bind(id)
        .bind(tenant.as_str())
        .execute(&self.pool)
        .await
        .ws_db()?;
        Ok(())
    }

    async fn list_eval_runs(&self, tenant: &TenantId) -> Result<Vec<EvalRun>, WorkspaceError> {
        let rows = sqlx::query_as::<_, EvalRunRow>(
            "SELECT id, config, status, parent_run_id, rerun_test_case_ids, created_at, updated_at \
             FROM eval_runs WHERE tenant_id = $1 ORDER BY created_at DESC",
        )
        .bind(tenant.as_str())
        .fetch_all(&self.pool)
        .await
        .ws_db()?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn delete_eval_run(&self, tenant: &TenantId, id: &str) -> Result<(), WorkspaceError> {
        sqlx::query("DELETE FROM eval_runs WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant.as_str())
            .execute(&self.pool)
            .await
            .ws_db()?;
        Ok(())
    }

    async fn insert_eval_result(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
        test_case_id: &str,
        result: &TestCaseResult,
    ) -> Result<(), WorkspaceError> {
        let result_json = serde_json::to_value(result).ws_serde()?;
        let inserted = sqlx::query(
            r#"
            INSERT INTO eval_results (eval_run_id, test_case_id, result)
            SELECT r.id, $2, $3
            FROM eval_runs r
            WHERE r.id = $1 AND r.tenant_id = $4
            "#,
        )
        .bind(eval_run_id)
        .bind(test_case_id)
        .bind(&result_json)
        .bind(tenant.as_str())
        .execute(&self.pool)
        .await
        .ws_db()?;

        if inserted.rows_affected() == 0 {
            return Err(WorkspaceError::Db(format!(
                "insert_eval_result: eval run {eval_run_id} not found for tenant {}",
                tenant.as_str()
            )));
        }
        Ok(())
    }

    async fn get_eval_results(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
    ) -> Result<Vec<TestCaseResult>, WorkspaceError> {
        let rows = sqlx::query_as::<_, EvalResultRow>(
            "SELECT er.result \
             FROM eval_results er \
             INNER JOIN eval_runs r ON er.eval_run_id = r.id AND r.tenant_id = $2 \
             WHERE er.eval_run_id = $1 \
             ORDER BY er.created_at",
        )
        .bind(eval_run_id)
        .bind(tenant.as_str())
        .fetch_all(&self.pool)
        .await
        .ws_db()?;
        rows.into_iter()
            .map(|row| serde_json::from_value(row.result).ws_serde())
            .collect()
    }

    async fn upsert_eval_test_case_set(
        &self,
        tenant: &TenantId,
        set: &TestCaseSet,
    ) -> Result<(), WorkspaceError> {
        let test_cases = serde_json::to_value(&set.test_cases).ws_serde()?;
        sqlx::query(
            r#"
            INSERT INTO eval_test_case_sets (
                id, tenant_id, name, description, test_cases, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6::timestamptz, NOW())
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                description = EXCLUDED.description,
                test_cases = EXCLUDED.test_cases,
                updated_at = NOW()
            "#,
        )
        .bind(&set.id)
        .bind(tenant.as_str())
        .bind(&set.name)
        .bind(&set.description)
        .bind(&test_cases)
        .bind(&set.created_at)
        .execute(&self.pool)
        .await
        .ws_db()?;
        Ok(())
    }

    async fn get_eval_test_case_set(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<TestCaseSet>, WorkspaceError> {
        let row = sqlx::query_as::<_, EvalTestCaseSetRow>(
            "SELECT id, name, description, test_cases, created_at \
             FROM eval_test_case_sets WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .ws_db()?;
        row.map(TryInto::try_into).transpose()
    }

    async fn list_eval_test_case_sets(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<TestCaseSet>, WorkspaceError> {
        let rows = sqlx::query_as::<_, EvalTestCaseSetRow>(
            "SELECT id, name, description, test_cases, created_at \
             FROM eval_test_case_sets WHERE tenant_id = $1 ORDER BY created_at DESC",
        )
        .bind(tenant.as_str())
        .fetch_all(&self.pool)
        .await
        .ws_db()?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn delete_eval_test_case_set(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<(), WorkspaceError> {
        sqlx::query("DELETE FROM eval_test_case_sets WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant.as_str())
            .execute(&self.pool)
            .await
            .ws_db()?;
        Ok(())
    }

    async fn append_eval_thread_fork(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
        test_case_id: &str,
        fork: &EvalThreadFork,
    ) -> Result<(), WorkspaceError> {
        sqlx::query(
            r#"
            INSERT INTO eval_forks (
                id, tenant_id, eval_run_id, test_case_id, thread_id, parent_thread_id,
                fork_at_message_index, edited_content, label, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::timestamptz)
            "#,
        )
        .bind(&fork.id)
        .bind(tenant.as_str())
        .bind(eval_run_id)
        .bind(test_case_id)
        .bind(&fork.thread_id)
        .bind(&fork.parent_thread_id)
        .bind(fork.fork_at_message_index.map(|value| value as i32))
        .bind(&fork.edited_content)
        .bind(&fork.label)
        .bind(&fork.created_at)
        .execute(&self.pool)
        .await
        .ws_db()?;
        Ok(())
    }

    async fn list_eval_thread_forks(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
        test_case_id: &str,
    ) -> Result<Vec<EvalThreadFork>, WorkspaceError> {
        let rows = sqlx::query_as::<_, EvalForkRow>(
            "SELECT id, thread_id, parent_thread_id, fork_at_message_index, edited_content, label, created_at \
             FROM eval_forks \
             WHERE tenant_id = $1 AND eval_run_id = $2 AND test_case_id = $3 \
             ORDER BY created_at",
        )
        .bind(tenant.as_str())
        .bind(eval_run_id)
        .bind(test_case_id)
        .fetch_all(&self.pool)
        .await
        .ws_db()?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn delete_eval_thread_fork(
        &self,
        tenant: &TenantId,
        eval_run_id: &str,
        test_case_id: &str,
        fork_id: &str,
    ) -> Result<bool, WorkspaceError> {
        let result = sqlx::query(
            "DELETE FROM eval_forks \
             WHERE tenant_id = $1 AND eval_run_id = $2 AND test_case_id = $3 AND id = $4",
        )
        .bind(tenant.as_str())
        .bind(eval_run_id)
        .bind(test_case_id)
        .bind(fork_id)
        .execute(&self.pool)
        .await
        .ws_db()?;
        Ok(result.rows_affected() > 0)
    }
}

#[async_trait::async_trait]
impl DataSourceStore for PostgresWorkspaceStore {
    async fn list_data_sources(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<DataSource>, WorkspaceError> {
        let rows = sqlx::query_as::<_, DataSourceRow>(
            "SELECT id, name, database_type, host, port, database_name, schema_name, \
             encrypted_credentials, is_active, created_at, updated_at \
             FROM data_sources WHERE tenant_id = $1 ORDER BY created_at DESC",
        )
        .bind(tenant.as_str())
        .fetch_all(&self.pool)
        .await
        .ws_db()?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn get_data_source(
        &self,
        tenant: &TenantId,
        id: &str,
    ) -> Result<Option<DataSource>, WorkspaceError> {
        let row = sqlx::query_as::<_, DataSourceRow>(
            "SELECT id, name, database_type, host, port, database_name, schema_name, \
             encrypted_credentials, is_active, created_at, updated_at \
             FROM data_sources WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .ws_db()?;
        row.map(TryInto::try_into).transpose()
    }

    async fn upsert_data_source(
        &self,
        tenant: &TenantId,
        ds: &DataSource,
    ) -> Result<(), WorkspaceError> {
        sqlx::query(
            r#"
            INSERT INTO data_sources (
                id, tenant_id, name, database_type, host, port, database_name,
                schema_name, encrypted_credentials, is_active, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::timestamptz, $12::timestamptz)
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                database_type = EXCLUDED.database_type,
                host = EXCLUDED.host,
                port = EXCLUDED.port,
                database_name = EXCLUDED.database_name,
                schema_name = EXCLUDED.schema_name,
                encrypted_credentials = EXCLUDED.encrypted_credentials,
                is_active = EXCLUDED.is_active,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(&ds.id)
        .bind(tenant.as_str())
        .bind(&ds.name)
        .bind(enum_to_str(&ds.database_type)?)
        .bind(&ds.host)
        .bind(ds.port as i32)
        .bind(&ds.database_name)
        .bind(&ds.schema_name)
        .bind(&ds.encrypted_credentials)
        .bind(ds.is_active)
        .bind(&ds.created_at)
        .bind(&ds.updated_at)
        .execute(&self.pool)
        .await
        .ws_db()?;
        Ok(())
    }

    async fn delete_data_source(&self, tenant: &TenantId, id: &str) -> Result<(), WorkspaceError> {
        sqlx::query("DELETE FROM data_sources WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant.as_str())
            .execute(&self.pool)
            .await
            .ws_db()?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl WorkspaceEntityStore for PostgresWorkspaceStore {
    async fn list_workspaces(&self, tenant: &TenantId) -> Result<Vec<Workspace>, WorkspaceError> {
        let rows = sqlx::query_as::<_, WorkspaceRow>(
            "SELECT id, tenant_id, name, slug, description, database_config, model_config, created_at, updated_at \
             FROM workspaces WHERE tenant_id = $1 ORDER BY created_at DESC",
        )
        .bind(tenant.as_str())
        .fetch_all(&self.pool)
        .await
        .ws_db()?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn get_workspace(
        &self,
        tenant: &TenantId,
        workspace_id: &str,
    ) -> Result<Option<Workspace>, WorkspaceError> {
        let row = sqlx::query_as::<_, WorkspaceRow>(
            "SELECT id, tenant_id, name, slug, description, database_config, model_config, created_at, updated_at \
             FROM workspaces WHERE id = $1 AND tenant_id = $2",
        )
        .bind(workspace_id)
        .bind(tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .ws_db()?;
        row.map(TryInto::try_into).transpose()
    }

    async fn create_workspace(
        &self,
        tenant: &TenantId,
        workspace: &Workspace,
    ) -> Result<(), WorkspaceError> {
        let db_config = serde_json::to_value(&workspace.database_config).ws_serde()?;
        let model_config = serde_json::to_value(&workspace.model_config).ws_serde()?;

        sqlx::query(
            r#"INSERT INTO workspaces (id, tenant_id, name, slug, description, database_config, model_config, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               ON CONFLICT (id) DO UPDATE SET
                   name = EXCLUDED.name,
                   slug = EXCLUDED.slug,
                   description = EXCLUDED.description,
                   database_config = EXCLUDED.database_config,
                   model_config = EXCLUDED.model_config,
                   updated_at = EXCLUDED.updated_at"#,
        )
        .bind(workspace.id.as_str())
        .bind(tenant.as_str())
        .bind(&workspace.name)
        .bind(&workspace.slug)
        .bind(&workspace.description)
        .bind(&db_config)
        .bind(&model_config)
        .bind(&workspace.created_at)
        .bind(&workspace.updated_at)
        .execute(&self.pool)
        .await
        .ws_db()?;
        Ok(())
    }

    async fn update_workspace(
        &self,
        tenant: &TenantId,
        workspace: &Workspace,
    ) -> Result<(), WorkspaceError> {
        let db_config = serde_json::to_value(&workspace.database_config).ws_serde()?;
        let model_config = serde_json::to_value(&workspace.model_config).ws_serde()?;

        sqlx::query(
            r#"UPDATE workspaces SET
                   name = $1,
                   description = $2,
                   database_config = $3,
                   model_config = $4,
                   updated_at = NOW()
               WHERE id = $5 AND tenant_id = $6"#,
        )
        .bind(&workspace.name)
        .bind(&workspace.description)
        .bind(&db_config)
        .bind(&model_config)
        .bind(workspace.id.as_str())
        .bind(tenant.as_str())
        .execute(&self.pool)
        .await
        .ws_db()?;
        Ok(())
    }

    async fn delete_workspace(
        &self,
        tenant: &TenantId,
        workspace_id: &str,
    ) -> Result<(), WorkspaceError> {
        sqlx::query("DELETE FROM workspaces WHERE id = $1 AND tenant_id = $2")
            .bind(workspace_id)
            .bind(tenant.as_str())
            .execute(&self.pool)
            .await
            .ws_db()?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct ThreadRow {
    id: String,
    tenant_id: String,
    title: Option<String>,
    source_id: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<ThreadRow> for Thread {
    fn from(row: ThreadRow) -> Self {
        Thread {
            id: row.id,
            title: row.title,
            resource_id: row.tenant_id,
            source_id: row.source_id,
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
        }
    }
}

#[derive(sqlx::FromRow)]
struct MessageRow {
    id: String,
    role: String,
    content: String,
    tool_interactions: Option<serde_json::Value>,
    parts: Option<serde_json::Value>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<MessageRow> for Message {
    fn from(row: MessageRow) -> Self {
        let tool_interactions = row
            .tool_interactions
            .and_then(|v| serde_json::from_value(v).ok());
        let parts = row.parts.and_then(|v| serde_json::from_value(v).ok());
        Message {
            id: row.id,
            role: row.role,
            content: row.content,
            created_at: row.created_at.to_rfc3339(),
            tool_interactions,
            parts,
        }
    }
}

#[derive(sqlx::FromRow)]
struct BatchMessageRow {
    thread_id: String,
    id: String,
    role: String,
    content: String,
    tool_interactions: Option<serde_json::Value>,
    parts: Option<serde_json::Value>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl BatchMessageRow {
    fn into_message(self) -> Message {
        Message {
            id: self.id,
            role: self.role,
            content: self.content,
            created_at: self.created_at.to_rfc3339(),
            tool_interactions: self
                .tool_interactions
                .and_then(|value| serde_json::from_value(value).ok()),
            parts: self
                .parts
                .and_then(|value| serde_json::from_value(value).ok()),
        }
    }
}

fn parse_jsonb<T: serde::de::DeserializeOwned>(
    value: Option<serde_json::Value>,
) -> Result<Option<T>, WorkspaceError> {
    value.map(serde_json::from_value).transpose().ws_serde()
}

fn parse_jsonb_or_default<T: serde::de::DeserializeOwned + Default>(
    value: Option<serde_json::Value>,
) -> Result<T, WorkspaceError> {
    match value {
        Some(v) => serde_json::from_value(v).ws_serde(),
        None => Ok(T::default()),
    }
}

#[derive(sqlx::FromRow)]
struct TestCaseRow {
    id: String,
    input: String,
    #[sqlx(try_from = "String")]
    status: TestCaseStatus,
    tags: Option<Vec<String>>,
    expected_trajectory: serde_json::Value,
    trajectory_sources: Option<serde_json::Value>,
    #[sqlx(try_from = "String")]
    trajectory_mode: agent_fw_eval::TrajectoryMode,
    structured_ground_truth: Option<serde_json::Value>,
    source_thread_id: Option<String>,
    source_session_id: Option<String>,
    tag_warnings: Option<Vec<String>>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<TestCaseRow> for AuthoredTestCase {
    type Error = WorkspaceError;

    fn try_from(row: TestCaseRow) -> Result<Self, Self::Error> {
        let id = agent_fw_core::TestCaseId::new(row.id)
            .ok_or_else(|| WorkspaceError::Serde("test case ID is empty".to_string()))?;
        let expected_trajectory: Vec<TrajectoryStep> =
            serde_json::from_value(row.expected_trajectory).ws_serde()?;
        let trajectory_sources: Vec<TrajectorySource> =
            parse_jsonb_or_default(row.trajectory_sources)?;
        let structured_ground_truth: Option<GroundTruth> =
            parse_jsonb(row.structured_ground_truth)?;

        Ok(AuthoredTestCase {
            id,
            input: row.input,
            status: row.status,
            tags: row.tags.unwrap_or_default(),
            expected_trajectory,
            trajectory_sources,
            trajectory_mode: row.trajectory_mode,
            structured_ground_truth,
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
            source_thread_id: row.source_thread_id,
            source_session_id: row.source_session_id,
            tag_warnings: row.tag_warnings.unwrap_or_default(),
        })
    }
}

#[derive(sqlx::FromRow)]
struct EvalRunRow {
    id: String,
    config: serde_json::Value,
    status: serde_json::Value,
    parent_run_id: Option<String>,
    rerun_test_case_ids: Option<serde_json::Value>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<EvalRunRow> for EvalRun {
    type Error = WorkspaceError;

    fn try_from(row: EvalRunRow) -> Result<Self, Self::Error> {
        let id = agent_fw_core::EvalRunId::new(row.id)
            .ok_or_else(|| WorkspaceError::Serde("eval run ID is empty".to_string()))?;
        let config = serde_json::from_value(row.config).ws_serde()?;
        let status = serde_json::from_value(row.status).ws_serde()?;
        let rerun_test_case_ids = parse_jsonb(row.rerun_test_case_ids)?;

        Ok(EvalRun {
            id,
            config,
            status,
            results: Vec::new(),
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
            parent_run_id: row.parent_run_id.and_then(agent_fw_core::EvalRunId::new),
            rerun_test_case_ids,
        })
    }
}

#[derive(sqlx::FromRow)]
struct EvalResultRow {
    result: serde_json::Value,
}

#[derive(sqlx::FromRow)]
struct EvalTestCaseSetRow {
    id: String,
    name: String,
    description: String,
    test_cases: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<EvalTestCaseSetRow> for TestCaseSet {
    type Error = WorkspaceError;

    fn try_from(row: EvalTestCaseSetRow) -> Result<Self, Self::Error> {
        Ok(TestCaseSet {
            id: row.id,
            name: row.name,
            description: row.description,
            test_cases: serde_json::from_value(row.test_cases).ws_serde()?,
            created_at: row.created_at.to_rfc3339(),
        })
    }
}

#[derive(sqlx::FromRow)]
struct EvalForkRow {
    id: String,
    thread_id: String,
    parent_thread_id: Option<String>,
    fork_at_message_index: Option<i32>,
    edited_content: Option<String>,
    label: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<EvalForkRow> for EvalThreadFork {
    fn from(row: EvalForkRow) -> Self {
        Self {
            id: row.id,
            thread_id: row.thread_id,
            parent_thread_id: row.parent_thread_id,
            fork_at_message_index: row.fork_at_message_index.map(|value| value as u32),
            edited_content: row.edited_content,
            label: row.label,
            created_at: row.created_at.to_rfc3339(),
        }
    }
}

#[derive(sqlx::FromRow)]
struct DataSourceRow {
    id: String,
    name: String,
    #[sqlx(try_from = "String")]
    database_type: crate::data_source::DatabaseType,
    host: String,
    port: i32,
    database_name: String,
    schema_name: String,
    encrypted_credentials: Option<String>,
    is_active: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<DataSourceRow> for DataSource {
    type Error = WorkspaceError;

    fn try_from(row: DataSourceRow) -> Result<Self, Self::Error> {
        Ok(DataSource {
            id: row.id,
            name: row.name,
            database_type: row.database_type,
            host: row.host,
            port: row.port as u16,
            database_name: row.database_name,
            schema_name: row.schema_name,
            encrypted_credentials: row.encrypted_credentials,
            is_active: row.is_active,
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
        })
    }
}

#[derive(sqlx::FromRow)]
struct WorkspaceRow {
    id: String,
    #[allow(dead_code)]
    tenant_id: String,
    name: String,
    slug: String,
    description: Option<String>,
    database_config: serde_json::Value,
    model_config: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<WorkspaceRow> for Workspace {
    type Error = WorkspaceError;

    fn try_from(row: WorkspaceRow) -> Result<Self, Self::Error> {
        let id = WorkspaceId::new(row.id)
            .ok_or_else(|| WorkspaceError::Serde("workspace ID is empty".to_string()))?;
        let database_config: DatabaseConfig =
            serde_json::from_value(row.database_config).ws_serde()?;
        let model_config: WorkspaceModelConfig =
            serde_json::from_value(row.model_config).ws_serde()?;

        Ok(Workspace {
            id,
            name: row.name,
            slug: row.slug,
            description: row.description,
            database_config,
            model_config,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

fn enum_to_str<T: serde::Serialize>(val: &T) -> Result<String, WorkspaceError> {
    let json = serde_json::to_value(val)
        .map_err(|e| WorkspaceError::Serde(format!("enum serialization: {e}")))?;
    json.as_str()
        .map(ToString::to_string)
        .ok_or_else(|| WorkspaceError::Serde("enum did not serialize to string".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_eval::{EvalConfig, EvalStatus, TrajectoryMode};

    #[test]
    fn thread_row_conversion() {
        let row = ThreadRow {
            id: "t-1".into(),
            tenant_id: "tenant-abc".into(),
            title: Some("Hello".into()),
            source_id: Some("source-1".into()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let thread: Thread = row.into();
        assert_eq!(thread.id, "t-1");
        assert_eq!(thread.title, Some("Hello".into()));
        assert_eq!(thread.resource_id, "tenant-abc");
        assert_eq!(thread.source_id.as_deref(), Some("source-1"));
    }

    #[test]
    fn message_row_conversion() {
        let row = MessageRow {
            id: "m-1".into(),
            role: "user".into(),
            content: "hello".into(),
            tool_interactions: None,
            parts: None,
            created_at: chrono::Utc::now(),
        };
        let msg: Message = row.into();
        assert_eq!(msg.id, "m-1");
        assert_eq!(msg.role, "user");
        assert!(msg.tool_interactions.is_none());
    }

    #[test]
    fn test_case_row_conversion() {
        let row = TestCaseRow {
            id: "tc-1".into(),
            input: "increase prices".into(),
            status: TestCaseStatus::Active,
            tags: Some(vec!["pricing".into()]),
            expected_trajectory: serde_json::json!([
                {
                    "toolName": "draft_plan",
                    "source": { "type": "manual", "reason": null },
                    "position": 0
                }
            ]),
            trajectory_sources: Some(serde_json::json!([
                { "type": "manual", "reason": null }
            ])),
            trajectory_mode: TrajectoryMode::Strict,
            structured_ground_truth: Some(serde_json::json!({
                "kind": "text",
                "text": "plan created"
            })),
            source_thread_id: Some("thread-1".into()),
            source_session_id: Some("session-1".into()),
            tag_warnings: Some(vec!["duplicate".into()]),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let tc: AuthoredTestCase = row.try_into().expect("valid test case row");
        assert_eq!(tc.id.as_str(), "tc-1");
        assert_eq!(tc.expected_trajectory.len(), 1);
        assert_eq!(tc.expected_trajectory[0].tool_name, "draft_plan");
        assert_eq!(tc.tag_warnings, vec!["duplicate"]);
    }

    #[test]
    fn eval_run_row_conversion() {
        let row = EvalRunRow {
            id: "run-1".into(),
            config: serde_json::to_value(EvalConfig::default()).unwrap(),
            status: serde_json::to_value(EvalStatus::Queued).unwrap(),
            parent_run_id: Some("run-parent".into()),
            rerun_test_case_ids: Some(serde_json::json!(["tc-1"])),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let run: EvalRun = row.try_into().expect("valid eval run row");
        assert_eq!(run.id.as_str(), "run-1");
        assert_eq!(
            run.parent_run_id.as_ref().map(|id| id.as_str()),
            Some("run-parent")
        );
        assert_eq!(
            run.rerun_test_case_ids.as_ref().map(|ids| ids.len()),
            Some(1)
        );
        assert!(run.results.is_empty());
    }

    #[test]
    fn data_source_row_conversion() {
        use crate::data_source::DatabaseType;
        let row = DataSourceRow {
            id: "ds-1".into(),
            name: "prod".into(),
            database_type: DatabaseType::PostgreSQL,
            host: "localhost".into(),
            port: 5432,
            database_name: "mydb".into(),
            schema_name: "public".into(),
            encrypted_credentials: None,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let ds: DataSource = row.try_into().expect("valid database_type");
        assert_eq!(ds.id, "ds-1");
        assert_eq!(ds.port, 5432);
        assert!(ds.is_active);
    }

    #[test]
    fn workspace_row_conversion() {
        let row = WorkspaceRow {
            id: "ws-1".into(),
            tenant_id: "tenant-abc".into(),
            name: "Demo".into(),
            slug: "demo".into(),
            description: Some("A demo workspace".into()),
            database_config: serde_json::json!({"type": "default"}),
            model_config: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let ws: Workspace = row.try_into().expect("valid workspace row");
        assert_eq!(ws.id.as_str(), "ws-1");
        assert_eq!(ws.slug, "demo");
        assert!(matches!(ws.database_config, DatabaseConfig::Default));
    }
}
