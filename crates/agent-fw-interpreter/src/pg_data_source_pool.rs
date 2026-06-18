//! PostgreSQL-backed DataSourcePool via sqlx.
//!
//! Caches `PgPool` instances by data source ID, creating new pools on first
//! resolve and reusing them for subsequent calls. The pool handle is stored
//! in [`PooledConnection::Postgresql`] via type-erased `Box<dyn Any + Send>`,
//! which consumers downcast via [`PooledConnection::pg_downcast::<PgPool>()`].
//!
//! # Feature Gate
//!
//! Requires the `postgres` feature.
//!
//! # Laws Satisfied
//!
//! - L1 (Get-or-Create): Atomic via DashMap entry API
//! - L2 (Invalidation): Removes cache entry, closes pool
//! - L3 (Eviction): Removes entries not accessed in `max_age`
//! - L4 (Has-Source-Consistency): Reflects DashMap state
//! - L5 (Count-Accuracy): Delegates to DashMap::len
//! - L6 (Invalidation-Idempotent): No error on missing
//! - L7 (Clear-Empties): Clears all entries
//! - L8 (Accessibility-Liveness): Checks cache presence (best-effort for async pool)
//! - L9 (Maintenance-Best-Effort): Delegates to eviction

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use sqlx::postgres::PgPool;
use tracing::debug;

use agent_fw_algebra::{
    DataSourceConfig, DataSourcePool, DataSourcePoolError, DatabaseType, PooledConnection,
    ResolvedSource,
};

// =============================================================================
// Cached Entry
// =============================================================================

/// Current time as epoch seconds (monotonic-safe via SystemTime).
fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Cached information about a PostgreSQL data source.
struct PgSourceInfo {
    /// The sqlx connection pool.
    pool: PgPool,
    /// Last access time as epoch seconds. AtomicU64 eliminates lock poisoning.
    last_accessed_epoch: AtomicU64,
}

// =============================================================================
// PgDataSourcePool
// =============================================================================

/// PostgreSQL connection pool cache implementing [`DataSourcePool`].
///
/// Each data source gets its own `PgPool` with configurable connection limits.
/// Pools are created lazily on first `resolve()` or `open_connection()` call
/// and cached by source ID.
///
/// # Async Bridge
///
/// The [`DataSourcePool`] trait is synchronous, but `PgPool::connect()` is async.
/// This implementation uses `tokio::task::block_in_place` + `Handle::block_on`
/// to bridge async pool creation within the sync trait. This requires running
/// inside a multi-threaded Tokio runtime.
///
/// # Example
///
/// ```ignore
/// use agent_fw_algebra::{DataSourceConfig, DataSourcePool};
/// use agent_fw_interpreter::PgDataSourcePool;
///
/// let cache = PgDataSourcePool::new();
/// let config = DataSourceConfig::postgresql("my-pg", "postgresql://localhost/mydb");
///
/// // Resolve (creates PgPool on first call)
/// let resolved = cache.resolve(&config)?;
///
/// // Open connection (returns PooledConnection::Postgresql with PgPool inside)
/// let conn = cache.open_connection(&config)?;
/// let pool = conn.pg_downcast::<sqlx::PgPool>().unwrap();
/// ```
pub struct PgDataSourcePool {
    sources: DashMap<String, PgSourceInfo>,
    idle_timeout: Duration,
    max_connections: u32,
}

impl PgDataSourcePool {
    /// Create a new PostgreSQL data source pool with default settings.
    pub fn new() -> Self {
        Self {
            sources: DashMap::new(),
            idle_timeout: Duration::from_secs(600),
            max_connections: 10,
        }
    }

    /// Set the idle timeout for eviction.
    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Set the default max connections per pool.
    pub fn with_max_connections(mut self, max: u32) -> Self {
        self.max_connections = max;
        self
    }

    /// Create a PgPool from a DataSourceConfig.
    ///
    /// Bridges async pool creation into the sync `DataSourcePool` trait by
    /// spawning a scoped OS thread that calls `Handle::block_on`. This avoids
    /// `block_in_place`, which panics on `current_thread` runtimes.
    ///
    /// Thread creation overhead is acceptable since pools are created once per
    /// data source and cached thereafter.
    fn create_pool(&self, source: &DataSourceConfig) -> Result<PgPool, DataSourcePoolError> {
        let url = source.url.clone();
        let max_conns = source.max_connections.unwrap_or(self.max_connections);
        let timeout_secs = source.connect_timeout_secs.unwrap_or(30);

        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            DataSourcePoolError::ConnectionFailed(
                "PgDataSourcePool requires a running Tokio runtime".into(),
            )
        })?;

        std::thread::scope(|s| {
            s.spawn(|| {
                handle.block_on(async move {
                    sqlx::postgres::PgPoolOptions::new()
                        .max_connections(max_conns)
                        .acquire_timeout(Duration::from_secs(timeout_secs))
                        .connect(&url)
                        .await
                        .map_err(|e| DataSourcePoolError::ConnectionFailed(e.to_string()))
                })
            })
            .join()
            .unwrap_or_else(|_| {
                Err(DataSourcePoolError::ConnectionFailed(
                    "Pool creation thread panicked".into(),
                ))
            })
        })
    }

    /// Get or create a cached source entry, returning a clone of the PgPool.
    fn get_or_create_pool(&self, source: &DataSourceConfig) -> Result<PgPool, DataSourcePoolError> {
        match self.sources.entry(source.id.clone()) {
            Entry::Occupied(e) => {
                let info = e.get();
                info.last_accessed_epoch
                    .store(epoch_secs(), Ordering::Relaxed);
                Ok(info.pool.clone())
            }
            Entry::Vacant(e) => {
                let pool = self.create_pool(source)?;
                let info = PgSourceInfo {
                    pool: pool.clone(),
                    last_accessed_epoch: AtomicU64::new(epoch_secs()),
                };
                e.insert(info);
                Ok(pool)
            }
        }
    }

    /// Get a reference to a cached PgPool by source ID.
    ///
    /// This is a convenience for consumers who need the pool without
    /// going through the `PooledConnection` type-erasure.
    pub fn pool_for(&self, source_id: &str) -> Option<PgPool> {
        self.sources.get(source_id).map(|e| {
            e.last_accessed_epoch.store(epoch_secs(), Ordering::Relaxed);
            e.pool.clone()
        })
    }

    /// Close all cached pools.
    ///
    /// Pools are drained when dropped, so this clears the cache and
    /// lets Drop handle the actual connection shutdown.
    pub fn close_all(&self) {
        self.sources.clear();
    }
}

impl Default for PgDataSourcePool {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// DataSourcePool Implementation
// =============================================================================

impl DataSourcePool for PgDataSourcePool {
    fn resolve(&self, source: &DataSourceConfig) -> Result<ResolvedSource, DataSourcePoolError> {
        if source.db_type != DatabaseType::PostgreSQL {
            return Err(DataSourcePoolError::UnsupportedType(format!(
                "PgDataSourcePool only supports PostgreSQL, got: {}",
                source.db_type
            )));
        }

        // Ensure pool is created/cached
        let _ = self.get_or_create_pool(source)?;

        Ok(ResolvedSource {
            config: source.clone(),
            resolved_path: None, // PostgreSQL has no file path
        })
    }

    fn open_connection(
        &self,
        source: &DataSourceConfig,
    ) -> Result<PooledConnection, DataSourcePoolError> {
        if source.db_type != DatabaseType::PostgreSQL {
            return Err(DataSourcePoolError::UnsupportedType(format!(
                "PgDataSourcePool only supports PostgreSQL, got: {}",
                source.db_type
            )));
        }

        let pool = self.get_or_create_pool(source)?;
        Ok(PooledConnection::Postgresql(Box::new(pool)))
    }

    /// Check if a PostgreSQL source is accessible.
    ///
    /// For PostgreSQL, this checks that the pool is cached and not closed.
    /// A full async health check (`SELECT 1`) should be done by the consumer
    /// using the PgPool directly.
    fn is_accessible(&self, source_id: &str) -> bool {
        self.sources
            .get(source_id)
            .map(|e| !e.pool.is_closed())
            .unwrap_or(false)
    }

    fn invalidate(&self, source_id: &str) {
        if self.sources.remove(source_id).is_some() {
            debug!(source_id = %source_id, "Invalidated PostgreSQL source cache");
        }
    }

    fn evict_idle(&self, max_age: Duration) -> usize {
        let now = epoch_secs();
        let max_age_secs = max_age.as_secs();
        let mut evicted = 0;

        // Collect IDs to evict
        let to_evict: Vec<String> = self
            .sources
            .iter()
            .filter_map(|entry| {
                let last = entry.last_accessed_epoch.load(Ordering::Relaxed);
                if now.saturating_sub(last) > max_age_secs {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        for id in to_evict {
            if self.sources.remove(&id).is_some() {
                evicted += 1;
                debug!(source_id = %id, "Evicted idle PostgreSQL source");
            }
        }

        evicted
    }

    fn source_count(&self) -> usize {
        self.sources.len()
    }

    fn has_source(&self, source_id: &str) -> bool {
        self.sources.contains_key(source_id)
    }

    fn clear(&self) {
        self.sources.clear();
        debug!("Cleared all PostgreSQL source caches");
    }

    fn maintenance(&self) -> usize {
        self.evict_idle(self.idle_timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_sqlite_config() {
        let pool = PgDataSourcePool::new();
        let config = DataSourceConfig::sqlite("test", "sqlite:test.db");
        assert!(pool.resolve(&config).is_err());
        assert!(pool.open_connection(&config).is_err());
    }

    #[test]
    fn cache_operations_without_connection() {
        let pool = PgDataSourcePool::new();

        // Initially empty
        assert_eq!(pool.source_count(), 0);
        assert!(!pool.has_source("nonexistent"));
        assert!(!pool.is_accessible("nonexistent"));

        // Invalidate non-existent is safe (L6)
        pool.invalidate("nonexistent");
        assert_eq!(pool.source_count(), 0);

        // Clear empty is safe (L7)
        pool.clear();
        assert_eq!(pool.source_count(), 0);

        // Maintenance on empty returns 0 (L9)
        assert_eq!(pool.maintenance(), 0);
    }

    #[test]
    fn default_config() {
        let pool = PgDataSourcePool::default();
        assert_eq!(pool.idle_timeout, Duration::from_secs(600));
        assert_eq!(pool.max_connections, 10);
    }

    #[test]
    fn builder_methods() {
        let pool = PgDataSourcePool::new()
            .with_idle_timeout(Duration::from_secs(300))
            .with_max_connections(20);
        assert_eq!(pool.idle_timeout, Duration::from_secs(300));
        assert_eq!(pool.max_connections, 20);
    }
}
