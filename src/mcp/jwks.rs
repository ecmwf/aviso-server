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
//! issuer. Verification of tokens with known keys never waits for a
//! fetch: a stale set keeps serving while one caller refreshes it.

use crate::telemetry::{SERVICE_NAME, SERVICE_VERSION};
use jsonwebtoken::DecodingKey;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet, KeyAlgorithm, PublicKeyUse};
use reqwest::{Client, StatusCode, redirect};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

/// Largest key set document accepted from the issuer.
const MAX_JWKS_BYTES: usize = 256 * 1024;
/// Most signing keys kept from one key set.
const MAX_KEYS: usize = 32;
/// Factor of the TTL after which a cached set is no longer trusted.
const MAX_STALE_TTL_FACTOR: u32 = 12;
/// Lower bound of the maximum stale age.
const MIN_MAX_STALE: Duration = Duration::from_secs(3600);

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
    /// Age after which a cached set is refused when it cannot be
    /// refreshed, so an issuer outage cannot keep revoked keys alive.
    pub max_stale: Duration,
}

impl JwksTiming {
    /// Production timing: the given TTL, refetch at most once per minute,
    /// a five second fetch deadline, and a maximum stale age of twelve
    /// TTLs but at least one hour.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            min_refetch_interval: Duration::from_secs(60),
            fetch_timeout: Duration::from_secs(5),
            max_stale: ttl.saturating_mul(MAX_STALE_TTL_FACTOR).max(MIN_MAX_STALE),
        }
    }
}

/// One fetched key set. Replaced as a whole, never mutated.
#[derive(Default)]
struct Snapshot {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Option<Instant>,
}

impl Snapshot {
    fn age(&self, now: Instant) -> Option<Duration> {
        self.fetched_at.map(|at| now.duration_since(at))
    }

    fn lookup(&self, kid: &str, max_stale: Duration) -> Result<DecodingKey, JwksError> {
        let Some(age) = self.age(Instant::now()) else {
            return Err(JwksError::Unavailable("no key set fetched yet".to_string()));
        };
        if age >= max_stale {
            return Err(JwksError::Unavailable(
                "cached key set is past its maximum stale age".to_string(),
            ));
        }
        self.keys.get(kid).cloned().ok_or(JwksError::UnknownKey)
    }

    /// Whether `kid` is cached and the set is still within `max_stale`.
    fn usable(&self, kid: &str, now: Instant, max_stale: Duration) -> bool {
        self.keys.contains_key(kid) && self.age(now).is_some_and(|age| age < max_stale)
    }
}

/// Bookkeeping guarded by the single-flight fetch lock.
#[derive(Default)]
struct FetchState {
    last_attempt: Option<Instant>,
}

/// Shared key cache.
///
/// Readers clone the current [`Snapshot`] under a short read lock and
/// never wait for the network. At most one fetch runs at a time, guarded
/// by a separate lock:
/// - a known `kid` in a fresh set is served at once;
/// - a known `kid` in a stale set is served from the cached copy, while
///   the first caller to find the fetch lock free refreshes the set;
/// - an unknown `kid`, no set at all, or a set past its maximum stale age
///   waits for the fetch lock, then uses the set the previous holder
///   fetched or fetches one itself.
pub struct JwksCache {
    client: Client,
    uri: String,
    timing: JwksTiming,
    snapshot: RwLock<Arc<Snapshot>>,
    fetch_state: Mutex<FetchState>,
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
            snapshot: RwLock::new(Arc::new(Snapshot::default())),
            fetch_state: Mutex::new(FetchState::default()),
        })
    }

    /// Return the key for `kid`, fetching the set when it is stale or
    /// missing the id and the refetch interval allows it.
    pub async fn key(&self, kid: &str) -> Result<DecodingKey, JwksError> {
        let max_stale = self.timing.max_stale;
        let snapshot = self.snapshot().await;
        let now = Instant::now();
        if snapshot.usable(kid, now, max_stale) {
            let fresh = snapshot.age(now).is_some_and(|age| age < self.timing.ttl);
            if !fresh && let Ok(mut fetch_state) = self.fetch_state.try_lock() {
                self.refresh(&mut fetch_state).await;
                return self.snapshot().await.lookup(kid, max_stale);
            }
            return snapshot.lookup(kid, max_stale);
        }

        let mut fetch_state = self.fetch_state.lock().await;
        let snapshot = self.snapshot().await;
        if !snapshot.usable(kid, Instant::now(), max_stale) {
            self.refresh(&mut fetch_state).await;
            return self.snapshot().await.lookup(kid, max_stale);
        }
        snapshot.lookup(kid, max_stale)
    }

    async fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&*self.snapshot.read().await)
    }

    /// Fetch and publish a new set unless the last attempt is too recent.
    /// A failed fetch keeps the cached set.
    async fn refresh(&self, fetch_state: &mut FetchState) {
        let now = Instant::now();
        let may_fetch = fetch_state
            .last_attempt
            .is_none_or(|at| now.duration_since(at) >= self.timing.min_refetch_interval);
        if !may_fetch {
            return;
        }
        fetch_state.last_attempt = Some(now);
        match self.fetch().await {
            Ok(keys) => {
                info!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_name = "mcp.jwks.fetch.succeeded",
                    key_count = keys.len(),
                    "Fetched MCP token signing keys"
                );
                *self.snapshot.write().await = Arc::new(Snapshot {
                    keys,
                    fetched_at: Some(now),
                });
            }
            Err(error) => {
                let cached_keys = self.snapshot().await.keys.len();
                warn!(
                    service_name = SERVICE_NAME,
                    service_version = SERVICE_VERSION,
                    event_name = "mcp.jwks.fetch.failed",
                    error = %error,
                    cached_keys,
                    "Fetching MCP token signing keys failed"
                );
            }
        }
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
