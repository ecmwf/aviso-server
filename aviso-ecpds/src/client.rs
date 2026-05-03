use crate::config::{EcpdsConfig, PartialOutagePolicy};
use futures::future::join_all;
use serde::Deserialize;
use std::collections::HashSet;
use thiserror::Error;
use tracing::{debug, info, warn};

/// Errors produced by the ECPDS client and checker.
///
/// `Display` strings are not part of the public contract; route handlers
/// match on variants, not message text.
#[derive(Debug, Error)]
pub enum EcpdsError {
    /// All configured ECPDS servers failed to respond, or no server-level
    /// success was achieved under the active partial-outage policy.
    /// Maps to HTTP 503 at the route layer.
    #[error("ECPDS service is unaccessible")]
    ServiceUnavailable,

    /// The user is not allowed to read for the requested destination, or
    /// the destination identifier was missing from the request. Maps to
    /// HTTP 403 at the route layer.
    #[error("Access denied: {0}")]
    AccessDenied(String),

    /// An ECPDS server returned a body that could not be parsed against
    /// our expected schema. Logged with the offending server index.
    #[error("Invalid response from ECPDS server {server_index}: {source}")]
    InvalidResponse {
        server_index: usize,
        #[source]
        source: serde_json::Error,
    },

    /// One of the configured server URLs failed to parse at construction
    /// time. Surfaces during `EcpdsClient::new` so misconfigurations fail
    /// at startup rather than per request.
    #[error("Invalid ECPDS server URL '{server}': {source}")]
    InvalidServerUrl {
        server: String,
        #[source]
        source: url::ParseError,
    },

    /// The underlying `reqwest::Client` could not be built. Should not
    /// happen at runtime under normal conditions; treated as a fatal
    /// configuration error at startup.
    #[error("HTTP client construction failed: {0}")]
    HttpClientBuild(#[source] reqwest::Error),
}

#[derive(Deserialize)]
struct EcpdsResponse {
    #[serde(rename = "destinationList")]
    destination_list: Vec<serde_json::Value>,
    #[allow(dead_code)]
    success: String,
}

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
            .timeout(std::time::Duration::from_secs(config.request_timeout_seconds))
            .connect_timeout(std::time::Duration::from_secs(config.connect_timeout_seconds))
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
    /// [`EcpdsError::ServiceUnavailable`] when the policy's success
    /// criterion is not met.
    pub async fn fetch_user_destinations(&self, username: &str) -> Result<Vec<String>, EcpdsError> {
        let futures = self
            .servers
            .iter()
            .map(|server| self.fetch_from_server(server, username));

        let results = join_all(futures).await;

        for (i, result) in results.iter().enumerate() {
            match result {
                Ok(_) => debug!(
                    server_index = i,
                    server = %self.servers[i],
                    username,
                    "auth.ecpds.fetch.succeeded"
                ),
                Err(e) => warn!(
                    server_index = i,
                    server = %self.servers[i],
                    username,
                    error = %e,
                    "auth.ecpds.fetch.failed"
                ),
            }
        }

        match self.partial_outage_policy {
            PartialOutagePolicy::Strict => self.merge_strict(results),
            PartialOutagePolicy::AnySuccess => self.merge_any_success(results),
        }
    }

    fn merge_strict(
        &self,
        results: Vec<Result<Vec<String>, String>>,
    ) -> Result<Vec<String>, EcpdsError> {
        let mut canonical: Option<HashSet<String>> = None;
        for (i, result) in results.into_iter().enumerate() {
            let dests = result.map_err(|_| EcpdsError::ServiceUnavailable)?;
            let set: HashSet<String> = dests.into_iter().collect();
            match canonical.as_ref() {
                None => canonical = Some(set),
                Some(prev) if prev == &set => {}
                Some(prev) => {
                    let divergence_count = prev.symmetric_difference(&set).count();
                    warn!(
                        server_index = i,
                        divergence_count,
                        "auth.ecpds.fetch.divergence"
                    );
                    return Err(EcpdsError::ServiceUnavailable);
                }
            }
        }
        let mut destinations: Vec<String> = canonical.unwrap_or_default().into_iter().collect();
        destinations.sort();
        Ok(destinations)
    }

    fn merge_any_success(
        &self,
        results: Vec<Result<Vec<String>, String>>,
    ) -> Result<Vec<String>, EcpdsError> {
        let mut server_sets: Vec<HashSet<String>> = Vec::new();
        for result in results {
            if let Ok(dests) = result {
                server_sets.push(dests.into_iter().collect());
            }
        }
        if server_sets.is_empty() {
            return Err(EcpdsError::ServiceUnavailable);
        }

        let union: HashSet<String> = server_sets.iter().flat_map(|s| s.iter().cloned()).collect();
        let all_agree = server_sets.iter().all(|s| s == &union);
        if !all_agree && server_sets.len() > 1 {
            let max_div: usize = server_sets
                .iter()
                .map(|s| union.symmetric_difference(s).count())
                .max()
                .unwrap_or(0);
            warn!(
                divergence_count = max_div,
                reachable_servers = server_sets.len(),
                "auth.ecpds.fetch.divergence"
            );
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
        server: &reqwest::Url,
        username: &str,
    ) -> Result<Vec<String>, String> {
        let url = Self::build_request_url(server, username)?;
        let response = self
            .http
            .get(url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }

        let ecpds_resp: EcpdsResponse = response
            .json()
            .await
            .map_err(|e| format!("JSON parse error: {e}"))?;

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
                target_field = %self.target_field,
                skipped,
                total,
                "auth.ecpds.fetch.skipped_record"
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
        assert!(matches!(result, Err(EcpdsError::ServiceUnavailable)));
    }

    #[tokio::test]
    async fn any_success_policy_succeeds_when_one_server_is_down() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", "/ecpds/v1/destination/list")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"destinationList":[{"name":"CIP"}],"success":"yes"}"#,
            )
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
        assert!(matches!(err, EcpdsError::ServiceUnavailable));
    }

    #[tokio::test]
    async fn strict_policy_fails_when_servers_disagree() {
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
        let err = client
            .fetch_user_destinations("testuser")
            .await
            .expect_err("strict policy must fail when servers disagree");
        assert!(matches!(err, EcpdsError::ServiceUnavailable));
    }

    #[tokio::test]
    async fn strict_policy_succeeds_when_servers_agree() {
        let mut server_a = mockito::Server::new_async().await;
        let mut server_b = mockito::Server::new_async().await;
        for srv in [&mut server_a, &mut server_b] {
            srv.mock("GET", "/ecpds/v1/destination/list")
                .match_query(mockito::Matcher::Any)
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(
                    r#"{"destinationList":[{"name":"CIP"},{"name":"FOO"}],"success":"yes"}"#,
                )
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
        assert!(s.contains("with+spaces") || s.contains("with%20spaces"), "got {s}");
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
