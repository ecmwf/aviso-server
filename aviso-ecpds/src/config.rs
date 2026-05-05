use serde::{Deserialize, Serialize};
use std::fmt;

fn default_cache_ttl() -> u64 {
    300
}

fn default_target_field() -> String {
    "name".to_string()
}

fn default_max_entries() -> u64 {
    10_000
}

fn default_request_timeout() -> u64 {
    30
}

fn default_connect_timeout() -> u64 {
    5
}

/// How to handle per-server failures when more than one ECPDS server
/// is configured.
///
/// Both policies build the user's destination list as the **union** of
/// per-server responses. They differ only in failure tolerance:
///
/// - [`PartialOutagePolicy::Strict`] (default): every configured
///   server must return successfully within the per-request timeout.
///   If any one fails, the whole call fails with
///   [`crate::EcpdsError::ServiceUnavailable`]. Use this when every
///   configured server is critical for completeness and you would
///   rather return 503 than potentially deny access the user actually
///   has on an unreachable server.
///
/// - [`PartialOutagePolicy::AnySuccess`]: succeed as long as at least
///   one server returned successfully. Servers that timed out or
///   failed are silently dropped from the merge. Use this when each
///   server is independently authoritative for its own slice and the
///   user's effective access is best approximated by the union of
///   whichever servers were reachable.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PartialOutagePolicy {
    /// All configured servers must respond successfully. The user's
    /// destination list is the union of their responses. Any per-server
    /// failure aborts the call with `ServiceUnavailable`.
    #[default]
    Strict,
    /// Take the union of whichever servers responded successfully
    /// within the per-request timeout. Only fails when zero servers
    /// returned a usable response.
    AnySuccess,
}

fn default_partial_outage_policy() -> PartialOutagePolicy {
    PartialOutagePolicy::default()
}

/// Static configuration for the ECPDS authorization plugin.
///
/// Field defaults are operationally meaningful:
/// - `cache_ttl_seconds = 300`: 5-minute lookahead is short enough to
///   pick up most operational role changes within a typical SSE
///   reconnect window.
/// - `max_entries = 10_000`: bounded to prevent unbounded growth from
///   high-cardinality usernames; eviction policy is moka's TinyLFU.
/// - `target_field = "name"`: the field of each ECPDS destination
///   record whose string value is matched against the request's
///   `match_key`.
/// - `partial_outage_policy = Strict`: see [`PartialOutagePolicy`]
///   for the failure-tolerance trade-off between Strict and AnySuccess.
#[derive(Deserialize, Serialize, Clone)]
pub struct EcpdsConfig {
    /// Service-account username for HTTP Basic Auth to ECPDS.
    pub username: String,
    /// Service-account password. Redacted in [`fmt::Debug`] output and
    /// in the redacted `Settings` dump emitted at startup.
    pub password: String,
    /// JSON field of each ECPDS destination record whose string value
    /// is matched against the request's `match_key` value. Default
    /// `"name"`.
    #[serde(default = "default_target_field")]
    pub target_field: String,
    /// Identifier field whose value is treated as the destination to
    /// authorise. Must appear in the schema's `topic.key_order` and
    /// be marked `required: true` in the schema's `identifier`.
    pub match_key: String,
    /// How long to cache a user's destination list before re-fetching
    /// (seconds). Default 300.
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_seconds: u64,
    /// Maximum number of distinct usernames held in the cache before
    /// TinyLFU eviction. Default 10 000.
    #[serde(default = "default_max_entries")]
    pub max_entries: u64,
    /// Total wall-clock timeout for a single ECPDS HTTP request.
    /// Default 30 seconds.
    #[serde(default = "default_request_timeout")]
    pub request_timeout_seconds: u64,
    /// TCP + TLS handshake timeout for a single ECPDS HTTP request.
    /// Default 5 seconds.
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_seconds: u64,
    /// How to merge per-server destination lists when more than one
    /// ECPDS server is configured. Default
    /// [`PartialOutagePolicy::Strict`].
    #[serde(default = "default_partial_outage_policy")]
    pub partial_outage_policy: PartialOutagePolicy,
    /// List of ECPDS server base URLs (each `http://...` or
    /// `https://...`, no query string, no fragment; trailing slashes
    /// are normalised).
    pub servers: Vec<String>,
}

impl fmt::Debug for EcpdsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EcpdsConfig")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("target_field", &self.target_field)
            .field("match_key", &self.match_key)
            .field("cache_ttl_seconds", &self.cache_ttl_seconds)
            .field("max_entries", &self.max_entries)
            .field("request_timeout_seconds", &self.request_timeout_seconds)
            .field("connect_timeout_seconds", &self.connect_timeout_seconds)
            .field("partial_outage_policy", &self.partial_outage_policy)
            .field("servers", &self.servers)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_redacted_in_debug() {
        let config = EcpdsConfig {
            username: "testuser".to_string(),
            password: "super-secret-password".to_string(),
            target_field: "name".to_string(),
            match_key: "destination".to_string(),
            cache_ttl_seconds: 300,
            max_entries: 10_000,
            request_timeout_seconds: 30,
            connect_timeout_seconds: 5,
            partial_outage_policy: PartialOutagePolicy::Strict,
            servers: vec!["http://server1.example.com".to_string()],
        };

        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("[REDACTED]"));
        assert!(!debug_str.contains("super-secret-password"));
    }

    #[test]
    fn test_defaults_applied() {
        let json = r#"{
            "username": "testuser",
            "password": "testpass",
            "match_key": "destination",
            "servers": ["http://server1.example.com"]
        }"#;

        let config: EcpdsConfig = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(config.cache_ttl_seconds, 300);
        assert_eq!(config.target_field, "name");
        assert_eq!(config.max_entries, 10_000);
        assert_eq!(config.request_timeout_seconds, 30);
        assert_eq!(config.connect_timeout_seconds, 5);
        assert_eq!(config.partial_outage_policy, PartialOutagePolicy::Strict);
    }

    #[test]
    fn test_full_deserialization() {
        let json = r#"{
            "username": "testuser",
            "password": "testpass",
            "target_field": "custom_field",
            "match_key": "destination",
            "cache_ttl_seconds": 600,
            "max_entries": 5000,
            "servers": ["http://server1.example.com", "http://server2.example.com"]
        }"#;

        let config: EcpdsConfig = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(config.username, "testuser");
        assert_eq!(config.password, "testpass");
        assert_eq!(config.target_field, "custom_field");
        assert_eq!(config.match_key, "destination");
        assert_eq!(config.cache_ttl_seconds, 600);
        assert_eq!(config.max_entries, 5000);
        assert_eq!(config.servers.len(), 2);
        assert_eq!(config.servers[0], "http://server1.example.com");
        assert_eq!(config.servers[1], "http://server2.example.com");
    }
}
