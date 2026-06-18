//! Request-scoped corpus cache placeholder.
//!
//! This keeps the amortized "fetch once per key" helper as a generic utility
//! without tying `agent-fw-resolve` back to a concrete fuzzy-search corpus type.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use tokio::sync::Mutex;

/// Request-scoped async cache keyed by a logical corpus identifier.
pub struct CorpusCache<C> {
    inner: Mutex<HashMap<String, Arc<C>>>,
}

impl<C> CorpusCache<C> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Get the cached corpus for `key`, or fetch and cache it.
    ///
    /// The fetch closure is not run while the cache lock is held. If another
    /// task inserts the same key while the fetch is running, the first stored
    /// value wins.
    pub async fn get_or_fetch<F, Fut, E>(
        &self,
        key: impl Into<String>,
        fetch: F,
    ) -> Result<Arc<C>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<C, E>>,
    {
        let key = key.into();
        {
            let guard = self.inner.lock().await;
            if let Some(cached) = guard.get(&key) {
                return Ok(Arc::clone(cached));
            }
        }

        let fetched = Arc::new(fetch().await?);
        let mut guard = self.inner.lock().await;
        Ok(Arc::clone(guard.entry(key).or_insert(fetched)))
    }

    pub async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

impl<C> Default for CorpusCache<C> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn fetch_once_per_key() {
        let cache = CorpusCache::<Vec<String>>::new();
        let count = Arc::new(AtomicU32::new(0));

        let c1 = Arc::clone(&count);
        let first = cache
            .get_or_fetch("region", || async move {
                c1.fetch_add(1, Ordering::SeqCst);
                Ok::<_, std::convert::Infallible>(vec!["emea".to_string()])
            })
            .await
            .unwrap();

        let c2 = Arc::clone(&count);
        let second = cache
            .get_or_fetch("region", || async move {
                c2.fetch_add(1, Ordering::SeqCst);
                Ok::<_, std::convert::Infallible>(vec!["na".to_string()])
            })
            .await
            .unwrap();

        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(&*first, &vec!["emea".to_string()]);
    }
}
