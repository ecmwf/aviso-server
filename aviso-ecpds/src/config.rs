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

/// How to merge per-server destination lists when more than one ECPDS
/// server is configured.
///
/// Operational trade-off:
///
/// - [`PartialOutagePolicy::Strict`] (default) — every configured
///   server must respond successfully **and** return the same set of
///   destinations. Any server failure or any divergence between
///   servers fails the lookup with [`crate::EcpdsError::ServiceUnavailable`].
///   This is the confidentiality-preserving default: under a
///   replication lag or partial outage, denying access is preferable
///   to potentially granting access based on a stale or incomplete
///   server's view.
///
/// - [`PartialOutagePolicy::AnySuccess`] — succeed as soon as any one
///   server responds; union the destination lists across reachable
///   servers. This is more available but lets a single permissive (or
///   compromised) server widen access. Loud `tracing::warn!` is
///   emitted on divergence so on-call sees it.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PartialOutagePolicy {
    #[default]
    Strict,
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
/// - `partial_outage_policy = Strict`: see
///   [`PartialOutagePolicy`] for the security trade-off.
#[derive(Deserialize, Serialize, Clone)]
pub struct EcpdsConfig {
    pub username: String,
    pub password: String,
    #[serde(default = "default_target_field")]
    pub target_field: String,
    pub match_key: String,
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_seconds: u64,
    #[serde(default = "default_max_entries")]
    pub max_entries: u64,
    #[serde(default = "default_partial_outage_policy")]
    pub partial_outage_policy: PartialOutagePolicy,
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
