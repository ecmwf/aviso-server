use crate::{
    cache::DestinationCache,
    client::{EcpdsClient, EcpdsError},
    config::EcpdsConfig,
};
use std::collections::HashMap;

pub struct EcpdsChecker {
    client: EcpdsClient,
    cache: DestinationCache,
    match_key: String,
}

impl EcpdsChecker {
    pub fn new(config: &EcpdsConfig) -> Self {
        Self {
            client: EcpdsClient::new(config),
            cache: DestinationCache::new(config.cache_ttl_seconds),
            match_key: config.match_key.clone(),
        }
    }

    /// Check if `username` has access to the destination value extracted from `identifier`.
    /// - Extracts the value of `match_key` from the identifier map
    /// - Checks cache first; on miss, fetches from ECPDS servers
    /// - Returns Ok(()) if the destination is in the user's list, Err(AccessDenied) if not
    pub async fn check_access(
        &self,
        username: &str,
        identifier: &HashMap<String, String>,
    ) -> Result<(), EcpdsError> {
        let destination = identifier.get(&self.match_key).ok_or_else(|| {
            EcpdsError::AccessDenied(format!(
                "Required field '{}' not found in request identifiers",
                self.match_key
            ))
        })?;

        let destinations = if let Some(cached) = self.cache.get(username).await {
            cached
        } else {
            let fetched = self.client.fetch_user_destinations(username).await?;
            self.cache.set(username, fetched.clone()).await;
            fetched
        };

        if destinations.contains(destination) {
            Ok(())
        } else {
            Err(EcpdsError::AccessDenied(format!(
                "User '{}' does not have access to destination '{}'",
                username, destination
            )))
        }
    }
}
