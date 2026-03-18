use serde::{Deserialize, Serialize};
use std::fmt;

fn default_cache_ttl() -> u64 {
    300
}

fn default_target_field() -> String {
    "name".to_string()
}

#[derive(Deserialize, Serialize, Clone)]
pub struct EcpdsConfig {
    pub username: String,
    pub password: String,
    #[serde(default = "default_target_field")]
    pub target_field: String,
    pub match_key: String,
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_seconds: u64,
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
    }

    #[test]
    fn test_full_deserialization() {
        let json = r#"{
            "username": "testuser",
            "password": "testpass",
            "target_field": "custom_field",
            "match_key": "destination",
            "cache_ttl_seconds": 600,
            "servers": ["http://server1.example.com", "http://server2.example.com"]
        }"#;

        let config: EcpdsConfig = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(config.username, "testuser");
        assert_eq!(config.password, "testpass");
        assert_eq!(config.target_field, "custom_field");
        assert_eq!(config.match_key, "destination");
        assert_eq!(config.cache_ttl_seconds, 600);
        assert_eq!(config.servers.len(), 2);
        assert_eq!(config.servers[0], "http://server1.example.com");
        assert_eq!(config.servers[1], "http://server2.example.com");
    }
}
