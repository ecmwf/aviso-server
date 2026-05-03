use crate::{
    cache::{CacheOutcome, DestinationCache},
    client::{EcpdsClient, EcpdsError},
    config::EcpdsConfig,
};
use std::collections::HashMap;

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
    /// 1. Read `match_key` from `identifier` (returns
    ///    [`EcpdsError::AccessDenied`] if missing).
    /// 2. Look up the user's destination list via the single-flight
    ///    cache (one upstream fetch per missing key, even under
    ///    concurrent reconnects).
    /// 3. Linearly scan the list for `destination`.
    ///
    /// Returns the [`CacheOutcome`] alongside the result so the route
    /// layer can record cache hit/miss as a Prometheus label.
    pub async fn check_access(
        &self,
        username: &str,
        identifier: &HashMap<String, String>,
    ) -> Result<CacheOutcome, EcpdsError> {
        let destination = identifier.get(&self.match_key).ok_or_else(|| {
            EcpdsError::AccessDenied(format!(
                "Required field '{}' not found in request identifiers",
                self.match_key
            ))
        })?;

        let client = &self.client;
        let (destinations, outcome) = self
            .cache
            .try_get_or_fetch(username, || client.fetch_user_destinations(username))
            .await?;

        if destinations.contains(destination) {
            Ok(outcome)
        } else {
            Err(EcpdsError::AccessDenied(format!(
                "User '{}' does not have access to destination '{}'",
                username, destination
            )))
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

        let outcome = checker
            .check_access("john", &make_identifier("CIP"))
            .await
            .expect("must succeed");
        assert_eq!(outcome, CacheOutcome::Hit);
    }

    #[tokio::test]
    async fn access_denied_when_destination_not_in_list() {
        let config = make_checker_config();
        let checker = EcpdsChecker::new(&config).expect("checker must build");
        checker.cache.set("john", vec!["CIP".to_string()]).await;

        let result = checker
            .check_access("john", &make_identifier("BAR"))
            .await;
        assert!(matches!(result, Err(EcpdsError::AccessDenied(_))));
    }

    #[tokio::test]
    async fn access_denied_when_match_key_missing_from_identifier() {
        let config = make_checker_config();
        let checker = EcpdsChecker::new(&config).expect("checker must build");
        let empty: HashMap<String, String> = HashMap::new();

        let result = checker.check_access("john", &empty).await;
        assert!(matches!(result, Err(EcpdsError::AccessDenied(_))));
    }

    #[tokio::test]
    async fn service_unavailable_when_cache_miss_and_server_down() {
        let config = make_checker_config();
        let checker = EcpdsChecker::new(&config).expect("checker must build");

        let result = checker
            .check_access("john", &make_identifier("CIP"))
            .await;
        assert!(matches!(result, Err(EcpdsError::ServiceUnavailable)));
    }
}
