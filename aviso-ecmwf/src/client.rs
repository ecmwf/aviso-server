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
