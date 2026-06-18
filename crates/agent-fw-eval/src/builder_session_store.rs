//! KV-backed persistence helpers for [`TestCaseBuilderSession`].
//!
//! These helpers own the generic load/create/mutate/save bracket used by
//! interactive test-case authoring flows. Domain applications still choose:
//! - key layout
//! - TTL policy
//! - the concrete empty-session constructor
//! - boundary error mapping

use std::{sync::Arc, time::Duration};

use agent_fw_algebra::{KVError, KVStore, KVStoreExt};

use crate::test_case::TestCaseBuilderSession;

/// Session-key derivation policy for persisted builder sessions.
pub type BuilderSessionKeyFn = dyn Fn(&str, &str) -> String + Send + Sync;

/// Empty-session factory used when a session is first created.
pub type BuilderSessionFactoryFn = dyn Fn(&str) -> TestCaseBuilderSession + Send + Sync;

/// Reusable builder-session persistence policy.
///
/// This packages the three pieces every consuming app/toolkit otherwise has to
/// thread manually:
/// - session key layout
/// - TTL policy
/// - the empty-session factory
///
/// The policy does not own a concrete KV backend. Callers provide `&dyn KVStore`
/// at execution time, which keeps the API usable in request handlers, tool
/// environments, and tests without introducing another trait surface.
#[derive(Clone)]
pub struct BuilderSessionStoreConfig {
    ttl: Option<Duration>,
    key_fn: Arc<BuilderSessionKeyFn>,
    factory: Arc<BuilderSessionFactoryFn>,
}

/// Concrete builder-session store for callers that already own a KV backend.
///
/// This removes the need for application services to thread both
/// `Arc<dyn KVStore>` and `BuilderSessionStoreConfig` through every call site.
#[derive(Clone)]
pub struct BuilderSessionStore {
    kv: Arc<dyn KVStore>,
    config: BuilderSessionStoreConfig,
}

impl Default for BuilderSessionStoreConfig {
    fn default() -> Self {
        Self {
            ttl: None,
            key_fn: Arc::new(|_tenant, session_id| session_id.to_string()),
            factory: Arc::new(|session_id| TestCaseBuilderSession::new(session_id, "")),
        }
    }
}

impl BuilderSessionStoreConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_ttl(mut self, ttl: Option<Duration>) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn with_session_key_fn<F>(mut self, key_fn: F) -> Self
    where
        F: Fn(&str, &str) -> String + Send + Sync + 'static,
    {
        self.key_fn = Arc::new(key_fn);
        self
    }

    pub fn with_session_factory<F>(mut self, factory: F) -> Self
    where
        F: Fn(&str) -> TestCaseBuilderSession + Send + Sync + 'static,
    {
        self.factory = Arc::new(factory);
        self
    }

    pub fn ttl(&self) -> Option<Duration> {
        self.ttl
    }

    pub fn key(&self, tenant: &str, session_id: &str) -> String {
        (self.key_fn)(tenant, session_id)
    }

    pub fn create(&self, session_id: &str) -> TestCaseBuilderSession {
        (self.factory)(session_id)
    }

    pub async fn exists(
        &self,
        kv: &dyn KVStore,
        tenant: &str,
        session_id: &str,
    ) -> Result<bool, KVError> {
        self.load(kv, tenant, session_id)
            .await
            .map(|session| session.is_some())
    }

    pub async fn load(
        &self,
        kv: &dyn KVStore,
        tenant: &str,
        session_id: &str,
    ) -> Result<Option<TestCaseBuilderSession>, KVError> {
        let key = self.key(tenant, session_id);
        load_builder_session(kv, tenant, &key).await
    }

    pub async fn load_or_create(
        &self,
        kv: &dyn KVStore,
        tenant: &str,
        session_id: &str,
    ) -> Result<TestCaseBuilderSession, KVError> {
        let key = self.key(tenant, session_id);
        load_or_create_builder_session(kv, tenant, &key, || self.create(session_id)).await
    }

    pub async fn save(
        &self,
        kv: &dyn KVStore,
        tenant: &str,
        session: &TestCaseBuilderSession,
    ) -> Result<(), KVError> {
        let key = self.key(tenant, session.session_id());
        save_builder_session(kv, tenant, &key, session, self.ttl).await
    }

    pub async fn delete(
        &self,
        kv: &dyn KVStore,
        tenant: &str,
        session_id: &str,
    ) -> Result<(), KVError> {
        let key = self.key(tenant, session_id);
        kv.delete(tenant, &key).await.map(|_| ())
    }

    pub async fn mutate<F, T, E>(
        &self,
        kv: &dyn KVStore,
        tenant: &str,
        session_id: &str,
        f: F,
    ) -> Result<(TestCaseBuilderSession, T), MutateBuilderSessionError<E>>
    where
        F: FnOnce(&mut TestCaseBuilderSession) -> Result<T, E>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let key = self.key(tenant, session_id);
        mutate_builder_session(kv, tenant, &key, self.ttl, || self.create(session_id), f).await
    }
}

impl BuilderSessionStore {
    pub fn new(kv: Arc<dyn KVStore>, config: BuilderSessionStoreConfig) -> Self {
        Self { kv, config }
    }

    pub fn kv(&self) -> &Arc<dyn KVStore> {
        &self.kv
    }

    pub fn config(&self) -> &BuilderSessionStoreConfig {
        &self.config
    }

    pub fn key(&self, tenant: &str, session_id: &str) -> String {
        self.config.key(tenant, session_id)
    }

    pub async fn exists(&self, tenant: &str, session_id: &str) -> Result<bool, KVError> {
        self.config
            .exists(self.kv.as_ref(), tenant, session_id)
            .await
    }

    pub async fn load(
        &self,
        tenant: &str,
        session_id: &str,
    ) -> Result<Option<TestCaseBuilderSession>, KVError> {
        self.config.load(self.kv.as_ref(), tenant, session_id).await
    }

    pub async fn load_or_create(
        &self,
        tenant: &str,
        session_id: &str,
    ) -> Result<TestCaseBuilderSession, KVError> {
        self.config
            .load_or_create(self.kv.as_ref(), tenant, session_id)
            .await
    }

    pub async fn save(
        &self,
        tenant: &str,
        session: &TestCaseBuilderSession,
    ) -> Result<(), KVError> {
        self.config.save(self.kv.as_ref(), tenant, session).await
    }

    pub async fn delete(&self, tenant: &str, session_id: &str) -> Result<(), KVError> {
        self.config
            .delete(self.kv.as_ref(), tenant, session_id)
            .await
    }

    pub async fn mutate<F, T, E>(
        &self,
        tenant: &str,
        session_id: &str,
        f: F,
    ) -> Result<(TestCaseBuilderSession, T), MutateBuilderSessionError<E>>
    where
        F: FnOnce(&mut TestCaseBuilderSession) -> Result<T, E>,
        E: std::error::Error + Send + Sync + 'static,
    {
        self.config
            .mutate(self.kv.as_ref(), tenant, session_id, f)
            .await
    }
}

/// Load a builder session from KV.
pub async fn load_builder_session(
    kv: &dyn KVStore,
    tenant: &str,
    key: &str,
) -> Result<Option<TestCaseBuilderSession>, KVError> {
    kv.get::<TestCaseBuilderSession>(tenant, key).await
}

/// Load a builder session from KV, or create a new one if it does not exist.
pub async fn load_or_create_builder_session<F>(
    kv: &dyn KVStore,
    tenant: &str,
    key: &str,
    create: F,
) -> Result<TestCaseBuilderSession, KVError>
where
    F: FnOnce() -> TestCaseBuilderSession,
{
    match load_builder_session(kv, tenant, key).await? {
        Some(session) => Ok(session),
        None => Ok(create()),
    }
}

/// Save a builder session to KV with the caller's TTL policy.
pub async fn save_builder_session(
    kv: &dyn KVStore,
    tenant: &str,
    key: &str,
    session: &TestCaseBuilderSession,
    ttl: Option<Duration>,
) -> Result<(), KVError> {
    kv.put(tenant, key, session, ttl).await
}

/// Error from mutating a builder session.
#[derive(Debug, thiserror::Error)]
pub enum MutateBuilderSessionError<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Store(#[from] KVError),
    #[error(transparent)]
    Mutate(E),
}

/// Bracket pattern: load/create → mutate → touch → save.
pub async fn mutate_builder_session<F, C, T, E>(
    kv: &dyn KVStore,
    tenant: &str,
    key: &str,
    ttl: Option<Duration>,
    create: C,
    f: F,
) -> Result<(TestCaseBuilderSession, T), MutateBuilderSessionError<E>>
where
    F: FnOnce(&mut TestCaseBuilderSession) -> Result<T, E>,
    C: FnOnce() -> TestCaseBuilderSession,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut session = load_or_create_builder_session(kv, tenant, key, create).await?;
    let result = f(&mut session).map_err(MutateBuilderSessionError::Mutate)?;
    session.touch();
    save_builder_session(kv, tenant, key, &session, ttl).await?;
    Ok((session, result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_interpreter::DashMapKVStore;

    #[tokio::test]
    async fn load_or_create_returns_existing_session() {
        let kv = DashMapKVStore::new();
        let session = TestCaseBuilderSession::new("sess-1", "prompt");
        save_builder_session(&kv, "tenant", "k", &session, None)
            .await
            .unwrap();

        let loaded = load_or_create_builder_session(&kv, "tenant", "k", || {
            TestCaseBuilderSession::new("other", "")
        })
        .await
        .unwrap();

        assert_eq!(loaded.session_id(), "sess-1");
        assert_eq!(loaded.input(), "prompt");
    }

    #[tokio::test]
    async fn load_or_create_uses_factory_when_missing() {
        let kv = DashMapKVStore::new();
        let loaded = load_or_create_builder_session(&kv, "tenant", "missing", || {
            TestCaseBuilderSession::new("sess-1", "")
        })
        .await
        .unwrap();

        assert_eq!(loaded.session_id(), "sess-1");
        assert_eq!(loaded.input(), "");
    }

    #[tokio::test]
    async fn builder_session_store_roundtrips_with_bound_kv() {
        let kv: Arc<dyn KVStore> = Arc::new(DashMapKVStore::new());
        let store = BuilderSessionStore::new(
            Arc::clone(&kv),
            BuilderSessionStoreConfig::default()
                .with_session_key_fn(|tenant, session_id| format!("{tenant}:{session_id}")),
        );
        let session = TestCaseBuilderSession::new("sess-1", "prompt");

        store.save("tenant-a", &session).await.unwrap();

        let loaded = store.load("tenant-a", "sess-1").await.unwrap().unwrap();
        assert_eq!(loaded.session_id(), "sess-1");
        assert_eq!(loaded.input(), "prompt");
        assert_eq!(store.key("tenant-a", "sess-1"), "tenant-a:sess-1");
    }

    #[derive(Debug, thiserror::Error)]
    #[error("boom")]
    struct Boom;

    #[tokio::test]
    async fn mutate_builder_session_persists_mutation() {
        let kv = DashMapKVStore::new();
        let (session, step_count) = mutate_builder_session(
            &kv,
            "tenant",
            "sess",
            None,
            || TestCaseBuilderSession::new("sess-1", "draft"),
            |session| {
                session.add_step(
                    "draft_plan",
                    crate::test_case::TrajectoryStepSource::manual(),
                )?;
                Ok::<_, crate::test_case::TestCaseBuilderError>(session.trajectory_steps.len())
            },
        )
        .await
        .unwrap();

        assert_eq!(step_count, 1);
        assert_eq!(session.trajectory_steps.len(), 1);

        let reloaded = load_builder_session(&kv, "tenant", "sess")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.trajectory_steps.len(), 1);
    }

    #[tokio::test]
    async fn mutate_builder_session_preserves_mutation_error() {
        let kv = DashMapKVStore::new();
        let err = mutate_builder_session(
            &kv,
            "tenant",
            "sess",
            None,
            || TestCaseBuilderSession::new("sess-1", ""),
            |_session| Err::<(), _>(Boom),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, MutateBuilderSessionError::Mutate(Boom)));
    }

    #[tokio::test]
    async fn config_applies_custom_key_and_factory() {
        let kv = DashMapKVStore::new();
        let config = BuilderSessionStoreConfig::new()
            .with_session_key_fn(|tenant, session_id| format!("{tenant}:builder:{session_id}"))
            .with_session_factory(|session_id| {
                TestCaseBuilderSession::new(session_id, "framework default prompt")
            });

        let session = config
            .load_or_create(&kv, "tenant-a", "sess-42")
            .await
            .unwrap();
        assert_eq!(session.session_id(), "sess-42");
        assert_eq!(session.input(), "framework default prompt");

        let stored = kv
            .get::<TestCaseBuilderSession>("tenant-a", "tenant-a:builder:sess-42")
            .await
            .unwrap();
        assert!(
            stored.is_none(),
            "load_or_create should not persist implicitly"
        );

        config.save(&kv, "tenant-a", &session).await.unwrap();
        let stored = kv
            .get::<TestCaseBuilderSession>("tenant-a", "tenant-a:builder:sess-42")
            .await
            .unwrap();
        assert!(stored.is_some());
    }

    #[tokio::test]
    async fn config_mutate_uses_ttl_and_deletes_by_policy_key() {
        let kv = DashMapKVStore::new();
        let config = BuilderSessionStoreConfig::new()
            .with_ttl(Some(Duration::from_secs(60)))
            .with_session_key_fn(|tenant, session_id| format!("{tenant}:builder:{session_id}"));

        let (session, ()) = config
            .mutate(
                &kv,
                "tenant-a",
                "sess-7",
                |session| -> Result<(), crate::test_case::TestCaseBuilderError> {
                    session.add_step(
                        "draft_plan",
                        crate::test_case::TrajectoryStepSource::manual(),
                    )
                },
            )
            .await
            .unwrap();
        assert_eq!(session.trajectory_steps.len(), 1);
        assert!(config.exists(&kv, "tenant-a", "sess-7").await.unwrap());

        config.delete(&kv, "tenant-a", "sess-7").await.unwrap();
        assert!(!config.exists(&kv, "tenant-a", "sess-7").await.unwrap());
    }
}
