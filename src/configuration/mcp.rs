// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Settings for the optional MCP endpoint (`/mcp`).
//!
//! The block is disabled by default. While disabled only the field types are
//! checked; [`McpSettings::validate`] enforces every other rule once
//! `enabled` is true.

use crate::configuration::{
    MCP_REALM_MAX_CHARS, MCP_ROLE_MAX_CHARS, McpIdentityError, McpIdentitySettings,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Hard ceiling for [`McpSettings::max_results`].
pub const MCP_MAX_RESULTS_CEILING: usize = 200;

/// Path the MCP endpoint is mounted at. `resource_url` must end with it.
pub const MCP_PATH: &str = "/mcp";

/// Top-level `mcp` configuration block.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct McpSettings {
    /// Mount `/mcp` and the protected resource metadata routes.
    pub enabled: bool,
    /// Public URL of the MCP endpoint. Tokens must name it as audience.
    pub resource_url: String,
    /// Browser origins allowed to call `/mcp`. Requests without an
    /// `Origin` header are always allowed.
    pub allowed_origins: Vec<String>,
    /// Upper bound on notifications one tool call may return.
    pub max_results: usize,
    /// Deadline for one tool call that reads the backend (milliseconds).
    pub request_timeout_ms: u64,
    /// Authorization server that issues the access tokens.
    pub oauth: McpOAuthSettings,
    /// How token subjects map onto Aviso users.
    pub identity: McpIdentitySettings,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            resource_url: String::new(),
            allowed_origins: Vec::new(),
            max_results: 100,
            request_timeout_ms: 10_000,
            oauth: McpOAuthSettings::default(),
            identity: McpIdentitySettings::default(),
        }
    }
}

/// `mcp.oauth`: token issuer and key source.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct McpOAuthSettings {
    /// Expected `iss` claim, compared byte for byte.
    pub issuer: String,
    /// HTTPS URL of the issuer's JSON Web Key Set.
    pub jwks_uri: String,
    /// Scopes every token must carry.
    pub required_scopes: Vec<String>,
    /// How long a fetched key set stays fresh (seconds).
    pub jwks_cache_ttl_seconds: u64,
}

impl Default for McpOAuthSettings {
    fn default() -> Self {
        Self {
            issuer: String::new(),
            jwks_uri: String::new(),
            required_scopes: vec!["aviso:read".to_string()],
            jwks_cache_ttl_seconds: 300,
        }
    }
}

/// A rule the enabled `mcp` block violates.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum McpConfigError {
    /// A URL field is empty, unparsable, or not HTTPS.
    #[error("mcp.{field} must be an absolute https URL without query or fragment")]
    InvalidUrl { field: &'static str },
    /// `resource_url` does not point at the mounted endpoint.
    #[error("mcp.resource_url path must be exactly '{MCP_PATH}'")]
    ResourcePath,
    /// An `allowed_origins` entry is not a bare origin.
    #[error("mcp.allowed_origins entry '{0}' must be a bare origin like https://host:port")]
    InvalidOrigin(String),
    /// `max_results` is outside `1..=200`.
    #[error("mcp.max_results must be between 1 and {MCP_MAX_RESULTS_CEILING}")]
    MaxResults,
    /// A duration field is zero.
    #[error("mcp.{0} must be greater than zero")]
    ZeroDuration(&'static str),
    /// `required_scopes` is empty or has an empty or spaced entry.
    #[error("mcp.oauth.required_scopes must be non-empty scope tokens without spaces")]
    Scopes,
    /// The `identity` block is invalid.
    #[error(transparent)]
    Identity(#[from] McpIdentityError),
    /// The fixed `identity.realm` is a realm the REST auth configuration
    /// uses.
    #[error(
        "mcp.identity.realm '{realm}' is also a key of {location}; \
         pick a realm that no REST user can have"
    )]
    RealmInUse { realm: String, location: String },
    /// With token claims, a REST realm is not a realm a token can carry.
    #[error(
        "mcp.identity.source is token_claims, but realm '{realm}' in {location} \
         cannot appear in a token: token realms are 1 to {MCP_REALM_MAX_CHARS} \
         characters of A-Z, a-z, 0-9, '.', '_' or '-'"
    )]
    RealmNotCarried { realm: String, location: String },
    /// With token claims, a configured REST role is not a role a token
    /// can carry.
    #[error(
        "mcp.identity.source is token_claims, but role {role:?} in {location} \
         cannot appear in a token: token roles are 1 to {MCP_ROLE_MAX_CHARS} \
         characters without surrounding whitespace or control characters"
    )]
    RoleNotCarried { role: String, location: String },
}

impl McpSettings {
    /// Check the enabled block. A disabled block always passes.
    pub fn validate(&self) -> Result<(), McpConfigError> {
        if !self.enabled {
            return Ok(());
        }
        let resource = parse_https("resource_url", &self.resource_url)?;
        if resource.path() != MCP_PATH {
            return Err(McpConfigError::ResourcePath);
        }
        parse_https("oauth.issuer", &self.oauth.issuer)?;
        parse_https("oauth.jwks_uri", &self.oauth.jwks_uri)?;
        for origin in &self.allowed_origins {
            validate_origin(origin)?;
        }
        if !(1..=MCP_MAX_RESULTS_CEILING).contains(&self.max_results) {
            return Err(McpConfigError::MaxResults);
        }
        if self.request_timeout_ms == 0 {
            return Err(McpConfigError::ZeroDuration("request_timeout_ms"));
        }
        if self.oauth.jwks_cache_ttl_seconds == 0 {
            return Err(McpConfigError::ZeroDuration("oauth.jwks_cache_ttl_seconds"));
        }
        let scopes_ok = !self.oauth.required_scopes.is_empty()
            && self
                .oauth
                .required_scopes
                .iter()
                .all(|scope| !scope.is_empty() && !scope.chars().any(char::is_whitespace));
        if !scopes_ok {
            return Err(McpConfigError::Scopes);
        }
        self.identity.validate()?;
        Ok(())
    }
}

/// Parse an absolute HTTPS URL with a host and no query or fragment.
fn parse_https(field: &'static str, raw: &str) -> Result<Url, McpConfigError> {
    let invalid = McpConfigError::InvalidUrl { field };
    let url = Url::parse(raw).map_err(|_| invalid.clone())?;
    let ok = url.scheme() == "https"
        && url.host_str().is_some()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    ok.then_some(url).ok_or(invalid)
}

/// Accept only the serialized origin form, as browsers send it.
///
/// - valid: `https://claude.ai`, `http://localhost:6274`
/// - invalid: `https://claude.ai/`, `https://claude.ai/path`, `*`
fn validate_origin(origin: &str) -> Result<(), McpConfigError> {
    let invalid = || McpConfigError::InvalidOrigin(origin.to_string());
    let url = Url::parse(origin).map_err(|_| invalid())?;
    let is_http = matches!(url.scheme(), "http" | "https");
    if is_http && url.origin().ascii_serialization() == origin {
        Ok(())
    } else {
        Err(invalid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn enabled() -> McpSettings {
        serde_json::from_value(json!({
            "enabled": true,
            "resource_url": "https://aviso.example.int/mcp",
            "allowed_origins": ["http://localhost:6274"],
            "oauth": {
                "issuer": "https://auth.example.int/",
                "jwks_uri": "https://auth.example.int/.well-known/jwks.json"
            }
        }))
        .unwrap()
    }

    #[test]
    fn defaults_are_disabled_and_bounded() {
        let settings: McpSettings = serde_json::from_str("{}").unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.max_results, 100);
        assert_eq!(settings.oauth.required_scopes, vec!["aviso:read"]);
        assert_eq!(settings.identity, McpIdentitySettings::default());
        assert!(settings.allowed_origins.is_empty());
    }

    #[test]
    fn disabled_block_skips_value_checks() {
        let settings: McpSettings =
            serde_json::from_value(json!({"resource_url": "not a url", "max_results": 0})).unwrap();
        assert_eq!(settings.validate(), Ok(()));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let nested = serde_json::from_value::<McpSettings>(json!({"oauth": {"audience": "x"}}));
        assert!(nested.is_err());
        let top = serde_json::from_value::<McpSettings>(json!({"endpoint": "/mcp"}));
        assert!(top.is_err());
    }

    #[test]
    fn valid_enabled_block_passes() {
        assert_eq!(enabled().validate(), Ok(()));
    }

    #[test]
    fn enabled_block_requires_https_urls() {
        let mut settings = enabled();
        settings.oauth.issuer = String::new();
        assert_eq!(
            settings.validate(),
            Err(McpConfigError::InvalidUrl {
                field: "oauth.issuer"
            })
        );
        let mut settings = enabled();
        settings.oauth.jwks_uri = "http://auth.example.int/jwks".to_string();
        assert!(matches!(
            settings.validate(),
            Err(McpConfigError::InvalidUrl { .. })
        ));
        let mut settings = enabled();
        settings.resource_url = "https://aviso.example.int/mcp?x=1".to_string();
        assert!(matches!(
            settings.validate(),
            Err(McpConfigError::InvalidUrl { .. })
        ));
    }

    #[test]
    fn resource_url_must_end_at_the_mount_path() {
        let mut settings = enabled();
        settings.resource_url = "https://aviso.example.int/api/mcp".to_string();
        assert_eq!(settings.validate(), Err(McpConfigError::ResourcePath));
    }

    #[test]
    fn origins_must_be_bare() {
        for origin in ["https://claude.ai/", "*", "https://x.int/path"] {
            let mut settings = enabled();
            settings.allowed_origins = vec![origin.to_string()];
            assert!(
                matches!(settings.validate(), Err(McpConfigError::InvalidOrigin(_))),
                "{origin}"
            );
        }
    }

    #[test]
    fn numeric_bounds_are_enforced() {
        for max in [0, MCP_MAX_RESULTS_CEILING + 1] {
            let mut settings = enabled();
            settings.max_results = max;
            assert_eq!(settings.validate(), Err(McpConfigError::MaxResults));
        }
        let mut settings = enabled();
        settings.request_timeout_ms = 0;
        assert!(matches!(
            settings.validate(),
            Err(McpConfigError::ZeroDuration(_))
        ));
        let mut settings = enabled();
        settings.oauth.jwks_cache_ttl_seconds = 0;
        assert!(matches!(
            settings.validate(),
            Err(McpConfigError::ZeroDuration(_))
        ));
    }

    #[test]
    fn scopes_and_identity_are_checked() {
        let mut settings = enabled();
        settings.oauth.required_scopes = vec!["aviso read".to_string()];
        assert_eq!(settings.validate(), Err(McpConfigError::Scopes));
        let mut settings = enabled();
        settings.oauth.required_scopes.clear();
        assert_eq!(settings.validate(), Err(McpConfigError::Scopes));
        let mut settings = enabled();
        settings.identity = McpIdentitySettings::Fixed {
            realm: " mcp".to_string(),
        };
        assert_eq!(
            settings.validate(),
            Err(McpConfigError::Identity(McpIdentityError::Realm))
        );
        let mut settings = enabled();
        settings.identity =
            serde_json::from_value(json!({"source": "token_claims", "realm_claim": "sub"}))
                .unwrap();
        let error = settings.validate().unwrap_err();
        assert!(error.to_string().contains("realm_claim 'sub'"), "{error}");
    }
}
