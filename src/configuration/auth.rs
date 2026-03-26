// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt;

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    Direct,
    TrustedProxy,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct AuthSettings {
    pub enabled: bool,
    pub mode: AuthMode,
    pub auth_o_tron_url: String,
    #[serde(serialize_with = "serialize_redacted_jwt_secret")]
    pub jwt_secret: String,
    pub admin_roles: HashMap<String, Vec<String>>,
    pub timeout_ms: u64,
}

fn serialize_redacted_jwt_secret<S>(_: &String, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str("[REDACTED]")
}

impl fmt::Debug for AuthSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthSettings")
            .field("enabled", &self.enabled)
            .field("mode", &self.mode)
            .field("auth_o_tron_url", &self.auth_o_tron_url)
            .field("jwt_secret", &"[REDACTED]")
            .field("admin_roles", &self.admin_roles)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

impl Default for AuthSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: AuthMode::Direct,
            auth_o_tron_url: String::new(),
            jwt_secret: String::new(),
            admin_roles: HashMap::new(),
            timeout_ms: 5_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthMode, AuthSettings};
    use std::collections::HashMap;

    #[test]
    fn auth_settings_default_to_disabled() {
        let settings: AuthSettings = serde_json::from_str("{}").expect("should deserialize");

        assert!(!settings.enabled);
        assert_eq!(settings.mode, AuthMode::Direct);
        assert!(settings.auth_o_tron_url.is_empty());
        assert!(settings.jwt_secret.is_empty());
        assert!(settings.admin_roles.is_empty());
        assert_eq!(settings.timeout_ms, 5_000);
    }

    #[test]
    fn auth_settings_deserialize_explicit_values() {
        let settings: AuthSettings = serde_json::from_str(
            r#"{
                "enabled": true,
                "mode": "trusted_proxy",
                "auth_o_tron_url": "http://auth-o-tron:8080",
                "jwt_secret": "top-secret",
                "admin_roles": {"ecmwf": ["admin", "operator"]},
                "timeout_ms": 1200
            }"#,
        )
        .expect("should deserialize");

        assert!(settings.enabled);
        assert_eq!(settings.mode, AuthMode::TrustedProxy);
        assert_eq!(settings.auth_o_tron_url, "http://auth-o-tron:8080");
        assert_eq!(settings.jwt_secret, "top-secret");
        assert_eq!(
            settings.admin_roles.get("ecmwf").map(Vec::as_slice),
            Some(["admin".to_string(), "operator".to_string()].as_slice())
        );
        assert_eq!(settings.timeout_ms, 1200);
    }

    #[test]
    fn auth_settings_reject_unknown_fields() {
        let result: Result<AuthSettings, _> = serde_json::from_str(
            r#"{
                "enabled": true,
                "unknown_field": "value"
            }"#,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown_field"));
    }

    #[test]
    fn auth_settings_debug_redacts_jwt_secret() {
        let settings = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "super-secret".to_string(),
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
            timeout_ms: 5000,
        };

        let debug = format!("{settings:?}");
        assert!(debug.contains("jwt_secret: \"[REDACTED]\""));
        assert!(!debug.contains("super-secret"));
    }

    #[test]
    fn auth_settings_serialize_redacts_jwt_secret() {
        let settings = AuthSettings {
            enabled: true,
            mode: AuthMode::Direct,
            auth_o_tron_url: "http://auth-o-tron:8080".to_string(),
            jwt_secret: "super-secret".to_string(),
            admin_roles: HashMap::from([("testrealm".to_string(), vec!["admin".to_string()])]),
            timeout_ms: 5000,
        };

        let serialized = serde_json::to_value(&settings).expect("settings should serialize");
        assert_eq!(
            serialized
                .get("jwt_secret")
                .and_then(|value| value.as_str()),
            Some("[REDACTED]")
        );
    }
}
