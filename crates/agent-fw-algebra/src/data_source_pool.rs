//! DataSourcePool algebra for connection configuration caching.
//!
//! Provides efficient configuration caching and lazy connection creation for
//! multiple data sources. This trait abstracts the connection pool/cache
//! concept, allowing different implementations (in-memory cache, distributed
//! cache, etc.).
//!
//! # Laws
//!
//! Implementations must satisfy these laws:
//!
//! - **L1. Get-or-Create**: First `resolve(source)` caches and returns config;
//!   subsequent calls return cached config. **Atomic**: no race between check and insert.
//!
//! - **L2. Invalidation**: `invalidate(id)` removes cached config; next
//!   `resolve(source)` creates fresh config.
//!
//! - **L3. Eviction**: `evict_idle(age)` removes entries not accessed in `age`
//!   time; returns count of evicted entries.
//!
//! - **L4. Has-Source-Consistency**: `has_source(id)` ⟺ source is cached.
//!
//! - **L5. Count-Accuracy**: `source_count()` equals number of cached sources.
//!
//! - **L6. Invalidation-Idempotent**: `invalidate(id)` called multiple times
//!   has same effect as calling once (no error on missing).
//!
//! - **L7. Clear-Empties**: `clear()` results in `source_count() == 0`.
//!
//! - **L8. Accessibility-Liveness**: `is_accessible(id)` reflects current state,
//!   not cached state. Must check actual file/connection existence.
//!
//! - **L9. Maintenance-Best-Effort**: `maintenance()` is safe to call repeatedly;
//!   each call evicts entries that have become idle since the last call.
//!
//! # Object Safety
//!
//! This trait is object-safe and can be used as `Arc<dyn DataSourcePool>`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

// =============================================================================
// Error Type
// =============================================================================

/// Errors from data source pool operations.
///
/// This enum is `#[non_exhaustive]` to allow adding new error variants
/// without breaking changes. Consumers should handle the wildcard case
/// or use `matches!` for specific variants.
#[derive(Debug, Clone, Error)]
#[non_exhaustive]
pub enum DataSourcePoolError {
    /// Connection failed.
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Invalid configuration.
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// Source not found.
    #[error("Source not found: {0}")]
    NotFound(String),

    /// Unsupported database type.
    #[error("Unsupported database type: {0}")]
    UnsupportedType(String),

    /// Database file not found.
    #[error("Database file not found: {0}")]
    FileNotFound(PathBuf),

    /// Lock poisoned (internal synchronization error).
    #[error("Lock poisoned for source: {0}")]
    LockPoisoned(String),
}

// =============================================================================
// Configuration Types
// =============================================================================

/// Supported database types.
pub use agent_fw_core::DatabaseType;

/// Configuration for a data source connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceConfig {
    /// Unique identifier for this data source.
    pub id: String,
    /// Database type (sqlite, postgresql, mysql).
    pub db_type: DatabaseType,
    /// Connection URL.
    pub url: String,
    /// Maximum connections in the pool (future: for PostgreSQL).
    pub max_connections: Option<u32>,
    /// Connection timeout in seconds.
    pub connect_timeout_secs: Option<u64>,
}

impl DataSourceConfig {
    /// Create a new SQLite config.
    pub fn sqlite(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            db_type: DatabaseType::SQLite,
            url: path.into(),
            max_connections: None,
            connect_timeout_secs: None,
        }
    }

    /// Create a new PostgreSQL config.
    pub fn postgresql(id: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            db_type: DatabaseType::PostgreSQL,
            url: url.into(),
            max_connections: Some(10),
            connect_timeout_secs: Some(30),
        }
    }
}

/// Resolved information about a data source.
///
/// This is a pure data structure - accessibility is NOT cached here
/// because it can become stale. Use `is_accessible()` on the pool
/// to check current liveness.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedSource {
    /// The source configuration.
    pub config: DataSourceConfig,
    /// Resolved file path (for SQLite).
    pub resolved_path: Option<PathBuf>,
}

// =============================================================================
// Algebra Trait
// =============================================================================

/// Connection configuration cache for data sources.
///
/// This trait abstracts the caching of data source configurations and
/// connection creation. Implementations may cache in memory, use
/// distributed caching, or create connections on every call.
///
/// # Thread Safety
///
/// All operations must be safe for concurrent access from multiple threads.
/// The `resolve` method must be atomic - no race between check and insert.
///
/// # Example
///
/// ```ignore
/// use agent_fw_algebra::{DataSourceConfig, DataSourcePool};
///
/// fn use_pool(pool: &dyn DataSourcePool) -> Result<(), DataSourcePoolError> {
///     let config = DataSourceConfig::sqlite("my-db", "sqlite:/path/to/db.sqlite");
///
///     // Resolve (caches internally, atomic)
///     let resolved = pool.resolve(&config)?;
///
///     // Open a connection
///     let conn = pool.open_connection(&config)?;
///
///     // Check accessibility (live check, not cached)
///     if pool.is_accessible("my-db") {
///         // ...
///     }
///     Ok(())
/// }
/// ```
pub trait DataSourcePool: Send + Sync {
    // =========================================================================
    // Core Operations
    // =========================================================================

    /// Resolve a data source configuration.
    ///
    /// Returns cached info if available, otherwise resolves and caches.
    /// This operation is **atomic** - no race condition between check and insert.
    ///
    /// # Law L1: Get-or-Create
    /// First call caches; subsequent calls return cached. Atomic.
    fn resolve(&self, source: &DataSourceConfig) -> Result<ResolvedSource, DataSourcePoolError>;

    /// Open a connection to the data source.
    ///
    /// Creates a fresh connection on each call. For SQLite, this is efficient
    /// because SQLite manages its own internal connection pooling.
    ///
    /// # Errors
    /// Returns `ConnectionFailed` if the connection cannot be established.
    fn open_connection(
        &self,
        source: &DataSourceConfig,
    ) -> Result<PooledConnection, DataSourcePoolError>;

    /// Check if a source is currently accessible.
    ///
    /// This performs a **live check** - it does not return cached state.
    /// For SQLite, this checks if the file exists. For PostgreSQL, this
    /// would attempt a ping.
    ///
    /// # Law L8: Accessibility-Liveness
    /// Must reflect current state, not cached state.
    fn is_accessible(&self, source_id: &str) -> bool;

    // =========================================================================
    // Cache Management
    // =========================================================================

    /// Invalidate a cached source.
    ///
    /// # Law L2: Invalidation
    /// Removes cached config; next resolve creates fresh.
    ///
    /// # Law L6: Invalidation-Idempotent
    /// Calling on non-existent source is a no-op.
    fn invalidate(&self, source_id: &str);

    /// Evict sources not accessed in the last `max_age` duration.
    ///
    /// # Law L3: Eviction
    /// Returns count of evicted entries.
    fn evict_idle(&self, max_age: Duration) -> usize;

    /// Get the number of cached sources.
    ///
    /// # Law L5: Count-Accuracy
    /// Must equal the actual number of cached sources.
    fn source_count(&self) -> usize;

    /// Check if a source is cached.
    ///
    /// # Law L4: Has-Source-Consistency
    /// `has_source(id)` is true iff source is cached.
    fn has_source(&self, source_id: &str) -> bool;

    /// Clear all cached sources.
    ///
    /// # Law L7: Clear-Empties
    /// After `clear()`, `source_count() == 0`.
    fn clear(&self);

    /// Run periodic maintenance operations.
    ///
    /// This method should be called periodically (e.g., by a background task)
    /// to perform housekeeping operations like evicting idle connections.
    ///
    /// # Returns
    /// The number of cache entries evicted or resources freed.
    ///
    /// # Law L9: Maintenance-Best-Effort
    /// This method is safe to call repeatedly. Each call evicts entries that
    /// have become idle since the last call. Returns 0 if no entries were
    /// eligible for eviction.
    ///
    /// # Example
    /// ```ignore
    /// // In a background task:
    /// loop {
    ///     tokio::time::sleep(Duration::from_secs(60)).await;
    ///     let evicted = pool.maintenance();
    ///     if evicted > 0 {
    ///         tracing::debug!("Evicted {} idle sources", evicted);
    ///     }
    /// }
    /// ```
    fn maintenance(&self) -> usize;
}

/// A pooled connection to a data source.
///
/// This enum allows the trait to return different connection types
/// depending on the database backend.
pub enum PooledConnection {
    /// SQLite connection.
    Sqlite(rusqlite::Connection),
    /// PostgreSQL connection (type-erased pool handle).
    ///
    /// The inner `Box<dyn Any + Send>` holds a concrete connection pool
    /// (e.g. `sqlx::PgPool`). Consumers downcast via [`Self::pg_downcast`].
    Postgresql(Box<dyn std::any::Any + Send>),
}

impl std::fmt::Debug for PooledConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PooledConnection::Sqlite(conn) => f.debug_tuple("Sqlite").field(conn).finish(),
            PooledConnection::Postgresql(_) => {
                f.debug_tuple("Postgresql").field(&"<pool>").finish()
            }
        }
    }
}

impl PooledConnection {
    /// Get the SQLite connection, if this is one.
    pub fn as_sqlite(&self) -> Option<&rusqlite::Connection> {
        match self {
            PooledConnection::Sqlite(conn) => Some(conn),
            PooledConnection::Postgresql(_) => None,
        }
    }

    /// Get the SQLite connection mutably, if this is one.
    pub fn as_sqlite_mut(&mut self) -> Option<&mut rusqlite::Connection> {
        match self {
            PooledConnection::Sqlite(conn) => Some(conn),
            PooledConnection::Postgresql(_) => None,
        }
    }

    /// Convert into SQLite connection, if this is one.
    pub fn into_sqlite(self) -> Option<rusqlite::Connection> {
        match self {
            PooledConnection::Sqlite(conn) => Some(conn),
            PooledConnection::Postgresql(_) => None,
        }
    }

    /// Downcast the PostgreSQL handle to a concrete type.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // In code that has sqlx available:
    /// if let Some(pool) = conn.pg_downcast::<sqlx::PgPool>() {
    ///     let rows = sqlx::query("SELECT 1").fetch_all(pool).await?;
    /// }
    /// ```
    pub fn pg_downcast<T: 'static>(&self) -> Option<&T> {
        match self {
            PooledConnection::Postgresql(inner) => inner.downcast_ref::<T>(),
            _ => None,
        }
    }

    /// Check if this is a PostgreSQL connection.
    pub fn is_postgresql(&self) -> bool {
        matches!(self, PooledConnection::Postgresql(_))
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Parse SQLite path from a URL.
///
/// Supports:
/// - `sqlite:path/to/db.sqlite`
/// - `sqlite:///absolute/path/to/db.sqlite`
/// - `sqlite://relative/path/to/db.sqlite`
/// - Plain path: `path/to/db.sqlite`
pub fn parse_sqlite_path(url: &str) -> Option<PathBuf> {
    if let Some(path) = url.strip_prefix("sqlite:///") {
        // sqlite:///absolute/path → absolute path (the /// means root)
        Some(PathBuf::from(format!("/{path}")))
    } else if let Some(path) = url.strip_prefix("sqlite://") {
        Some(PathBuf::from(path))
    } else if let Some(path) = url.strip_prefix("sqlite:") {
        Some(PathBuf::from(path))
    } else if !url.contains("://") {
        Some(PathBuf::from(url))
    } else {
        None
    }
}

/// Parse SQLite path from a URL, resolving relative paths against a root.
///
/// This is useful when SQLite URLs are relative to a project root.
///
/// # Example
///
/// ```
/// use std::path::Path;
/// use agent_fw_algebra::parse_sqlite_path_with_root;
///
/// let root = Path::new("/project");
///
/// // Relative path gets resolved
/// let path = parse_sqlite_path_with_root("sqlite:seeds/test.db", Some(root));
/// assert_eq!(path, Some(std::path::PathBuf::from("/project/seeds/test.db")));
///
/// // Absolute path stays absolute
/// let path = parse_sqlite_path_with_root("sqlite:///absolute/path.db", Some(root));
/// assert_eq!(path, Some(std::path::PathBuf::from("/absolute/path.db")));
/// ```
pub fn parse_sqlite_path_with_root(url: &str, root: Option<&Path>) -> Option<PathBuf> {
    let path = parse_sqlite_path(url)?;

    if path.is_absolute() {
        Some(path)
    } else if let Some(r) = root {
        Some(r.join(path))
    } else {
        Some(path)
    }
}

/// Reconstruct a SQLite URL with project root for relative paths.
///
/// This is useful for converting relative SQLite URLs to absolute URLs
/// when storing or passing to systems that need absolute paths.
///
/// # Example
///
/// ```
/// use std::path::Path;
/// use agent_fw_algebra::resolve_sqlite_url_with_root;
///
/// let root = Path::new("/project");
///
/// // Relative path gets resolved
/// let url = resolve_sqlite_url_with_root("sqlite:seeds/test.db", Some(root));
/// assert_eq!(url, "sqlite:/project/seeds/test.db");
///
/// // Absolute path stays the same
/// let url = resolve_sqlite_url_with_root("sqlite:///absolute/path.db", Some(root));
/// assert_eq!(url, "sqlite:///absolute/path.db");
/// ```
pub fn resolve_sqlite_url_with_root(url: &str, root: Option<&Path>) -> String {
    // Handle sqlite:// prefix with relative path
    if let Some(path) = url.strip_prefix("sqlite://") {
        let path_buf = PathBuf::from(path);
        if path_buf.is_relative() {
            if let Some(r) = root {
                return format!("sqlite://{}", r.join(path).to_string_lossy());
            }
        }
    }
    // Handle sqlite: prefix with relative path
    if let Some(path) = url.strip_prefix("sqlite:") {
        // Check if it's already an absolute path or has :// scheme
        if !path.starts_with('/') && !path.contains("://") {
            let path_buf = PathBuf::from(path);
            if path_buf.is_relative() {
                if let Some(r) = root {
                    return format!("sqlite:{}", r.join(path).to_string_lossy());
                }
            }
        }
    }
    url.to_string()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sqlite_path_variants() {
        assert_eq!(
            parse_sqlite_path("sqlite:/path/to/db.sqlite"),
            Some(PathBuf::from("/path/to/db.sqlite"))
        );
        assert_eq!(
            parse_sqlite_path("sqlite:///abs/path.db"),
            Some(PathBuf::from("/abs/path.db"))
        );
        assert_eq!(
            parse_sqlite_path("sqlite://relative/path.db"),
            Some(PathBuf::from("relative/path.db"))
        );
        assert_eq!(
            parse_sqlite_path("relative/path.db"),
            Some(PathBuf::from("relative/path.db"))
        );
        assert_eq!(parse_sqlite_path("postgresql://host/db"), None);
    }

    #[test]
    fn parse_sqlite_path_with_root_resolves_relative() {
        let root = Path::new("/project");

        // Relative path gets resolved
        assert_eq!(
            parse_sqlite_path_with_root("sqlite:seeds/test.db", Some(root)),
            Some(PathBuf::from("/project/seeds/test.db"))
        );

        // Relative with sqlite:// prefix
        assert_eq!(
            parse_sqlite_path_with_root("sqlite://relative/path.db", Some(root)),
            Some(PathBuf::from("/project/relative/path.db"))
        );

        // Absolute path stays absolute
        assert_eq!(
            parse_sqlite_path_with_root("sqlite:///absolute/path.db", Some(root)),
            Some(PathBuf::from("/absolute/path.db"))
        );

        // No root - relative stays relative
        assert_eq!(
            parse_sqlite_path_with_root("sqlite:seeds/test.db", None),
            Some(PathBuf::from("seeds/test.db"))
        );
    }

    #[test]
    fn resolve_sqlite_url_with_root_transforms_urls() {
        let root = Path::new("/project");

        // Relative path with sqlite: prefix
        assert_eq!(
            resolve_sqlite_url_with_root("sqlite:seeds/test.db", Some(root)),
            "sqlite:/project/seeds/test.db"
        );

        // Relative path with sqlite:// prefix
        assert_eq!(
            resolve_sqlite_url_with_root("sqlite://seeds/test.db", Some(root)),
            "sqlite:///project/seeds/test.db"
        );

        // Absolute path stays the same
        assert_eq!(
            resolve_sqlite_url_with_root("sqlite:///absolute/path.db", Some(root)),
            "sqlite:///absolute/path.db"
        );

        // Non-sqlite URL stays the same
        assert_eq!(
            resolve_sqlite_url_with_root("postgresql://localhost/db", Some(root)),
            "postgresql://localhost/db"
        );
    }

    #[test]
    fn database_type_from_str() {
        assert_eq!(
            "sqlite".parse::<DatabaseType>().unwrap(),
            DatabaseType::SQLite
        );
        assert_eq!(
            "postgresql".parse::<DatabaseType>().unwrap(),
            DatabaseType::PostgreSQL
        );
        assert_eq!(
            "postgres".parse::<DatabaseType>().unwrap(),
            DatabaseType::PostgreSQL
        );
        assert_eq!(
            "mysql".parse::<DatabaseType>().unwrap(),
            DatabaseType::MySQL
        );
    }

    #[test]
    fn data_source_config_convenience() {
        let sqlite = DataSourceConfig::sqlite("test", "/path/to/db.sqlite");
        assert_eq!(sqlite.id, "test");
        assert_eq!(sqlite.db_type, DatabaseType::SQLite);

        let pg = DataSourceConfig::postgresql("pg-test", "postgresql://localhost/db");
        assert_eq!(pg.id, "pg-test");
        assert_eq!(pg.db_type, DatabaseType::PostgreSQL);
        assert_eq!(pg.max_connections, Some(10));
    }

    #[test]
    fn data_source_config_serde() {
        let config = DataSourceConfig {
            id: "source-1".into(),
            db_type: DatabaseType::PostgreSQL,
            url: "postgresql://localhost/db".into(),
            max_connections: Some(20),
            connect_timeout_secs: Some(10),
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"id\":\"source-1\""));
        assert!(json.contains("\"dbType\":\"postgresql\""));

        let parsed: DataSourceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "source-1");
        assert_eq!(parsed.db_type, DatabaseType::PostgreSQL);
    }
}
