use crate::config::{EcpdsConfig, PartialOutagePolicy};
use futures::future::join_all;
use serde::Deserialize;
use std::collections::HashSet;
use thiserror::Error;
use tracing::{debug, info, warn};

/// Coarse-grained reason a single ECPDS fetch (or merged fetch under a
/// partial-outage policy) failed. Surfaces in
/// [`EcpdsError::ServiceUnavailable::fetch_outcome`] and is recorded as
/// the `outcome` label on `aviso_ecpds_fetch_total` so on-call can
/// distinguish "upstream is down" from "credentials are wrong" without
/// reading logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchOutcome {
    /// All servers under the active policy returned a parseable response.
    Success,
    /// At least one server returned HTTP 401: service-account creds are
    /// wrong, expired, or revoked.
    Unauthorized,
    /// At least one server returned HTTP 403: service-account has no
    /// permission to query the destination list.
    Forbidden,
    /// At least one server returned HTTP 5xx: ECPDS itself is broken.
    ServerError,
    /// At least one server returned a body that did not parse against
    /// the expected schema (likely an ECPDS contract drift).
    InvalidResponse,
    /// Network-level failure (DNS, connect timeout, request timeout,
    /// TLS handshake), or no servers configured.
    Unreachable,
}

impl FetchOutcome {
    /// Stable Prometheus label string for this outcome.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Unauthorized => "http_401",
            Self::Forbidden => "http_403",
            Self::ServerError => "http_5xx",
            Self::InvalidResponse => "invalid_response",
            Self::Unreachable => "unreachable",
        }
    }

    fn pessimistic_max(self, other: Self) -> Self {
        fn rank(o: FetchOutcome) -> u8 {
            match o {
                FetchOutcome::Unauthorized => 5,
                FetchOutcome::Forbidden => 4,
                FetchOutcome::InvalidResponse => 3,
                FetchOutcome::ServerError => 2,
                FetchOutcome::Unreachable => 1,
                FetchOutcome::Success => 0,
            }
        }
        if rank(other) > rank(self) {
            other
        } else {
            self
        }
    }
}

/// Why an ECPDS access check denied the user. Maps 1:1 to the
/// `outcome` label on `aviso_ecpds_access_decisions_total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// The user authenticated successfully and the request carried a
    /// valid `match_key` value, but the destination is not in the
    /// user's authorised destination list.
    DestinationNotInList,
    /// The request did not carry a value for the configured
    /// `match_key`. Should be impossible if config validation passed
    /// (the schema enforces `required: true` on the field), but the
    /// runtime keeps the check as a defence-in-depth.
    MatchKeyMissing,
}

impl DenyReason {
    /// Stable Prometheus label string for this reason.
    pub fn label(&self) -> &'static str {
        match self {
            Self::DestinationNotInList => "deny_destination",
            Self::MatchKeyMissing => "deny_match_key_missing",
        }
    }
}

/// Errors produced by the ECPDS client and checker.
///
/// `Display` strings are not part of the public contract; route
/// handlers match on variants and use [`FetchOutcome`] / [`DenyReason`]
/// for typed branching, not message text.
#[derive(Debug, Error)]
pub enum EcpdsError {
    /// The merged fetch under the active partial-outage policy did
    /// not succeed. The `fetch_outcome` carries the dominant cause
    /// (e.g. all servers were unreachable, returned 401, etc.). Maps
    /// to HTTP 503 at the route layer.
    #[error("ECPDS service is unaccessible ({fetch_outcome:?})")]
    ServiceUnavailable {
        /// Dominant cause across all servers, used by the route layer
        /// to label `aviso_ecpds_fetch_total{outcome=...}`.
        fetch_outcome: FetchOutcome,
    },

    /// The user is not allowed to read for the requested destination,
    /// or the destination identifier was missing from the request.
    /// Maps to HTTP 403 at the route layer.
    #[error("Access denied: {message}")]
    AccessDenied {
        /// Typed deny reason consumed by the route layer's metrics
        /// labelling and tracing event field.
        reason: DenyReason,
        /// Human-readable detail; not part of any external contract.
        message: String,
    },

    /// An ECPDS server returned a body that could not be parsed against
    /// our expected schema. Logged with the offending server index.
    #[error("Invalid response from ECPDS server {server_index}: {message}")]
    InvalidResponse {
        /// Zero-based index into the configured `ecpds.servers` list.
        server_index: usize,
        /// Underlying parser error message; not part of any external
        /// contract.
        message: String,
    },

    /// One of the configured server URLs failed to parse at construction
    /// time. Surfaces during `EcpdsClient::new` so misconfigurations fail
    /// at startup rather than per request.
    #[error("Invalid ECPDS server URL '{server}': {source}")]
    InvalidServerUrl {
        /// The original (unparsed) server URL string from config.
        server: String,
        /// Underlying parse error.
        #[source]
        source: url::ParseError,
    },

    /// The underlying `reqwest::Client` could not be built. Should not
    /// happen at runtime under normal conditions; treated as a fatal
    /// configuration error at startup.
    #[error("HTTP client construction failed: {0}")]
    HttpClientBuild(#[source] reqwest::Error),

    /// In-flight HTTP request to an ECPDS server failed. `status` is
    /// `None` for network/timeout errors and `Some(code)` when the
    /// server returned a non-success HTTP status. Captured per-server
    /// so the merge layer can categorise the failure into a
    /// [`FetchOutcome`].
    #[error("HTTP request to ECPDS server {server_index} failed: {message}")]
    Http {
        /// Zero-based index into the configured `ecpds.servers` list.
        server_index: usize,
        /// HTTP status from the server, or `None` for transport-level
        /// failure (DNS, connect, TLS, request timeout).
        status: Option<u16>,
        /// Human-readable detail; not part of any external contract.
        message: String,
    },
}

impl EcpdsError {
    /// Per-error fetch outcome category. Used by the merge layer to
    /// derive the dominant `FetchOutcome` for a multi-server failure
    /// and by the route layer to label
    /// `aviso_ecpds_fetch_total{outcome=...}`.
    pub fn fetch_outcome(&self) -> FetchOutcome {
        match self {
            Self::Http { status, .. } => match status {
                Some(401) => FetchOutcome::Unauthorized,
                Some(403) => FetchOutcome::Forbidden,
                Some(s) if (500..600).contains(s) => FetchOutcome::ServerError,
                Some(_) => FetchOutcome::ServerError,
                None => FetchOutcome::Unreachable,
            },
            Self::InvalidResponse { .. } => FetchOutcome::InvalidResponse,
            Self::ServiceUnavailable { fetch_outcome } => *fetch_outcome,
            Self::AccessDenied { .. }
            | Self::InvalidServerUrl { .. }
            | Self::HttpClientBuild(_) => FetchOutcome::Unreachable,
        }
    }

    /// Typed deny reason if this error is an [`Self::AccessDenied`],
    /// otherwise `None`. Used by the route layer to label
    /// `aviso_ecpds_access_decisions_total`.
    pub fn deny_reason(&self) -> Option<DenyReason> {
        match self {
            Self::AccessDenied { reason, .. } => Some(*reason),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct EcpdsResponse {
    #[serde(rename = "destinationList")]
    destination_list: Vec<serde_json::Value>,
    #[allow(dead_code)]
    success: String,
}

/// HTTP client over one or more ECPDS servers.
///
/// Stateless aside from the prebuilt `reqwest::Client` and the parsed
/// server URLs. Cloning the underlying `reqwest::Client` is cheap (it
/// shares the connection pool internally), but this struct itself is
/// not cloned in practice — one global instance lives behind the
/// `OnceLock` in `aviso-server`'s configuration module.
#[derive(Debug)]
pub struct EcpdsClient {
    http: reqwest::Client,
    servers: Vec<reqwest::Url>,
    username: String,
    password: String,
    target_field: String,
    partial_outage_policy: PartialOutagePolicy,
}

impl EcpdsClient {
    /// Build an ECPDS client from a validated config.
    ///
    /// Fails fast if any configured server URL is malformed or if the
    /// underlying `reqwest::Client` cannot be constructed. The parsed
    /// server URLs are stored once so per-request URL building does not
    /// re-parse them.
    pub fn new(config: &EcpdsConfig) -> Result<Self, EcpdsError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(
                config.request_timeout_seconds,
            ))
            .connect_timeout(std::time::Duration::from_secs(
                config.connect_timeout_seconds,
            ))
            .build()
            .map_err(EcpdsError::HttpClientBuild)?;

        let servers = config
            .servers
            .iter()
            .map(|s| {
                reqwest::Url::parse(s).map_err(|source| EcpdsError::InvalidServerUrl {
                    server: s.clone(),
                    source,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            http,
            servers,
            username: config.username.clone(),
            password: config.password.clone(),
            target_field: config.target_field.clone(),
            partial_outage_policy: config.partial_outage_policy,
        })
    }

    /// Query all configured ECPDS servers in parallel for `username`'s
    /// destination list, then merge per the configured
    /// [`PartialOutagePolicy`].
    ///
    /// Returns a sorted, deduplicated `Vec<String>` on success, or
    /// [`EcpdsError::ServiceUnavailable { .. }`] when the policy's success
    /// criterion is not met.
    pub async fn fetch_user_destinations(&self, username: &str) -> Result<Vec<String>, EcpdsError> {
        if self.servers.is_empty() {
            return Err(EcpdsError::ServiceUnavailable {
                fetch_outcome: FetchOutcome::Unreachable,
            });
        }

        let futures = self
            .servers
            .iter()
            .enumerate()
            .map(|(i, server)| self.fetch_from_server(i, server, username));

        let results: Vec<Result<Vec<String>, EcpdsError>> = join_all(futures).await;

        for (i, result) in results.iter().enumerate() {
            match result {
                Ok(_) => debug!(
                    event_name = "auth.ecpds.fetch.succeeded",
                    server_index = i,
                    server = %self.servers[i],
                    username,
                    "ECPDS server fetch succeeded"
                ),
                Err(e) => warn!(
                    event_name = "auth.ecpds.fetch.failed",
                    server_index = i,
                    server = %self.servers[i],
                    username,
                    error = %e,
                    "ECPDS server fetch failed"
                ),
            }
        }

        match self.partial_outage_policy {
            PartialOutagePolicy::Strict => self.merge_strict(results),
            PartialOutagePolicy::AnySuccess => self.merge_any_success(results),
        }
    }

    /// Strict policy: every configured server must respond successfully.
    /// The resulting destination list is the union of every server's
    /// response. If any one server fails, the whole call fails with
    /// the dominant failure outcome.
    fn merge_strict(
        &self,
        results: Vec<Result<Vec<String>, EcpdsError>>,
    ) -> Result<Vec<String>, EcpdsError> {
        if results.is_empty() {
            return Err(EcpdsError::ServiceUnavailable {
                fetch_outcome: FetchOutcome::Unreachable,
            });
        }
        let mut union: HashSet<String> = HashSet::new();
        for result in results {
            match result {
                Ok(dests) => union.extend(dests),
                Err(e) => {
                    return Err(EcpdsError::ServiceUnavailable {
                        fetch_outcome: e.fetch_outcome(),
                    });
                }
            }
        }
        let mut destinations: Vec<String> = union.into_iter().collect();
        destinations.sort();
        Ok(destinations)
    }

    /// AnySuccess policy: take the union of every server response that
    /// arrived successfully within the per-request timeout. Fails only
    /// when no server returned a usable response. Federated ECPDS
    /// deployments (servers covering different destination namespaces)
    /// should run with this policy.
    fn merge_any_success(
        &self,
        results: Vec<Result<Vec<String>, EcpdsError>>,
    ) -> Result<Vec<String>, EcpdsError> {
        let mut union: HashSet<String> = HashSet::new();
        let mut any_success = false;
        let mut worst_failure: Option<FetchOutcome> = None;
        for result in results {
            match result {
                Ok(dests) => {
                    any_success = true;
                    union.extend(dests);
                }
                Err(e) => {
                    let outcome = e.fetch_outcome();
                    worst_failure = Some(match worst_failure {
                        None => outcome,
                        Some(prev) => prev.pessimistic_max(outcome),
                    });
                }
            }
        }
        if !any_success {
            return Err(EcpdsError::ServiceUnavailable {
                fetch_outcome: worst_failure.unwrap_or(FetchOutcome::Unreachable),
            });
        }
        let mut destinations: Vec<String> = union.into_iter().collect();
        destinations.sort();
        Ok(destinations)
    }

    /// Build a request URL by safely appending the destination-list path to
    /// the pre-parsed `base` server URL and adding the username as a
    /// percent-encoded query parameter.
    ///
    /// Accepts servers with or without a path component (e.g. a reverse-proxy
    /// prefix like `https://proxy.example/ecpds-api/`). Trailing slashes are
    /// normalised so paths join cleanly.
    fn build_request_url(base: &reqwest::Url, username: &str) -> Result<reqwest::Url, String> {
        let mut url = base.clone();
        url.path_segments_mut()
            .map_err(|()| format!("server URL '{base}' cannot be a base"))?
            .pop_if_empty()
            .extend(["ecpds", "v1", "destination", "list"]);
        url.query_pairs_mut().append_pair("id", username);
        Ok(url)
    }

    async fn fetch_from_server(
        &self,
        server_index: usize,
        server: &reqwest::Url,
        username: &str,
    ) -> Result<Vec<String>, EcpdsError> {
        let url =
            Self::build_request_url(server, username).map_err(|message| EcpdsError::Http {
                server_index,
                status: None,
                message,
            })?;
        let response = self
            .http
            .get(url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| EcpdsError::Http {
                server_index,
                status: None,
                message: e.to_string(),
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(EcpdsError::Http {
                server_index,
                status: Some(status.as_u16()),
                message: format!("HTTP {status}"),
            });
        }

        let ecpds_resp: EcpdsResponse =
            response
                .json()
                .await
                .map_err(|e| EcpdsError::InvalidResponse {
                    server_index,
                    message: e.to_string(),
                })?;

        let total = ecpds_resp.destination_list.len();
        let destinations: Vec<String> = ecpds_resp
            .destination_list
            .into_iter()
            .filter_map(|record| {
                record
                    .get(&self.target_field)
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();

        let skipped = total - destinations.len();
        if skipped > 0 {
            info!(
                event_name = "auth.ecpds.fetch.skipped_record",
                target_field = %self.target_field,
                skipped,
                total,
                "ECPDS records missing target_field were skipped"
            );
        }

        Ok(destinations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EcpdsConfig, PartialOutagePolicy};

    fn make_config(servers: Vec<String>) -> EcpdsConfig {
        EcpdsConfig {
            username: "testuser".to_string(),
            password: "testpass".to_string(),
            target_field: "name".to_string(),
            match_key: "destination".to_string(),
            cache_ttl_seconds: 300,
            max_entries: 1000,
            request_timeout_seconds: 30,
            connect_timeout_seconds: 5,
            partial_outage_policy: PartialOutagePolicy::Strict,
            servers,
        }
    }

    #[tokio::test]
    async fn fetch_parses_destination_names() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"destinationList":[{"name":"CIP","active":true},{"name":"FOO","active":true}],"success":"yes"}"#)
            .create_async()
            .await;

        let config = make_config(vec![server.url()]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let result = client.fetch_user_destinations("testuser").await.unwrap();

        mock.assert_async().await;
        assert!(result.contains(&"CIP".to_string()));
        assert!(result.contains(&"FOO".to_string()));
    }

    #[tokio::test]
    async fn any_success_policy_merges_and_deduplicates_multi_server() {
        let mut server_a = mockito::Server::new_async().await;
        let mut server_b = mockito::Server::new_async().await;

        server_a
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"destinationList":[{"name":"CIP"},{"name":"FOO"}],"success":"yes"}"#)
            .create_async()
            .await;

        server_b
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"destinationList":[{"name":"FOO"},{"name":"BAR"}],"success":"yes"}"#)
            .create_async()
            .await;

        let mut config = make_config(vec![server_a.url(), server_b.url()]);
        config.partial_outage_policy = PartialOutagePolicy::AnySuccess;
        let client = EcpdsClient::new(&config).expect("client must build");
        let mut result = client.fetch_user_destinations("testuser").await.unwrap();
        result.sort();

        assert_eq!(result, vec!["BAR", "CIP", "FOO"]);
    }

    #[tokio::test]
    async fn fetch_returns_service_unavailable_when_all_servers_down() {
        let config = make_config(vec!["http://localhost:1".to_string()]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let result = client.fetch_user_destinations("testuser").await;
        assert!(matches!(result, Err(EcpdsError::ServiceUnavailable { .. })));
    }

    #[tokio::test]
    async fn any_success_policy_succeeds_when_one_server_is_down() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"destinationList":[{"name":"CIP"}],"success":"yes"}"#)
            .create_async()
            .await;

        let mut config = make_config(vec!["http://localhost:1".to_string(), server.url()]);
        config.partial_outage_policy = PartialOutagePolicy::AnySuccess;
        let client = EcpdsClient::new(&config).expect("client must build");
        let result = client.fetch_user_destinations("testuser").await.unwrap();
        assert!(result.contains(&"CIP".to_string()));
    }

    #[tokio::test]
    async fn strict_policy_fails_when_one_server_is_down() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"destinationList":[{"name":"CIP"}],"success":"yes"}"#)
            .create_async()
            .await;

        let config = make_config(vec!["http://localhost:1".to_string(), server.url()]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let err = client
            .fetch_user_destinations("testuser")
            .await
            .expect_err("strict policy must fail when any server is unreachable");
        assert!(matches!(err, EcpdsError::ServiceUnavailable { .. }));
    }

    #[tokio::test]
    async fn strict_policy_unions_disjoint_responses_from_all_servers() {
        let mut server_a = mockito::Server::new_async().await;
        let mut server_b = mockito::Server::new_async().await;
        server_a
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"destinationList":[{"name":"CIP"}],"success":"yes"}"#)
            .create_async()
            .await;
        server_b
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"destinationList":[{"name":"BAR"}],"success":"yes"}"#)
            .create_async()
            .await;

        let config = make_config(vec![server_a.url(), server_b.url()]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let mut result = client.fetch_user_destinations("testuser").await.unwrap();
        result.sort();
        assert_eq!(result, vec!["BAR".to_string(), "CIP".to_string()]);
    }

    #[tokio::test]
    async fn strict_policy_unions_overlapping_responses_from_all_servers() {
        let mut server_a = mockito::Server::new_async().await;
        let mut server_b = mockito::Server::new_async().await;
        for srv in [&mut server_a, &mut server_b] {
            srv.mock("GET", "/ecpds/v1/destination/list")
                .match_query(mockito::Matcher::Any)
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(r#"{"destinationList":[{"name":"CIP"},{"name":"FOO"}],"success":"yes"}"#)
                .create_async()
                .await;
        }

        let config = make_config(vec![server_a.url(), server_b.url()]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let mut result = client.fetch_user_destinations("testuser").await.unwrap();
        result.sort();
        assert_eq!(result, vec!["CIP".to_string(), "FOO".to_string()]);
    }

    #[tokio::test]
    async fn fetch_classifies_http_401_as_unauthorized() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(401)
            .with_body(r#"{"error":"unauthorized"}"#)
            .create_async()
            .await;

        let config = make_config(vec![server.url()]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let err = client
            .fetch_user_destinations("testuser")
            .await
            .expect_err("must fail");
        let outcome = err.fetch_outcome();
        assert_eq!(outcome, FetchOutcome::Unauthorized, "got {outcome:?}");
        assert_eq!(outcome.label(), "http_401");
    }

    #[tokio::test]
    async fn fetch_classifies_http_500_as_server_error() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(500)
            .with_body(r#"{"error":"oops"}"#)
            .create_async()
            .await;

        let config = make_config(vec![server.url()]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let err = client
            .fetch_user_destinations("testuser")
            .await
            .expect_err("must fail");
        assert_eq!(err.fetch_outcome(), FetchOutcome::ServerError);
        assert_eq!(err.fetch_outcome().label(), "http_5xx");
    }

    #[tokio::test]
    async fn fetch_classifies_malformed_json_as_invalid_response() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("not even close to valid json")
            .create_async()
            .await;

        let config = make_config(vec![server.url()]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let err = client
            .fetch_user_destinations("testuser")
            .await
            .expect_err("must fail");
        assert_eq!(err.fetch_outcome(), FetchOutcome::InvalidResponse);
        assert_eq!(err.fetch_outcome().label(), "invalid_response");
    }

    #[tokio::test]
    async fn fetch_classifies_unreachable_as_unreachable() {
        let config = make_config(vec!["http://127.0.0.1:1".to_string()]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let err = client
            .fetch_user_destinations("testuser")
            .await
            .expect_err("must fail");
        assert_eq!(err.fetch_outcome(), FetchOutcome::Unreachable);
    }

    #[tokio::test]
    async fn parsing_tolerates_records_missing_target_field() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"destinationList":[{"name":"CIP"},{"id":"no-name"},{"name":"FOO"}],"success":"yes"}"#,
            )
            .create_async()
            .await;

        let config = make_config(vec![server.url()]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let mut result = client.fetch_user_destinations("testuser").await.unwrap();
        result.sort();
        assert_eq!(result, vec!["CIP".to_string(), "FOO".to_string()]);
    }

    #[tokio::test]
    async fn fetch_uses_custom_target_field() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"destinationList":[{"id":"DEST1","name":"CIP"}],"success":"yes"}"#)
            .create_async()
            .await;

        let mut config = make_config(vec![server.url()]);
        config.target_field = "id".to_string();
        let client = EcpdsClient::new(&config).expect("client must build");
        let result = client.fetch_user_destinations("testuser").await.unwrap();
        assert!(result.contains(&"DEST1".to_string()));
        assert!(!result.contains(&"CIP".to_string()));
    }

    #[test]
    fn build_request_url_percent_encodes_special_chars() {
        let base = reqwest::Url::parse("http://example.com").unwrap();
        let url = EcpdsClient::build_request_url(&base, "user+name with spaces&extra=injected")
            .expect("URL must build");
        let s = url.as_str();
        assert!(s.starts_with("http://example.com/ecpds/v1/destination/list?id="));
        assert!(s.contains("user%2Bname"), "got {s}");
        assert!(
            s.contains("with+spaces") || s.contains("with%20spaces"),
            "got {s}"
        );
        assert!(s.contains("%26extra%3Dinjected"), "got {s}");
        assert!(!s.contains("&extra=injected"), "got {s}");
    }

    #[test]
    fn build_request_url_handles_reverse_proxy_prefix_with_trailing_slash() {
        let base = reqwest::Url::parse("https://proxy.example/ecpds-api/").unwrap();
        let url = EcpdsClient::build_request_url(&base, "alice").unwrap();
        assert_eq!(
            url.as_str(),
            "https://proxy.example/ecpds-api/ecpds/v1/destination/list?id=alice"
        );
    }

    #[test]
    fn build_request_url_handles_reverse_proxy_prefix_without_trailing_slash() {
        let base = reqwest::Url::parse("https://proxy.example/ecpds-api").unwrap();
        let url = EcpdsClient::build_request_url(&base, "alice").unwrap();
        assert_eq!(
            url.as_str(),
            "https://proxy.example/ecpds-api/ecpds/v1/destination/list?id=alice"
        );
    }

    #[test]
    fn client_construction_rejects_invalid_server_url() {
        let config = make_config(vec!["not a url".to_string()]);
        let err = EcpdsClient::new(&config)
            .expect_err("invalid server URL must be rejected at construction");
        assert!(matches!(err, EcpdsError::InvalidServerUrl { .. }));
    }

    #[tokio::test]
    async fn fetch_url_encodes_username_with_special_chars() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::UrlEncoded(
                "id".into(),
                "u+s er&name".into(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"destinationList":[{"name":"OK"}],"success":"yes"}"#)
            .create_async()
            .await;

        let config = make_config(vec![server.url()]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let result = client
            .fetch_user_destinations("u+s er&name")
            .await
            .expect("should succeed");

        mock.assert_async().await;
        assert!(result.contains(&"OK".to_string()));
    }

    #[tokio::test]
    async fn fetch_works_with_reverse_proxy_prefix_server() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/some-prefix/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::UrlEncoded("id".into(), "alice".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"destinationList":[{"name":"OK"}],"success":"yes"}"#)
            .create_async()
            .await;

        let config = make_config(vec![format!("{}/some-prefix/", server.url())]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let result = client
            .fetch_user_destinations("alice")
            .await
            .expect("should succeed");

        mock.assert_async().await;
        assert!(result.contains(&"OK".to_string()));
    }
}
