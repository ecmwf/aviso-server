use crate::config::EcpdsConfig;
use futures::future::join_all;
use serde::Deserialize;
use thiserror::Error;
use tracing::{debug, warn};

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
            .timeout(std::time::Duration::from_secs(30))
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
        })
    }

    /// Query all configured ECPDS servers in parallel for a user's destinations,
    /// merge and deduplicate the results.
    pub async fn fetch_user_destinations(&self, username: &str) -> Result<Vec<String>, EcpdsError> {
        let futures = self
            .servers
            .iter()
            .map(|server| self.fetch_from_server(server, username));

        let results = join_all(futures).await;

        let mut all_destinations: Vec<String> = Vec::new();
        let mut any_success = false;

        for (i, result) in results.into_iter().enumerate() {
            match result {
                Ok(mut dests) => {
                    any_success = true;
                    all_destinations.append(&mut dests);
                    debug!(
                        "ECPDS server {} returned destinations for user {}",
                        self.servers[i], username
                    );
                }
                Err(e) => {
                    warn!("ECPDS server {} failed: {}", self.servers[i], e);
                }
            }
        }

        if !any_success {
            return Err(EcpdsError::ServiceUnavailable);
        }

        all_destinations.sort();
        all_destinations.dedup();

        Ok(all_destinations)
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

        let destinations = ecpds_resp
            .destination_list
            .into_iter()
            .filter_map(|record| {
                record
                    .get(&self.target_field)
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();

        Ok(destinations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EcpdsConfig;

    fn make_config(servers: Vec<String>) -> EcpdsConfig {
        EcpdsConfig {
            username: "testuser".to_string(),
            password: "testpass".to_string(),
            target_field: "name".to_string(),
            match_key: "destination".to_string(),
            cache_ttl_seconds: 300,
            max_entries: 1000,
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
    async fn fetch_merges_and_deduplicates_multi_server() {
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

        let config = make_config(vec![server_a.url(), server_b.url()]);
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
    async fn fetch_succeeds_when_one_server_is_down() {
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

        let config = make_config(vec!["http://localhost:1".to_string(), server.url()]);
        let client = EcpdsClient::new(&config).expect("client must build");
        let result = client.fetch_user_destinations("testuser").await.unwrap();
        assert!(result.contains(&"CIP".to_string()));
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
