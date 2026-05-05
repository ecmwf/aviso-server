use crate::{
    cache::{CacheOutcome, DestinationCache},
    client::{DenyReason, EcpdsClient, EcpdsError},
    config::EcpdsConfig,
};
use std::collections::HashMap;
use tracing::debug;

/// Outcome of a single [`EcpdsChecker::check_access`] call.
///
/// Pairs the typed authorisation result with the cache outcome so the
/// route layer can record cache and fetch metrics independently of
/// allow / deny / unavailable. `cache_outcome` is `None` when the
/// request never reached the cache (e.g. `match_key` was missing from
/// the request identifier and the check failed before any lookup).
#[derive(Debug)]
pub struct AccessCheckResult {
    /// Cache outcome when the cache was consulted, otherwise `None`.
    pub cache_outcome: Option<CacheOutcome>,
    /// Authorisation result. `Ok(())` means the destination is in the
    /// user's allow-list; `Err` distinguishes deny / unavailable /
    /// other error per [`EcpdsError`].
    pub result: Result<(), EcpdsError>,
}

/// Public facade combining the ECPDS HTTP client, the per-username
/// destination cache, and the destination match-key logic.
///
/// One instance is built per running Aviso process by `Application::build`
/// and shared across request handlers via actix `app_data`. Construction
/// validates the configuration; per-call behaviour is concentrated in
/// [`Self::check_access`].
pub struct EcpdsChecker {
    client: EcpdsClient,
    pub(crate) cache: DestinationCache,
    match_key: String,
}

impl EcpdsChecker {
    /// Build a checker from a validated config.
    ///
    /// Propagates [`EcpdsError`] from the underlying [`EcpdsClient`]
    /// constructor so misconfigurations (invalid server URLs, broken
    /// HTTP client) fail at startup rather than per request.
    pub fn new(config: &EcpdsConfig) -> Result<Self, EcpdsError> {
        Ok(Self {
            client: EcpdsClient::new(config)?,
            cache: DestinationCache::new(config.cache_ttl_seconds, config.max_entries),
            match_key: config.match_key.clone(),
        })
    }

    /// Check whether `username` is authorised to read for the
    /// destination value extracted from `identifier`.
    ///
    /// 1. Read `match_key` from `identifier`. If missing, returns
    ///    [`AccessCheckResult`] with `cache_outcome: None` and
    ///    `result: Err(AccessDenied { MatchKeyMissing })`.
    /// 2. Look up the user's destination list via the single-flight
    ///    cache (one upstream fetch per missing key, even under
    ///    concurrent reconnects).
    /// 3. Linearly scan the list for `destination`.
    ///
    /// Returns [`AccessCheckResult`] so the route layer can record
    /// cache hit/miss and fetch outcome on every code path (allow,
    /// deny, unavailable), not just the success path.
    pub async fn check_access(
        &self,
        username: &str,
        identifier: &HashMap<String, String>,
    ) -> AccessCheckResult {
        let Some(destination) = identifier.get(&self.match_key) else {
            return AccessCheckResult {
                cache_outcome: None,
                result: Err(EcpdsError::AccessDenied {
                    reason: DenyReason::MatchKeyMissing,
                    message: format!(
                        "Required field '{}' not found in request identifiers",
                        self.match_key
                    ),
                }),
            };
        };

        let client = &self.client;
        let (cache_outcome, fetch_result) = self
            .cache
            .try_get_or_fetch(username, || client.fetch_user_destinations(username))
            .await;

        match cache_outcome {
            CacheOutcome::Hit => debug!(
                event_name = "auth.ecpds.cache.hit",
                username, "ECPDS destination cache hit"
            ),
            CacheOutcome::MissCoalesced | CacheOutcome::MissFetched { .. } => debug!(
                event_name = "auth.ecpds.cache.miss",
                username, "ECPDS destination cache miss"
            ),
        }

        let result = match fetch_result {
            Ok(destinations) => {
                if destinations.contains(destination) {
                    Ok(())
                } else {
                    Err(EcpdsError::AccessDenied {
                        reason: DenyReason::DestinationNotInList,
                        message: format!(
                            "User '{}' does not have access to destination '{}'",
                            username, destination
                        ),
                    })
                }
            }
            Err(e) => Err(e),
        };

        AccessCheckResult {
            cache_outcome: Some(cache_outcome),
            result,
        }
    }

    /// Number of distinct usernames currently held in the cache.
    /// Sampled by the route layer's `cache_size` Prometheus gauge.
    pub fn cache_entry_count(&self) -> u64 {
        self.cache.entry_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EcpdsConfig;
    use std::collections::HashMap;

    fn make_checker_config() -> EcpdsConfig {
        EcpdsConfig {
            username: "masteruser".to_string(),
            password: "pass".to_string(),
            target_field: "name".to_string(),
            match_key: "destination".to_string(),
            cache_ttl_seconds: 300,
            max_entries: 1000,
            request_timeout_seconds: 30,
            connect_timeout_seconds: 5,
            partial_outage_policy: crate::config::PartialOutagePolicy::Strict,
            servers: vec!["http://localhost:1".to_string()],
        }
    }

    fn make_identifier(destination: &str) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("destination".to_string(), destination.to_string());
        m
    }

    #[tokio::test]
    async fn access_granted_when_destination_in_cached_list() {
        let config = make_checker_config();
        let checker = EcpdsChecker::new(&config).expect("checker must build");
        checker
            .cache
            .set("john", vec!["CIP".to_string(), "FOO".to_string()])
            .await;

        let access = checker.check_access("john", &make_identifier("CIP")).await;
        access.result.expect("must succeed");
        assert_eq!(access.cache_outcome, Some(CacheOutcome::Hit));
    }

    #[tokio::test]
    async fn access_denied_after_cache_hit_still_reports_hit_outcome() {
        let config = make_checker_config();
        let checker = EcpdsChecker::new(&config).expect("checker must build");
        checker.cache.set("john", vec!["CIP".to_string()]).await;

        let access = checker.check_access("john", &make_identifier("BAR")).await;
        assert!(matches!(
            access.result,
            Err(EcpdsError::AccessDenied { .. })
        ));
        assert_eq!(
            access.cache_outcome,
            Some(CacheOutcome::Hit),
            "deny must still propagate cache_outcome so the route layer \
             increments aviso_ecpds_cache_hits_total even on the deny path"
        );
    }

    #[tokio::test]
    async fn access_denied_when_match_key_missing_yields_no_cache_outcome() {
        let config = make_checker_config();
        let checker = EcpdsChecker::new(&config).expect("checker must build");
        let empty: HashMap<String, String> = HashMap::new();

        let access = checker.check_access("john", &empty).await;
        assert!(matches!(
            access.result,
            Err(EcpdsError::AccessDenied { .. })
        ));
        assert!(
            access.cache_outcome.is_none(),
            "MatchKeyMissing fails before any cache lookup so cache_outcome \
             must be None to avoid bogus cache_misses_total increments"
        );
    }

    #[tokio::test]
    async fn service_unavailable_when_cache_miss_and_server_down_still_reports_outcome() {
        let config = make_checker_config();
        let checker = EcpdsChecker::new(&config).expect("checker must build");

        let access = checker.check_access("john", &make_identifier("CIP")).await;
        assert!(matches!(
            access.result,
            Err(EcpdsError::ServiceUnavailable { .. })
        ));
        assert!(
            matches!(access.cache_outcome, Some(CacheOutcome::MissFetched { .. })),
            "ServiceUnavailable from a self-fetched failure must still \
             propagate MissFetched so fetch_total is labelled with the \
             real failure outcome (e.g. unreachable) on this path"
        );
    }
}
