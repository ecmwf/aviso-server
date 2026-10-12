// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Shared state for the MCP routes, built once at startup.

use crate::configuration::McpSettings;
use crate::mcp::auth::TokenVerifier;
use crate::mcp::jwks::JwksError;
use reqwest::Url;
use thiserror::Error;

/// Path prefix of RFC 9728 protected resource metadata.
pub const METADATA_PREFIX: &str = "/.well-known/oauth-protected-resource";

/// Why the MCP routes could not be prepared.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum McpInitError {
    /// `resource_url` is not an absolute URL.
    #[error("mcp.resource_url is not a valid URL")]
    ResourceUrl,
    /// The token verifier could not be built.
    #[error(transparent)]
    Jwks(#[from] JwksError),
}

/// Everything the MCP handlers need besides the backend.
pub struct McpState {
    /// The validated `mcp` block.
    pub settings: McpSettings,
    /// Bearer token verifier.
    pub verifier: TokenVerifier,
    /// Absolute URL of this resource's metadata document.
    pub metadata_url: String,
}

impl McpState {
    /// Build state from validated settings. No network calls are made.
    pub fn from_settings(settings: &McpSettings) -> Result<Self, McpInitError> {
        Self::new(settings, TokenVerifier::from_settings(settings)?)
    }

    /// Build state around an existing verifier.
    pub fn new(settings: &McpSettings, verifier: TokenVerifier) -> Result<Self, McpInitError> {
        Ok(Self {
            metadata_url: metadata_url(&settings.resource_url)?,
            settings: settings.clone(),
            verifier,
        })
    }
}

/// RFC 9728 section 3.1: insert the well-known prefix between the origin
/// and the resource path.
///
/// - `https://a.int/mcp` gives
///   `https://a.int/.well-known/oauth-protected-resource/mcp`
/// - `not a url` is an error
fn metadata_url(resource_url: &str) -> Result<String, McpInitError> {
    let url = Url::parse(resource_url).map_err(|_| McpInitError::ResourceUrl)?;
    let path = url.path().trim_end_matches('/');
    Ok(format!(
        "{}{METADATA_PREFIX}{path}",
        url.origin().ascii_serialization()
    ))
}

#[cfg(test)]
mod tests {
    use super::metadata_url;

    #[test]
    fn metadata_url_follows_rfc_9728() {
        assert_eq!(
            metadata_url("https://a.int:8443/mcp").unwrap(),
            "https://a.int:8443/.well-known/oauth-protected-resource/mcp"
        );
        assert!(metadata_url("not a url").is_err());
    }
}
