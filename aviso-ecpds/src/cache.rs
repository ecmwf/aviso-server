use crate::client::EcpdsError;
use moka::future::Cache;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

/// Whether a [`DestinationCache::try_get_or_fetch`] call was satisfied
/// from cache or required an upstream fetch. The route layer records
/// this as a Prometheus label so on-call can see the cache hit ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheOutcome {
    /// The destination list was served from the in-process cache; no
    /// upstream ECPDS call was made for this request.
    Hit,
    /// The destination list was not in the cache; an upstream ECPDS
    /// call was made (single-flight: at most one per missing key
    /// across concurrent waiters).
    Miss,
}

/// Per-username cache of authorised ECPDS destination lists.
///
/// Backed by [`moka::future::Cache`], which provides:
/// - native time-to-live expiration (`time_to_live`),
/// - bounded size with TinyLFU eviction (`max_capacity`),
/// - **built-in single-flight via `try_get_with_by_ref`**: under
///   concurrent cache misses for the same key, only one task runs the
///   `fetch` future; the rest await its result. This prevents the
///   thundering-herd that would otherwise hit the ECPDS servers when
///   many SSE clients reconnect simultaneously.
///
/// **Errors are not cached.** When `fetch` returns `Err(_)`, the error
/// is fanned out to all current waiters but no entry is inserted, so
/// the next request retries upstream. This is deliberate: caching
/// failures would convert a transient outage into a window of stale
/// 503s for downstream clients.
///
/// **Panic / cancel semantics.** If the `fetch` future panics or is
/// cancelled, moka propagates the panic to current waiters and inserts
/// nothing. The next request retries from scratch. We treat this as an
/// edge case for unwindable runtime panics; the route handler's catch-
/// all error path covers the resulting [`EcpdsError`] mapping.
pub struct DestinationCache {
    cache: Cache<String, Arc<Vec<String>>>,
}

impl DestinationCache {
    /// Build a cache with the given TTL and maximum entry count.
    pub fn new(ttl_seconds: u64, max_entries: u64) -> Self {
        let cache = Cache::builder()
            .time_to_live(Duration::from_secs(ttl_seconds))
            .max_capacity(max_entries)
            .build();
        Self { cache }
    }

    /// Look up `username`, falling back to `fetch` on a cache miss.
    ///
    /// Concurrent calls for the same `username` are coalesced: at most
    /// one `fetch` future runs per missing key; all waiters share its
    /// result. Successful fetches are inserted into the cache;
    /// failures are not.
    ///
    /// Returns the destination list (shared via `Arc` to avoid cloning
    /// the vector for every waiter) and a [`CacheOutcome`] indicating
    /// whether the call was a cache hit or miss.
    pub async fn try_get_or_fetch<F, Fut>(
        &self,
        username: &str,
        fetch: F,
    ) -> Result<(Arc<Vec<String>>, CacheOutcome), EcpdsError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<String>, EcpdsError>>,
    {
        if let Some(cached) = self.cache.get(username).await {
            return Ok((cached, CacheOutcome::Hit));
        }

        let inserted = self
            .cache
            .try_get_with_by_ref(username, async move { fetch().await.map(Arc::new) })
            .await
            .map_err(|arc_err: Arc<EcpdsError>| match Arc::try_unwrap(arc_err) {
                Ok(err) => err,
                Err(shared) => clone_ecpds_error(&shared),
            })?;

        Ok((inserted, CacheOutcome::Miss))
    }

    /// Number of entries currently held by the cache.
    ///
    /// Reads from moka's authoritative count after eviction passes so
    /// the value is honest (not a hand-maintained counter that would
    /// drift past expiry).
    pub fn entry_count(&self) -> u64 {
        self.cache.entry_count()
    }

    #[cfg(test)]
    pub(crate) async fn set(&self, username: &str, destinations: Vec<String>) {
        self.cache
            .insert(username.to_string(), Arc::new(destinations))
            .await;
    }

    #[cfg(test)]
    pub(crate) async fn get(&self, username: &str) -> Option<Vec<String>> {
        self.cache.get(username).await.map(|arc| (*arc).clone())
    }
}

/// Reconstruct an [`EcpdsError`] from a shared reference. Used when
/// moka's single-flight fans out the same error to multiple waiters
/// and only the last waiter receives the unique `Arc`.
///
/// Variants whose source type is not `Clone` (wrapped
/// `reqwest::Error`, `url::ParseError`) collapse to
/// [`EcpdsError::ServiceUnavailable { .. }`] — the route layer maps that to
/// HTTP 503, which is the right semantics when an upstream call
/// failed. Collapsing to `AccessDenied` would incorrectly surface as
/// HTTP 403 to waiters, suggesting the user lacks permission when
/// the real cause was a service-side error.
fn clone_ecpds_error(err: &EcpdsError) -> EcpdsError {
    use crate::client::FetchOutcome;
    match err {
        EcpdsError::ServiceUnavailable { fetch_outcome } => EcpdsError::ServiceUnavailable {
            fetch_outcome: *fetch_outcome,
        },
        EcpdsError::AccessDenied { reason, message } => EcpdsError::AccessDenied {
            reason: *reason,
            message: message.clone(),
        },
        EcpdsError::Http {
            server_index,
            status,
            message,
        } => EcpdsError::Http {
            server_index: *server_index,
            status: *status,
            message: message.clone(),
        },
        EcpdsError::InvalidResponse {
            server_index,
            message,
        } => EcpdsError::InvalidResponse {
            server_index: *server_index,
            message: message.clone(),
        },
        EcpdsError::HttpClientBuild(_) | EcpdsError::InvalidServerUrl { .. } => {
            EcpdsError::ServiceUnavailable {
                fetch_outcome: FetchOutcome::Unreachable,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn cache_miss_returns_none() {
        let cache = DestinationCache::new(300, 1000);
        assert_eq!(cache.get("unknown_user").await, None);
    }

    #[tokio::test]
    async fn cache_hit_returns_stored_destinations() {
        let cache = DestinationCache::new(300, 1000);
        cache
            .set("john", vec!["CIP".to_string(), "FOO".to_string()])
            .await;
        let result = cache.get("john").await;
        assert_eq!(result, Some(vec!["CIP".to_string(), "FOO".to_string()]));
    }

    #[tokio::test]
    async fn cache_entry_expires_after_ttl() {
        let cache = DestinationCache::new(1, 1000);
        cache.set("john", vec!["CIP".to_string()]).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(cache.get("john").await, None);
    }

    #[tokio::test]
    async fn cache_overwrite_replaces_value() {
        let cache = DestinationCache::new(300, 1000);
        cache.set("john", vec!["CIP".to_string()]).await;
        cache.set("john", vec!["FOO".to_string()]).await;
        assert_eq!(cache.get("john").await, Some(vec!["FOO".to_string()]));
    }

    #[tokio::test]
    async fn cache_independent_entries_per_user() {
        let cache = DestinationCache::new(300, 1000);
        cache.set("alice", vec!["CIP".to_string()]).await;
        cache.set("bob", vec!["FOO".to_string()]).await;
        assert_eq!(cache.get("alice").await, Some(vec!["CIP".to_string()]));
        assert_eq!(cache.get("bob").await, Some(vec!["FOO".to_string()]));
    }

    #[tokio::test]
    async fn try_get_or_fetch_hit_does_not_call_fetch() {
        let cache = DestinationCache::new(300, 1000);
        cache.set("john", vec!["CIP".to_string()]).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let (result, outcome) = cache
            .try_get_or_fetch("john", move || {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                async { Ok(vec!["BAD".to_string()]) }
            })
            .await
            .expect("must succeed");
        assert_eq!(result.as_ref(), &vec!["CIP".to_string()]);
        assert_eq!(outcome, CacheOutcome::Hit);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn try_get_or_fetch_miss_calls_fetch_and_caches() {
        let cache = DestinationCache::new(300, 1000);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let (result, outcome) = cache
            .try_get_or_fetch("alice", move || {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                async { Ok(vec!["CIP".to_string()]) }
            })
            .await
            .expect("must succeed");
        assert_eq!(result.as_ref(), &vec!["CIP".to_string()]);
        assert_eq!(outcome, CacheOutcome::Miss);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let calls_clone = calls.clone();
        let (result2, outcome2) = cache
            .try_get_or_fetch("alice", move || {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                async { Ok(vec!["IGNORED".to_string()]) }
            })
            .await
            .expect("must succeed");
        assert_eq!(result2.as_ref(), &vec!["CIP".to_string()]);
        assert_eq!(outcome2, CacheOutcome::Hit);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn single_flight_coalesces_concurrent_misses() {
        let cache = Arc::new(DestinationCache::new(300, 1000));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..10 {
            let cache = cache.clone();
            let calls = calls.clone();
            handles.push(tokio::spawn(async move {
                cache
                    .try_get_or_fetch("racer", move || {
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            Ok(vec!["CIP".to_string()])
                        }
                    })
                    .await
            }));
        }
        for handle in handles {
            let (result, _outcome) = handle.await.unwrap().expect("must succeed");
            assert_eq!(result.as_ref(), &vec!["CIP".to_string()]);
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "fetch must have run exactly once across concurrent waiters"
        );
    }

    #[tokio::test]
    async fn errors_are_not_cached() {
        let cache = DestinationCache::new(300, 1000);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let err = cache
            .try_get_or_fetch("doomed", move || {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                async {
                    Err(EcpdsError::ServiceUnavailable {
                        fetch_outcome: crate::client::FetchOutcome::Unreachable,
                    })
                }
            })
            .await
            .expect_err("must error");
        assert!(matches!(err, EcpdsError::ServiceUnavailable { .. }));
        let calls_clone = calls.clone();
        let _ok = cache
            .try_get_or_fetch("doomed", move || {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                async { Ok(vec!["CIP".to_string()]) }
            })
            .await
            .expect("must succeed");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "second call must reach upstream because the error was not cached"
        );
    }

    #[tokio::test]
    async fn concurrent_error_fan_out_yields_503_to_all_waiters() {
        let cache = Arc::new(DestinationCache::new(300, 1000));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..10 {
            let cache = cache.clone();
            let calls = calls.clone();
            handles.push(tokio::spawn(async move {
                cache
                    .try_get_or_fetch("doomed-racer", move || {
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            Err(EcpdsError::ServiceUnavailable {
                                fetch_outcome: crate::client::FetchOutcome::Unreachable,
                            })
                        }
                    })
                    .await
            }));
        }
        for handle in handles {
            let result = handle.await.unwrap();
            assert!(
                matches!(result, Err(EcpdsError::ServiceUnavailable { .. })),
                "all waiters must observe ServiceUnavailable, never AccessDenied"
            );
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "fetch must run exactly once even when it fails"
        );
    }

    #[tokio::test]
    async fn entry_count_reflects_size() {
        let cache = DestinationCache::new(300, 1000);
        cache.set("alice", vec!["A".to_string()]).await;
        cache.set("bob", vec!["B".to_string()]).await;
        cache.cache.run_pending_tasks().await;
        assert_eq!(cache.entry_count(), 2);
    }
}
