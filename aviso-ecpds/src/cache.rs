use crate::client::{EcpdsError, FetchOutcome};
use moka::future::Cache;
use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Whether a [`DestinationCache::try_get_or_fetch`] call was satisfied
/// from cache or required an upstream fetch. The route layer records
/// this as a Prometheus label so on-call can see the cache hit ratio,
/// and uses `MissFetched` to record `aviso_ecpds_fetch_total` exactly
/// once per upstream call (not once per coalesced waiter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheOutcome {
    /// Served from cache; no upstream call ran for this request.
    Hit,
    /// Cache was empty for this key but another concurrent caller
    /// was already fetching, so this caller waited on the in-flight
    /// fetch and received its result. No upstream call ran on this
    /// caller's behalf. This also covers the TOCTOU window where the
    /// initial `get()` returned `None` but the value was inserted
    /// before this caller entered single-flight.
    MissCoalesced,
    /// This caller ran the upstream fetch itself. The merged
    /// [`FetchOutcome`] across all configured ECPDS servers under
    /// the active partial-outage policy is reported here so the
    /// route layer can label `aviso_ecpds_fetch_total` precisely
    /// (e.g. `success`, `http_5xx`, partial-failure cases under
    /// `any_success`).
    MissFetched {
        /// Merged outcome across all servers for this fetch.
        fetch_outcome: FetchOutcome,
    },
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
    cache: Cache<String, Arc<HashSet<String>>>,
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
    /// `fetch` returns the destination list paired with the merged
    /// [`FetchOutcome`] for the fetch attempt. Only the destination
    /// list is cached; the outcome is reported back to the caller
    /// that ran the fetch via [`CacheOutcome::MissFetched`].
    ///
    /// Returns `(CacheOutcome, Result<...>)` regardless of whether
    /// the fetch succeeded so the route layer can record cache and
    /// fetch metrics on every code path (allow, deny, unavailable).
    /// Without this, denied requests after a cache miss never
    /// contribute to `aviso_ecpds_cache_misses_total`, and concurrent
    /// waiters on a failing fetch all increment
    /// `aviso_ecpds_fetch_total` even though only one upstream call
    /// happened.
    pub async fn try_get_or_fetch<F, Fut>(
        &self,
        username: &str,
        fetch: F,
    ) -> (CacheOutcome, Result<Arc<HashSet<String>>, EcpdsError>)
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(HashSet<String>, FetchOutcome), EcpdsError>>,
    {
        if let Some(cached) = self.cache.get(username).await {
            return (CacheOutcome::Hit, Ok(cached));
        }

        let fetched = Arc::new(AtomicBool::new(false));
        let fetched_in_closure = fetched.clone();
        let outcome_slot: Arc<Mutex<Option<FetchOutcome>>> = Arc::new(Mutex::new(None));
        let outcome_writer = outcome_slot.clone();
        let result = self
            .cache
            .try_get_with_by_ref(username, async move {
                fetched_in_closure.store(true, Ordering::SeqCst);
                match fetch().await {
                    Ok((destinations, outcome)) => {
                        *outcome_writer.lock().expect("outcome slot poisoned") = Some(outcome);
                        Ok::<Arc<HashSet<String>>, EcpdsError>(Arc::new(destinations))
                    }
                    Err(e) => {
                        *outcome_writer.lock().expect("outcome slot poisoned") =
                            Some(e.fetch_outcome());
                        Err(e)
                    }
                }
            })
            .await
            .map_err(|arc_err: Arc<EcpdsError>| match Arc::try_unwrap(arc_err) {
                Ok(err) => err,
                Err(shared) => clone_ecpds_error(&shared),
            });

        let cache_outcome = if fetched.load(Ordering::SeqCst) {
            let outcome = outcome_slot
                .lock()
                .expect("outcome slot poisoned")
                .expect("self-fetch path always writes the outcome before returning");
            CacheOutcome::MissFetched {
                fetch_outcome: outcome,
            }
        } else {
            CacheOutcome::MissCoalesced
        };
        (cache_outcome, result)
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
        let set: HashSet<String> = destinations.into_iter().collect();
        self.cache.insert(username.to_string(), Arc::new(set)).await;
    }

    #[cfg(test)]
    pub(crate) async fn get(&self, username: &str) -> Option<HashSet<String>> {
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

    fn dest_set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

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
        assert_eq!(result, Some(dest_set(&["CIP", "FOO"])));
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
        assert_eq!(cache.get("john").await, Some(dest_set(&["FOO"])));
    }

    #[tokio::test]
    async fn cache_independent_entries_per_user() {
        let cache = DestinationCache::new(300, 1000);
        cache.set("alice", vec!["CIP".to_string()]).await;
        cache.set("bob", vec!["FOO".to_string()]).await;
        assert_eq!(cache.get("alice").await, Some(dest_set(&["CIP"])));
        assert_eq!(cache.get("bob").await, Some(dest_set(&["FOO"])));
    }

    #[tokio::test]
    async fn try_get_or_fetch_hit_does_not_call_fetch() {
        let cache = DestinationCache::new(300, 1000);
        cache.set("john", vec!["CIP".to_string()]).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let (outcome, result) = cache
            .try_get_or_fetch("john", move || {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                async { Ok((dest_set(&["BAD"]), FetchOutcome::Success)) }
            })
            .await;
        let value = result.expect("must succeed");
        assert_eq!(value.as_ref(), &dest_set(&["CIP"]));
        assert_eq!(outcome, CacheOutcome::Hit);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn try_get_or_fetch_miss_calls_fetch_and_caches() {
        let cache = DestinationCache::new(300, 1000);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let (outcome, result) = cache
            .try_get_or_fetch("alice", move || {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                async { Ok((dest_set(&["CIP"]), FetchOutcome::Success)) }
            })
            .await;
        let value = result.expect("must succeed");
        assert_eq!(value.as_ref(), &dest_set(&["CIP"]));
        assert_eq!(
            outcome,
            CacheOutcome::MissFetched {
                fetch_outcome: FetchOutcome::Success
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let calls_clone = calls.clone();
        let (outcome2, result2) = cache
            .try_get_or_fetch("alice", move || {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                async { Ok((dest_set(&["IGNORED"]), FetchOutcome::Success)) }
            })
            .await;
        let value2 = result2.expect("must succeed");
        assert_eq!(value2.as_ref(), &dest_set(&["CIP"]));
        assert_eq!(outcome2, CacheOutcome::Hit);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn try_get_or_fetch_propagates_partial_failure_outcome_on_self_fetch() {
        let cache = DestinationCache::new(300, 1000);
        let (outcome, _result) = cache
            .try_get_or_fetch("bob", || async {
                Ok((dest_set(&["CIP"]), FetchOutcome::Unreachable))
            })
            .await;
        assert_eq!(
            outcome,
            CacheOutcome::MissFetched {
                fetch_outcome: FetchOutcome::Unreachable
            },
            "the fetch outcome from the merge layer must surface as MissFetched.fetch_outcome \
             so the route layer labels aviso_ecpds_fetch_total with the real outcome"
        );
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
                            Ok((dest_set(&["CIP"]), FetchOutcome::Success))
                        }
                    })
                    .await
            }));
        }
        let mut self_fetch_count = 0;
        let mut coalesced_count = 0;
        for handle in handles {
            let (outcome, result) = handle.await.unwrap();
            let value = result.expect("must succeed");
            assert_eq!(value.as_ref(), &dest_set(&["CIP"]));
            match outcome {
                CacheOutcome::Hit => unreachable!("cache started empty"),
                CacheOutcome::MissFetched { .. } => self_fetch_count += 1,
                CacheOutcome::MissCoalesced => coalesced_count += 1,
            }
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "fetch must have run exactly once across concurrent waiters"
        );
        assert_eq!(
            self_fetch_count, 1,
            "exactly one waiter must report MissFetched"
        );
        assert_eq!(
            coalesced_count, 9,
            "the other nine must report MissCoalesced (waited on the in-flight fetch)"
        );
    }

    #[tokio::test]
    async fn concurrent_error_fan_out_only_self_fetcher_reports_missfetched() {
        let cache = Arc::new(DestinationCache::new(300, 1000));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..10 {
            let cache = cache.clone();
            let calls = calls.clone();
            handles.push(tokio::spawn(async move {
                cache
                    .try_get_or_fetch("stampede-fail", move || {
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            Err(EcpdsError::ServiceUnavailable {
                                fetch_outcome: FetchOutcome::Unauthorized,
                            })
                        }
                    })
                    .await
            }));
        }
        let mut self_fetch_count = 0;
        let mut coalesced_count = 0;
        for handle in handles {
            let (outcome, result) = handle.await.unwrap();
            assert!(
                matches!(result, Err(EcpdsError::ServiceUnavailable { .. })),
                "all waiters must observe ServiceUnavailable, never AccessDenied"
            );
            match outcome {
                CacheOutcome::Hit => unreachable!("cache started empty"),
                CacheOutcome::MissFetched { fetch_outcome } => {
                    self_fetch_count += 1;
                    assert_eq!(
                        fetch_outcome,
                        FetchOutcome::Unauthorized,
                        "self-fetcher's MissFetched must carry the underlying \
                         FetchOutcome from the failure (extracted from the error) \
                         so the route layer can label aviso_ecpds_fetch_total \
                         with the real outcome (e.g. http_401), not synthetic Success"
                    );
                }
                CacheOutcome::MissCoalesced => coalesced_count += 1,
            }
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "fetch must run exactly once even when it fails"
        );
        assert_eq!(
            self_fetch_count, 1,
            "exactly one waiter must report MissFetched on the failure path"
        );
        assert_eq!(
            coalesced_count, 9,
            "the other nine must report MissCoalesced (so the route layer does \
             NOT increment aviso_ecpds_fetch_total for them, which would otherwise \
             over-report N upstream calls per stampede when only one happened)"
        );
    }

    #[tokio::test]
    async fn errors_are_not_cached() {
        let cache = DestinationCache::new(300, 1000);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let (_outcome, err_result) = cache
            .try_get_or_fetch("doomed", move || {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                async {
                    Err(EcpdsError::ServiceUnavailable {
                        fetch_outcome: FetchOutcome::Unreachable,
                    })
                }
            })
            .await;
        let err = err_result.expect_err("must error");
        assert!(matches!(err, EcpdsError::ServiceUnavailable { .. }));
        let calls_clone = calls.clone();
        let (_outcome2, ok_result) = cache
            .try_get_or_fetch("doomed", move || {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                async { Ok((dest_set(&["CIP"]), FetchOutcome::Success)) }
            })
            .await;
        let _ok = ok_result.expect("must succeed");
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
                                fetch_outcome: FetchOutcome::Unreachable,
                            })
                        }
                    })
                    .await
            }));
        }
        for handle in handles {
            let (_outcome, result) = handle.await.unwrap();
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
