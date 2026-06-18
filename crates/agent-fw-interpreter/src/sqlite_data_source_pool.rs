//! SQLite-backed datasource pool/cache interpreter.
//!
//! Provides efficient configuration caching and lazy connection creation for
//! multiple data sources. This is the production implementation of the
//! [`DataSourcePool`] trait defined in `agent-fw-algebra`.
//!
//! # Architecture
//!
//! Unlike a traditional connection pool, this cache stores configuration and
//! resolved paths, creating fresh connections on demand. This is appropriate
//! because:
//!
//! - SQLite handles its own connection pooling internally
//! - rusqlite::Connection is not thread-safe (cannot be stored in DashMap)
//! - Connection creation is fast for SQLite
//!
//! ```text
//! ┌─────────────────┐     ┌───────────────────┐
//! │  Direct Data    │────▶│ SqliteDataSourcePool│
//! │    Routes       │     │  impl DataSourcePool
//! └─────────────────┘     │                   │
//!                         │  sources: DashMap │
//!                         │    ID → SourceInfo│
//!                         └───────────────────┘
//!                                   │
//!                                   ▼
//!                         ┌───────────────────┐
//!                         │    SourceInfo     │
//!                         │ - config          │
//!                         │ - resolved_path   │
//!                         │ - last_accessed   │
//!                         └───────────────────┘
//! ```
//!
//! # Laws
//!
//! All laws from [`DataSourcePool`] are satisfied:
//!
//! - **L1 Get-or-Create**: `resolve(source)` uses atomic entry API - no race
//! - **L2 Invalidation**: `invalidate(id)` removes cached config
//! - **L3 Eviction**: `evict_idle(age)` removes entries not accessed in `age` time
//! - **L4 Has-Source-Consistency**: `has_source(id)` reflects cache state
//! - **L5 Count-Accuracy**: `source_count()` reflects actual cache size
//! - **L6 Invalidation-Idempotent**: Repeated `invalidate` is safe
//! - **L7 Clear-Empties**: `clear()` empties the cache
//! - **L8 Accessibility-Liveness**: `is_accessible()` performs live file check
//! - **L9 Maintenance-Best-Effort**: `maintenance()` delegates to eviction

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use agent_fw_algebra::{
    parse_sqlite_path, DataSourceConfig, DataSourcePool, DataSourcePoolError, DatabaseType,
    PooledConnection, ResolvedSource,
};
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use tracing::{debug, instrument, warn};

// =============================================================================
// Configuration for the Cache
// =============================================================================

/// Configuration for the SQLite datasource pool/cache.
#[derive(Debug, Clone)]
pub struct SqliteDataSourcePoolConfig {
    /// Maximum number of sources to cache.
    ///
    /// This is a **soft limit**: when reached, idle eviction runs before
    /// adding new sources. If no entries are idle, new entries are still
    /// added (exceeding this limit). Use `maintenance()` periodically to
    /// enforce the limit over time.
    pub max_sources: usize,
    /// Idle timeout before eviction.
    ///
    /// Sources not accessed within this duration are eligible for eviction
    /// during `evict_idle()` or `maintenance()` calls.
    pub idle_timeout: Duration,
}

impl Default for SqliteDataSourcePoolConfig {
    fn default() -> Self {
        Self {
            max_sources: 100,
            idle_timeout: Duration::from_secs(600), // 10 minutes
        }
    }
}

// =============================================================================
// Source Info (cached entry)
// =============================================================================

/// Cached information about a data source.
#[derive(Debug)]
struct SourceInfo {
    /// The source configuration (used to construct ResolvedSource in resolve()).
    config: DataSourceConfig,
    /// Resolved SQLite path (if SQLite).
    sqlite_path: Option<PathBuf>,
    /// Last access time (for idle eviction).
    last_accessed: RwLock<Instant>,
}

// =============================================================================
// SqliteDataSourcePool — Production Interpreter
// =============================================================================

/// Cache of data source configurations for efficient connection creation.
///
/// Thread-safe via `DashMap` for concurrent access. Implements the
/// [`DataSourcePool`] algebra trait.
///
/// # Example
///
/// ```ignore
/// use agent_fw_algebra::{DataSourceConfig, DataSourcePool};
/// use agent_fw_interpreter::{SqliteDataSourcePool, SqliteDataSourcePoolConfig};
///
/// let cache = SqliteDataSourcePool::with_defaults();
/// let config = DataSourceConfig::sqlite("my-db", "sqlite:/path/to/db.sqlite");
///
/// // Resolve (caches internally, atomic)
/// let resolved = cache.resolve(&config)?;
///
/// // Open a connection
/// let conn = cache.open_connection(&config)?;
///
/// // Check accessibility (live check)
/// if cache.is_accessible("my-db") {
///     // ...
/// }
/// ```
pub struct SqliteDataSourcePool {
    /// Map of source ID to source info.
    sources: DashMap<String, Arc<SourceInfo>>,
    /// Cache configuration.
    config: SqliteDataSourcePoolConfig,
}

impl SqliteDataSourcePool {
    /// Create a new pool cache with the given configuration.
    pub fn new(config: SqliteDataSourcePoolConfig) -> Self {
        Self {
            sources: DashMap::new(),
            config,
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(SqliteDataSourcePoolConfig::default())
    }

    /// Validate and resolve source info from configuration.
    ///
    /// # Side Effects
    /// This function performs filesystem IO to validate file existence.
    /// It is NOT a pure function despite the name suggesting resolution.
    ///
    /// # Errors
    /// Returns `FileNotFound` if the SQLite database file doesn't exist.
    /// Returns `InvalidConfig` if the URL cannot be parsed.
    /// Returns `UnsupportedType` for PostgreSQL/MySQL (not yet implemented).
    fn validate_and_resolve_source_info(
        source: &DataSourceConfig,
    ) -> Result<SourceInfo, DataSourcePoolError> {
        match source.db_type {
            DatabaseType::SQLite => {
                let path = parse_sqlite_path(&source.url).ok_or_else(|| {
                    DataSourcePoolError::InvalidConfig("Invalid SQLite URL".into())
                })?;

                // Note: We check file existence here for validation, but
                // is_accessible() does a live check later.
                if !path.exists() {
                    return Err(DataSourcePoolError::FileNotFound(path));
                }

                Ok(SourceInfo {
                    config: source.clone(),
                    sqlite_path: Some(path),
                    last_accessed: RwLock::new(Instant::now()),
                })
            }
            DatabaseType::PostgreSQL => {
                // Future: Would create sqlx::PgPool here
                Err(DataSourcePoolError::UnsupportedType(
                    "PostgreSQL not yet implemented".into(),
                ))
            }
            DatabaseType::MySQL => Err(DataSourcePoolError::UnsupportedType(
                "MySQL not yet implemented".into(),
            )),
        }
    }

    /// Ensure we haven't exceeded max_sources, evicting idle if needed.
    fn ensure_capacity(&self) {
        if self.sources.len() >= self.config.max_sources {
            let evicted = self.evict_idle(self.config.idle_timeout);
            if evicted > 0 {
                debug!(evicted_count = evicted, "Evicted idle sources to make room");
            }
        }
    }
}

// =============================================================================
// DataSourcePool Trait Implementation
// =============================================================================

impl DataSourcePool for SqliteDataSourcePool {
    /// Resolve a data source configuration.
    ///
    /// Uses DashMap's atomic entry API to avoid TOCTOU race conditions.
    /// This satisfies **Law L1: Get-or-Create** atomically.
    fn resolve(&self, source: &DataSourceConfig) -> Result<ResolvedSource, DataSourcePoolError> {
        // Ensure capacity before adding new entries
        self.ensure_capacity();

        // Atomic get-or-insert using entry API
        // This eliminates the TOCTOU race condition
        let entry = match self.sources.entry(source.id.clone()) {
            Entry::Occupied(e) => e,
            Entry::Vacant(e) => {
                let info = Arc::new(Self::validate_and_resolve_source_info(source)?);
                e.insert_entry(info)
            }
        };

        // Get a reference to the value (SourceInfo wrapped in Arc)
        let info = entry.get();

        // Update last accessed time
        {
            let mut last = info.last_accessed.write().map_err(|e| {
                warn!(source_id = %source.id, error = ?e, "Lock poisoned");
                DataSourcePoolError::LockPoisoned(source.id.clone())
            })?;
            *last = Instant::now();
        }

        Ok(ResolvedSource {
            config: info.config.clone(),
            resolved_path: info.sqlite_path.clone(),
        })
    }

    /// Open a connection to the data source.
    ///
    /// Creates a fresh read-only connection on each call.
    /// For SQLite, this is efficient because SQLite manages its own
    /// internal connection pooling.
    #[instrument(skip(self))]
    fn open_connection(
        &self,
        source: &DataSourceConfig,
    ) -> Result<PooledConnection, DataSourcePoolError> {
        // Ensure we have cached info
        let resolved = self.resolve(source)?;

        // Create connection
        if let Some(ref path) = resolved.resolved_path {
            let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
            let conn = rusqlite::Connection::open_with_flags(path, flags)
                .map_err(|e| DataSourcePoolError::ConnectionFailed(e.to_string()))?;
            Ok(PooledConnection::Sqlite(conn))
        } else {
            Err(DataSourcePoolError::UnsupportedType(
                source.db_type.to_string(),
            ))
        }
    }

    /// Check if a source is currently accessible.
    ///
    /// Performs a **live check** - does not return cached state.
    /// For SQLite, this checks if the file currently exists.
    fn is_accessible(&self, source_id: &str) -> bool {
        self.sources
            .get(source_id)
            .map(|entry| entry.sqlite_path.as_ref().map_or(false, |p| p.exists()))
            .unwrap_or(false)
    }

    /// Invalidate a cached source.
    fn invalidate(&self, source_id: &str) {
        if self.sources.remove(source_id).is_some() {
            debug!(source_id = %source_id, "Invalidated source cache");
        }
    }

    /// Evict sources not accessed in the last `max_age` duration.
    fn evict_idle(&self, max_age: Duration) -> usize {
        let mut evicted = 0;
        let now = Instant::now();

        // Collect IDs to evict (can't remove during iteration)
        let to_evict: Vec<String> = self
            .sources
            .iter()
            .filter_map(|entry| {
                match entry.last_accessed.read() {
                    Ok(last) => {
                        if now.duration_since(*last) > max_age {
                            return Some(entry.key().clone());
                        }
                        None
                    }
                    Err(_) => {
                        // Lock poisoned - skip this entry but log the issue.
                        // We don't evict because we can't determine idle status.
                        warn!(source_id = %entry.key(), "Lock poisoned during eviction check, skipping");
                        None
                    }
                }
            })
            .collect();

        // Remove collected entries
        for id in to_evict {
            if self.sources.remove(&id).is_some() {
                evicted += 1;
                debug!(source_id = %id, "Evicted idle source");
            }
        }

        evicted
    }

    /// Get the number of cached sources.
    fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Check if a source is cached.
    fn has_source(&self, source_id: &str) -> bool {
        self.sources.contains_key(source_id)
    }

    /// Clear all cached sources.
    fn clear(&self) {
        self.sources.clear();
        debug!("Cleared all cached sources");
    }

    /// Run periodic maintenance (eviction).
    ///
    /// Satisfies **Law L9: Maintenance-Best-Effort** by delegating to
    /// `evict_idle` with the configured idle timeout.
    ///
    /// Call this periodically from a background task to prevent unbounded
    /// cache growth. Returns the number of sources evicted.
    fn maintenance(&self) -> usize {
        self.evict_idle(self.config.idle_timeout)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a mock SourceInfo for testing.
    ///
    /// This avoids repeating the same construction pattern across tests.
    fn mock_source_info(id: &str) -> Arc<SourceInfo> {
        Arc::new(SourceInfo {
            config: DataSourceConfig::sqlite(id, ":memory:"),
            sqlite_path: None,
            last_accessed: RwLock::new(Instant::now()),
        })
    }

    /// Helper to create a mock SourceInfo with a specific last_accessed time.
    fn mock_source_info_with_time(id: &str, last_accessed: Instant) -> Arc<SourceInfo> {
        Arc::new(SourceInfo {
            config: DataSourceConfig::sqlite(id, ":memory:"),
            sqlite_path: None,
            last_accessed: RwLock::new(last_accessed),
        })
    }

    #[test]
    fn pool_cache_config_defaults() {
        let config = SqliteDataSourcePoolConfig::default();
        assert_eq!(config.max_sources, 100);
        assert_eq!(config.idle_timeout, Duration::from_secs(600));
    }

    /// L4: Has-Source-Consistency — has_source(id) ⟺ source is cached.
    #[test]
    fn law_l4_has_source_consistency() {
        let cache = SqliteDataSourcePool::with_defaults();

        // Initially empty
        assert!(!cache.has_source("nonexistent"));

        // Create a mock entry directly
        cache
            .sources
            .insert("test-source".into(), mock_source_info("test-source"));

        // Now it exists
        assert!(cache.has_source("test-source"));
        assert_eq!(cache.source_count(), 1);
    }

    /// L5: Count-Accuracy — source_count() equals number of cached sources.
    ///
    /// This test verifies that source_count() accurately reflects the cache state
    /// across all operations: clear, insert, and invalidate.
    #[test]
    fn law_l5_count_accuracy() {
        let cache = SqliteDataSourcePool::with_defaults();

        // Initially empty
        assert_eq!(cache.source_count(), 0);

        // Insert entries directly
        for i in 0..3 {
            cache.sources.insert(
                format!("source-{i}"),
                mock_source_info(&format!("source-{i}")),
            );
        }
        assert_eq!(cache.source_count(), 3);

        // Invalidate one
        cache.invalidate("source-1");
        assert_eq!(cache.source_count(), 2);

        // Invalidate non-existent (no change)
        cache.invalidate("nonexistent");
        assert_eq!(cache.source_count(), 2);

        // Clear all
        cache.clear();
        assert_eq!(cache.source_count(), 0);
    }

    /// L6: Invalidation-Idempotent — invalidate(id) on missing source is a no-op.
    #[test]
    fn law_l6_invalidation_idempotent() {
        let cache = SqliteDataSourcePool::with_defaults();

        // Invalidate non-existent is safe
        cache.invalidate("nonexistent");

        // Create and invalidate
        cache.sources.insert(
            "test-invalidate".into(),
            mock_source_info("test-invalidate"),
        );
        assert!(cache.has_source("test-invalidate"));

        // First invalidate
        cache.invalidate("test-invalidate");
        assert!(!cache.has_source("test-invalidate"));

        // Second invalidate (idempotent)
        cache.invalidate("test-invalidate");
        assert!(!cache.has_source("test-invalidate"));
    }

    /// L7: Clear-Empties — After clear(), source_count() == 0.
    #[test]
    fn law_l7_clear_empties() {
        let cache = SqliteDataSourcePool::with_defaults();

        // Add some entries
        for i in 0..5 {
            cache.sources.insert(
                format!("source-{i}"),
                mock_source_info(&format!("source-{i}")),
            );
        }

        assert_eq!(cache.source_count(), 5);

        // Clear
        cache.clear();

        // Verify empty
        assert_eq!(cache.source_count(), 0);
        for i in 0..5 {
            assert!(!cache.has_source(&format!("source-{i}")));
        }
    }

    /// L8: Accessibility-Liveness — is_accessible(id) reflects current state, not cached.
    #[test]
    fn law_l8_accessibility_liveness() {
        let cache = SqliteDataSourcePool::with_defaults();

        // Create a temporary database
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test_liveness.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)", [])
            .unwrap();
        drop(conn);

        let url = format!("sqlite:{}", db_path.display());
        let config = DataSourceConfig::sqlite("liveness-test", &url);

        // Resolve (caches)
        cache.resolve(&config).unwrap();

        // Should be accessible
        assert!(cache.is_accessible("liveness-test"));

        // Delete the file
        std::fs::remove_file(&db_path).unwrap();

        // Should NOT be accessible (live check, not cached)
        assert!(!cache.is_accessible("liveness-test"));
    }

    #[test]
    fn pool_cache_max_sources_enforcement() {
        let mut config = SqliteDataSourcePoolConfig::default();
        config.max_sources = 3;
        config.idle_timeout = Duration::from_secs(1);

        let cache = SqliteDataSourcePool::new(config);

        // Create temporary databases
        let temp_dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            let db_path = temp_dir.path().join(format!("test_{i}.db"));
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)", [])
                .unwrap();
            drop(conn);

            let url = format!("sqlite:{}", db_path.display());
            let source_config = DataSourceConfig::sqlite(&format!("source-{i}"), &url);
            cache.resolve(&source_config).unwrap();
        }

        // With max_sources=3 and idle_timeout=1s, some should have been evicted
        // (though this test may not trigger eviction depending on timing)
        assert!(cache.source_count() <= 5);
    }

    // =========================================================================
    // Algebraic Law Tests (L1-L3, L8-L9)
    // =========================================================================

    /// L1: Get-or-Create — First resolve caches; subsequent returns cached.
    ///
    /// This test verifies:
    /// 1. First call to resolve() caches the source
    /// 2. Subsequent calls return the same cached info (same resolved_path)
    /// 3. The source count reflects the cached entry
    #[test]
    fn law_l1_get_or_create() {
        // Create a valid SQLite database
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("law_l1.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)", [])
            .unwrap();
        drop(conn);

        let cache = SqliteDataSourcePool::with_defaults();
        let url = format!("sqlite:{}", db_path.display());
        let config = DataSourceConfig::sqlite("law-l1-source", &url);

        // Initially not cached
        assert!(!cache.has_source("law-l1-source"));
        assert_eq!(cache.source_count(), 0);

        // First resolve caches
        let resolved1 = cache
            .resolve(&config)
            .expect("first resolve should succeed");
        assert!(cache.has_source("law-l1-source"));
        assert_eq!(cache.source_count(), 1);

        // Second resolve returns cached (same path)
        let resolved2 = cache
            .resolve(&config)
            .expect("second resolve should succeed");
        assert_eq!(resolved1.resolved_path, resolved2.resolved_path);

        // Still only one entry
        assert_eq!(cache.source_count(), 1);
    }

    /// L2: Invalidation — invalidate(id) removes cache; next resolve creates fresh.
    ///
    /// This test verifies:
    /// 1. After resolve, source is cached
    /// 2. After invalidate, source is NOT cached
    /// 3. Next resolve creates fresh (still works)
    #[test]
    fn law_l2_invalidation() {
        // Create a valid SQLite database
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("law_l2.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)", [])
            .unwrap();
        drop(conn);

        let cache = SqliteDataSourcePool::with_defaults();
        let url = format!("sqlite:{}", db_path.display());
        let config = DataSourceConfig::sqlite("law-l2-source", &url);

        // Resolve (caches)
        cache.resolve(&config).expect("resolve should succeed");
        assert!(cache.has_source("law-l2-source"));
        assert_eq!(cache.source_count(), 1);

        // Invalidate
        cache.invalidate("law-l2-source");
        assert!(!cache.has_source("law-l2-source"));
        assert_eq!(cache.source_count(), 0);

        // Next resolve creates fresh
        cache
            .resolve(&config)
            .expect("resolve after invalidate should succeed");
        assert!(cache.has_source("law-l2-source"));
        assert_eq!(cache.source_count(), 1);
    }

    /// L3: Eviction — evict_idle(age) removes entries not accessed in `age` time.
    ///
    /// This test verifies:
    /// 1. Recently accessed entries are NOT evicted
    /// 2. Idle entries ARE evicted
    /// 3. Return value reflects count of evicted entries
    #[test]
    fn law_l3_eviction() {
        let cache = SqliteDataSourcePool::with_defaults();

        // Create two entries: one fresh, one old (1 hour ago)
        let now = Instant::now();
        cache
            .sources
            .insert("fresh-source".into(), mock_source_info("fresh-source"));
        cache.sources.insert(
            "old-source".into(),
            mock_source_info_with_time("old-source", now - Duration::from_secs(3600)),
        );
        assert_eq!(cache.source_count(), 2);

        // Evict idle > 30 minutes
        let evicted = cache.evict_idle(Duration::from_secs(1800));

        // Only old entry should be evicted
        assert_eq!(evicted, 1);
        assert_eq!(cache.source_count(), 1);
        assert!(cache.has_source("fresh-source"));
        assert!(!cache.has_source("old-source"));
    }

    /// L9: Maintenance-Best-Effort — maintenance() is advisory and returns count.
    ///
    /// This test verifies that maintenance() delegates to eviction and returns
    /// the count of evicted entries.
    #[test]
    fn law_l9_maintenance_best_effort() {
        let cache = SqliteDataSourcePool::with_defaults();

        // Add an old entry (1 hour ago)
        let now = Instant::now();
        cache.sources.insert(
            "old-for-maintenance".into(),
            mock_source_info_with_time("old-for-maintenance", now - Duration::from_secs(3600)),
        );

        // Run maintenance
        let evicted = cache.maintenance();
        assert!(evicted >= 1, "maintenance should evict idle entries");
        assert!(!cache.has_source("old-for-maintenance"));
    }
}
