// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Cached JSON Web Key Set for verifying MCP access tokens.
//!
//! Keys are fetched from one configured URI, kept for a fixed TTL, and
//! refetched early only when a token names an unknown `kid`. Every fetch,
//! scheduled or not, is spaced by a minimum interval so a flood of forged
//! `kid` values cannot turn the server into a load generator against the
//! issuer.

use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use jsonwebtoken::DecodingKey;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet, KeyAlgorithm, PublicKeyUse};
use reqwest::{Client, StatusCode, redirect};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Largest key set document accepted from the issuer.
const MAX_JWKS_BYTES: usize = 256 * 1024;
/// Most signing keys kept from one key set.
const MAX_KEYS: usize = 32;

/// Why no key could be produced for a token.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum JwksError {
    /// The key set is current and has no key with this id.
    #[error("no signing key matches the token key id")]
    UnknownKey,
    /// The key set could not be fetched and no copy is cached.
    #[error("signing keys are unavailable: {0}")]
    Unavailable(String),
    /// The HTTP client could not be built.
    #[error("failed to build JWKS client: {0}")]
    ClientBuild(#[source] reqwest::Error),
}

/// Timing knobs for [`JwksCache`].
#[derive(Debug, Clone, Copy)]
pub struct JwksTiming {
    /// How long a fetched set is used before a scheduled refetch.
    pub ttl: Duration,
    /// Minimum spacing between two fetches.
    pub min_refetch_interval: Duration,
    /// Deadline for one fetch.
    pub fetch_timeout: Duration,
}

impl JwksTiming {
    /// Production timing: the given TTL, refetch at most once per minute,
    /// and a five second fetch deadline.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            min_refetch_interval: Duration::from_secs(60),
            fetch_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Default)]
struct CacheState {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Option<Instant>,
    last_attempt: Option<Instant>,
}

/// Shared key cache. One fetch runs at a time; concurrent callers wait for
/// it and then read its result.
pub struct JwksCache {
    client: Client,
    uri: String,
    timing: JwksTiming,
    state: Mutex<CacheState>,
}

impl JwksCache {
    /// Build a cache for `uri`. Nothing is fetched until the first token.
    pub fn new(uri: &str, timing: JwksTiming) -> Result<Self, JwksError> {
        let client = Client::builder()
            .timeout(timing.fetch_timeout)
            .connect_timeout(timing.fetch_timeout)
            .redirect(redirect::Policy::none())
            .build()
            .map_err(JwksError::ClientBuild)?;
        Ok(Self {
            client,
            uri: uri.to_string(),
            timing,
            state: Mutex::new(CacheState::default()),
        })
    }

    /// Return the key for `kid`, fetching the set when it is stale or
    /// missing the id and the refetch interval allows it.
    pub async fn key(&self, kid: &str) -> Result<DecodingKey, JwksError> {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        let stale = state
            .fetched_at
            .is_none_or(|at| now.duration_since(at) >= self.timing.ttl);
        let known = state.keys.contains_key(kid);
        let may_fetch = state
            .last_attempt
            .is_none_or(|at| now.duration_since(at) >= self.timing.min_refetch_interval);

        if (stale || !known) && may_fetch {
            state.last_attempt = Some(now);
            match self.fetch().await {
                Ok(keys) => {
                    info!(
                        service_name = SERVICE_NAME,
                        service_version = SERVICE_VERSION,
                        event_name = "mcp.jwks.fetch.succeeded",
                        key_count = keys.len(),
                        "Fetched MCP token signing keys"
                    );
                    state.keys = keys;
                    state.fetched_at = Some(now);
                }
                Err(error) => {
                    warn!(
                        service_name = SERVICE_NAME,
                        service_version = SERVICE_VERSION,
                        event_name = "mcp.jwks.fetch.failed",
                        error = %error,
                        cached_keys = state.keys.len(),
                        "Fetching MCP token signing keys failed"
                    );
                    if state.keys.is_empty() {
                        return Err(JwksError::Unavailable(error));
                    }
                }
            }
        }

        if let Some(key) = state.keys.get(kid) {
            return Ok(key.clone());
        }
        if state.fetched_at.is_none() {
            return Err(JwksError::Unavailable("no key set fetched yet".to_string()));
        }
        Err(JwksError::UnknownKey)
    }

    async fn fetch(&self) -> Result<HashMap<String, DecodingKey>, String> {
        let mut response = self
            .client
            .get(&self.uri)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        if response.status() != StatusCode::OK {
            return Err(format!("unexpected status {}", response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|len| len > MAX_JWKS_BYTES as u64)
        {
            return Err("key set exceeds size limit".to_string());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("reading body failed: {error}"))?
        {
            if body.len() + chunk.len() > MAX_JWKS_BYTES {
                return Err("key set exceeds size limit".to_string());
            }
            body.extend_from_slice(&chunk);
        }
        let set: JwkSet =
            serde_json::from_slice(&body).map_err(|error| format!("invalid key set: {error}"))?;
        let keys = usable_keys(&set);
        if keys.is_empty() {
            return Err("key set has no usable RS256 signing key".to_string());
        }
        Ok(keys)
    }
}

/// Keep RSA signing keys that have a `kid` and do not declare another
/// algorithm or use.
fn usable_keys(set: &JwkSet) -> HashMap<String, DecodingKey> {
    set.keys
        .iter()
        .filter(|jwk| matches!(jwk.algorithm, AlgorithmParameters::RSA(_)))
        .filter(|jwk| {
            jwk.common
                .key_algorithm
                .is_none_or(|alg| alg == KeyAlgorithm::RS256)
        })
        .filter(|jwk| {
            jwk.common
                .public_key_use
                .as_ref()
                .is_none_or(|key_use| *key_use == PublicKeyUse::Signature)
        })
        .filter_map(|jwk| {
            let kid = jwk.common.key_id.clone()?;
            let key = DecodingKey::from_jwk(jwk).ok()?;
            Some((kid, key))
        })
        .take(MAX_KEYS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::usable_keys;
    use jsonwebtoken::jwk::JwkSet;
    use serde_json::json;

    #[test]
    fn only_rsa_signing_keys_with_ids_are_kept() {
        let rsa = json!({"kty": "RSA", "n": "sXch", "e": "AQAB"});
        let with = |extra: serde_json::Value| {
            let mut key = rsa.clone();
            key.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            key
        };
        let set: JwkSet = serde_json::from_value(json!({"keys": [
            with(json!({"kid": "good", "alg": "RS256", "use": "sig"})),
            with(json!({"kid": "plain"})),
            with(json!({"kid": "enc", "use": "enc"})),
            with(json!({"kid": "ps", "alg": "PS256"})),
            rsa,
            {"kty": "oct", "k": "c2VjcmV0", "kid": "hmac"}
        ]}))
        .unwrap();
        let mut kept: Vec<_> = usable_keys(&set).into_keys().collect();
        kept.sort();
        assert_eq!(kept, vec!["good", "plain"]);
    }
}
