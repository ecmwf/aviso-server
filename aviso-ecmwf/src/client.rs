use crate::config::EcpdsConfig;
use futures::future::join_all;
use serde::Deserialize;
use tracing::{debug, warn};

#[derive(Debug)]
pub enum EcpdsError {
    ServiceUnavailable,
    AccessDenied(String),
    InvalidResponse(String),
}

impl std::fmt::Display for EcpdsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ServiceUnavailable => write!(f, "ECPDS service is unaccessible"),
            Self::AccessDenied(msg) => write!(f, "Access denied: {}", msg),
            Self::InvalidResponse(msg) => write!(f, "Invalid response from ECPDS: {}", msg),
        }
    }
}

impl std::error::Error for EcpdsError {}

#[derive(Deserialize)]
struct EcpdsResponse {
    #[serde(rename = "destinationList")]
    destination_list: Vec<serde_json::Value>,
    #[allow(dead_code)]
    success: String,
}

pub struct EcpdsClient {
    http: reqwest::Client,
    servers: Vec<String>,
    username: String,
    password: String,
    target_field: String,
}

impl EcpdsClient {
    pub fn new(config: &EcpdsConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to build reqwest client");

        Self {
            http,
            servers: config.servers.clone(),
            username: config.username.clone(),
            password: config.password.clone(),
            target_field: config.target_field.clone(),
        }
    }

    /// Query all configured ECPDS servers in parallel for a user's destinations,
    /// merge and deduplicate the results.
    pub async fn fetch_user_destinations(&self, username: &str) -> Result<Vec<String>, EcpdsError> {
        let futures = self.servers.iter().map(|server| {
            let url = format!("{}/ecpds/v1/destination/list?id={}", server, username);
            self.fetch_from_server(url)
        });

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

    async fn fetch_from_server(&self, url: String) -> Result<Vec<String>, String> {
        let response = self
            .http
            .get(&url)
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
        let client = EcpdsClient::new(&config);
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
        let client = EcpdsClient::new(&config);
        let mut result = client.fetch_user_destinations("testuser").await.unwrap();
        result.sort();

        assert_eq!(result, vec!["BAR", "CIP", "FOO"]);
    }

    #[tokio::test]
    async fn fetch_returns_service_unavailable_when_all_servers_down() {
        let config = make_config(vec!["http://localhost:1".to_string()]);
        let client = EcpdsClient::new(&config);
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
        let client = EcpdsClient::new(&config);
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
        let client = EcpdsClient::new(&config);
        let result = client.fetch_user_destinations("testuser").await.unwrap();
        assert!(result.contains(&"DEST1".to_string()));
        assert!(!result.contains(&"CIP".to_string()));
    }
}
