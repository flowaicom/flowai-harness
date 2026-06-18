//! DataSourcePool algebraic law test harnesses.
//!
//! These harnesses verify that a `DataSourcePool` implementation satisfies
//! all documented algebraic laws.
//!
//! # Laws
//!
//! | # | Name | Statement |
//! |---|------|-----------|
//! | L1 | Get-or-Create | First `resolve(source)` caches; subsequent returns cached |
//! | L2 | Invalidation | `invalidate(id)` removes cache; next resolve creates fresh |
//! | L3 | Eviction | `evict_idle(age)` removes entries not accessed in `age` time |
//! | L4 | Has-Source-Consistency | `has_source(id)` ⟺ source is cached |
//! | L5 | Count-Accuracy | `source_count()` equals number of cached sources |
//! | L6 | Invalidation-Idempotent | `invalidate(id)` on missing source is a no-op |
//! | L7 | Clear-Empties | After `clear()`, `source_count() == 0` |
//! | L8 | Accessibility-Liveness | `is_accessible(id)` reflects current state, not cached |
//! | L9 | Maintenance-Best-Effort | `maintenance()` is advisory; returns count of resources freed |
//!
//! # Usage
//!
//! ```ignore
//! use agent_fw_test::data_source_pool_laws;
//! use agent_fw_interpreter::SqliteDataSourcePool;
//!
//! #[test]
//! fn my_pool_satisfies_laws() {
//!     let pool = SqliteDataSourcePool::with_defaults();
//!     data_source_pool_laws::test_all(&pool);
//! }
//! ```
//!
//! # Implementation-Specific Tests
//!
//! Laws L1-L3 and L8-L9 require actual file resolution or time-based behavior.
//! These are tested in implementation crates rather than here, since they need
//! real filesystem access.

use agent_fw_algebra::DataSourcePool;

/// Run all DataSourcePool laws against the given implementation.
///
/// Note: This tests laws L4-L7 which don't require actual file resolution.
/// Laws L1-L3 require implementations that can resolve real sources,
/// which should be tested separately with real files.
pub fn test_all(pool: &dyn DataSourcePool) {
    law_has_source_consistency(pool);
    law_count_accuracy(pool);
    law_invalidation_idempotent(pool);
    law_clear_empties(pool);
}

/// L4: Has-Source-Consistency — `has_source(id)` is true iff source is cached.
///
/// This test uses direct cache manipulation where possible.
pub fn law_has_source_consistency(pool: &dyn DataSourcePool) {
    // Non-existent source should not be cached
    assert!(
        !pool.has_source("law-l4-nonexistent"),
        "L4: has_source must return false for non-cached source"
    );

    // If we can add a mock entry, verify consistency
    // (implementation-specific; this tests the trait contract)
}

/// L5: Count-Accuracy — `source_count()` equals number of cached sources.
pub fn law_count_accuracy(pool: &dyn DataSourcePool) {
    // After clearing, count should be 0
    pool.clear();
    assert_eq!(
        pool.source_count(),
        0,
        "L5: source_count must be 0 after clear"
    );

    // Invalidation of non-existent should not change count
    pool.invalidate("law-l5-nonexistent");
    assert_eq!(
        pool.source_count(),
        0,
        "L5: invalidating non-existent must not change count"
    );
}

/// L6: Invalidation-Idempotent — `invalidate(id)` on missing source is a no-op.
pub fn law_invalidation_idempotent(pool: &dyn DataSourcePool) {
    pool.clear();

    // Multiple invalidations of non-existent should not panic
    pool.invalidate("law-l6-idempotent");
    pool.invalidate("law-l6-idempotent");
    pool.invalidate("law-l6-idempotent");

    // Count should still be 0
    assert_eq!(
        pool.source_count(),
        0,
        "L6: repeated invalidation of missing must be no-op"
    );
}

/// L7: Clear-Empties — After `clear()`, `source_count() == 0`.
pub fn law_clear_empties(pool: &dyn DataSourcePool) {
    // Clear should work even on empty pool
    pool.clear();
    assert_eq!(
        pool.source_count(),
        0,
        "L7: clear on empty must result in 0"
    );

    // Second clear is also fine
    pool.clear();
    assert_eq!(
        pool.source_count(),
        0,
        "L7: repeated clear must still result in 0"
    );
}

/// L3: Eviction — `evict_idle(age)` removes entries not accessed in `age` time.
///
/// This test requires the implementation to support adding mock entries
/// with custom timestamps. Use this as a template for implementation-specific tests.
///
/// ```ignore
/// fn law_eviction(pool: &dyn DataSourcePool) {
///     pool.clear();
///
///     // Add entry with old access time (implementation-specific)
///     // ... add mock entry with timestamp = now - 1 hour ...
///
///     // Evict idle > 30 minutes
///     let evicted = pool.evict_idle(Duration::from_secs(1800));
///
///     assert!(evicted >= 1, "L3: must evict idle entries");
///     assert!(!pool.has_source("idle-source"), "L3: idle source must be removed");
/// }
/// ```
pub fn law_eviction_template() {
    // Template for implementation-specific test
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_fw_algebra::{DataSourceConfig, PooledConnection};
    use std::collections::HashSet;
    use std::sync::RwLock;
    use std::time::Duration;

    /// Stub DataSourcePool with interior mutability so law tests are non-vacuous.
    ///
    /// Uses `RwLock` (required for `Send + Sync` bound on `DataSourcePool`) to
    /// actually track cached sources, ensuring `clear()`, `invalidate()`,
    /// `has_source()`, and `source_count()` exercise real state transitions
    /// rather than testing empty-set identity.
    struct StubPool {
        sources: RwLock<HashSet<String>>,
    }

    impl StubPool {
        fn new() -> Self {
            Self {
                sources: RwLock::new(HashSet::new()),
            }
        }

        /// Manually insert a source ID into the cache (for test setup).
        fn insert(&self, id: &str) {
            self.sources.write().unwrap().insert(id.to_string());
        }
    }

    impl DataSourcePool for StubPool {
        /// L1 compliant: caches the source ID and returns Ok with the config.
        fn resolve(
            &self,
            source: &DataSourceConfig,
        ) -> Result<agent_fw_algebra::ResolvedSource, agent_fw_algebra::DataSourcePoolError>
        {
            self.sources.write().unwrap().insert(source.id.clone());
            Ok(agent_fw_algebra::ResolvedSource {
                config: source.clone(),
                resolved_path: None,
            })
        }

        fn open_connection(
            &self,
            _source: &DataSourceConfig,
        ) -> Result<PooledConnection, agent_fw_algebra::DataSourcePoolError> {
            Err(agent_fw_algebra::DataSourcePoolError::NotFound(
                "stub: open_connection not supported".to_string(),
            ))
        }

        fn is_accessible(&self, source_id: &str) -> bool {
            self.sources.read().unwrap().contains(source_id)
        }

        fn invalidate(&self, source_id: &str) {
            self.sources.write().unwrap().remove(source_id);
        }

        fn evict_idle(&self, _max_age: Duration) -> usize {
            0
        }

        fn source_count(&self) -> usize {
            self.sources.read().unwrap().len()
        }

        fn has_source(&self, source_id: &str) -> bool {
            self.sources.read().unwrap().contains(source_id)
        }

        fn clear(&self) {
            self.sources.write().unwrap().clear();
        }

        fn maintenance(&self) -> usize {
            0
        }
    }

    #[test]
    fn stub_pool_passes_all_laws() {
        let pool = StubPool::new();
        test_all(&pool);
    }

    /// Verify that law tests actually exercise state transitions (non-vacuity).
    #[test]
    fn law_tests_are_non_vacuous() {
        let pool = StubPool::new();

        // Populate the pool so clear() has something to clear
        pool.insert("src-1");
        pool.insert("src-2");
        assert_eq!(pool.source_count(), 2);
        assert!(pool.has_source("src-1"));

        // L7: clear empties a non-empty pool
        pool.clear();
        assert_eq!(pool.source_count(), 0);
        assert!(!pool.has_source("src-1"));

        // Re-populate for invalidation test
        pool.insert("src-3");
        assert_eq!(pool.source_count(), 1);

        // L6: invalidate removes the entry
        pool.invalidate("src-3");
        assert_eq!(pool.source_count(), 0);
        assert!(!pool.has_source("src-3"));

        // L6: invalidate on missing is idempotent (no panic, no count change)
        pool.invalidate("src-3");
        assert_eq!(pool.source_count(), 0);

        // Still passes all law harnesses
        pool.insert("src-4");
        pool.insert("src-5");
        test_all(&pool); // clear() inside law_count_accuracy resets to 0
    }
}
