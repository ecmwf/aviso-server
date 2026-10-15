// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! RFC 6750 bearer challenges for `/mcp`. Every challenge points at the
//! RFC 9728 protected resource metadata, so clients can find the issuer.

use crate::mcp::auth::{OAuthError, TokenError};
use crate::mcp::state::McpState;
use actix_web::http::StatusCode;
use serde_json::{Value, json};

/// A refusal that asks for a (better) bearer token.
pub(super) struct Challenge {
    /// `401` or `403`.
    pub(super) status: StatusCode,
    /// Value of the `WWW-Authenticate` header.
    pub(super) header: String,
    /// JSON body of the response.
    pub(super) body: Value,
}

/// The OAuth error a challenge reports for a refused token.
pub(super) struct OAuthFault {
    /// RFC 6750 error code and its status.
    error: OAuthError,
    /// Which check failed. Token errors never contain token content.
    description: String,
}

impl OAuthFault {
    /// The fault to report for `error`, or `None` when the failure is on
    /// the server side and a new token would not help.
    pub(super) fn for_token_error(error: &TokenError) -> Option<Self> {
        error.oauth_error().map(|oauth_error| Self {
            error: oauth_error,
            description: error_description(&error.to_string()),
        })
    }

    /// RFC 6750 error code, such as `invalid_token`.
    pub(super) fn code(&self) -> &'static str {
        self.error.code()
    }
}

/// RFC 6750 and RFC 9728 challenge pointing at the resource metadata.
///
/// - no token: `Bearer resource_metadata="...", scope="aviso:read"`
/// - bad token: `Bearer error="invalid_token", error_description="...",
///   resource_metadata="...", scope="aviso:read"`
pub(super) fn challenge(state: &McpState, fault: Option<&OAuthFault>) -> Challenge {
    let error_part = fault
        .map(|fault| {
            format!(
                "error=\"{}\", error_description=\"{}\", ",
                fault.code(),
                fault.description
            )
        })
        .unwrap_or_default();
    let header = format!(
        "Bearer {error_part}resource_metadata=\"{}\", scope=\"{}\"",
        state.metadata_url,
        state.verifier.required_scope_string()
    );
    let (status, body) = match fault {
        Some(fault) => (
            fault.error.status(),
            json!({"error": fault.code(), "error_description": fault.description}),
        ),
        None => (StatusCode::UNAUTHORIZED, json!({"error": "unauthorized"})),
    };
    Challenge {
        status,
        header,
        body,
    }
}

/// Limit `text` to the characters RFC 6750 section 3 allows in
/// `error_description` (%x20-21 / %x23-5B / %x5D-7E); others become a
/// space.
///
/// - kept: `token is invalid: wrong audience`
/// - replaced: `"`, `\`, a newline, `é`
fn error_description(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            ' '..='~' if c != '"' && c != '\\' => c,
            _ => ' ',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::identity::IdentityClaimError;

    #[test]
    fn error_descriptions_use_only_allowed_characters() {
        assert_eq!(
            error_description("token is invalid: wrong audience"),
            "token is invalid: wrong audience"
        );
        assert_eq!(error_description("a\"b\\c\nd\u{e9}"), "a b c d ");
        let error = IdentityClaimError::RealmFormat("realm".to_string());
        let identity = TokenError::Identity(error).to_string();
        assert_eq!(error_description(&identity), identity);
    }

    #[test]
    fn token_errors_map_to_oauth_faults() {
        let scope = OAuthFault::for_token_error(&TokenError::InsufficientScope).unwrap();
        assert_eq!(
            (scope.code(), scope.error.status()),
            ("insufficient_scope", StatusCode::FORBIDDEN)
        );
        let invalid = OAuthFault::for_token_error(&TokenError::Malformed).unwrap();
        assert_eq!(
            (invalid.code(), invalid.error.status()),
            ("invalid_token", StatusCode::UNAUTHORIZED)
        );
        assert_eq!(invalid.description, "token is malformed");
        assert!(OAuthFault::for_token_error(&TokenError::KeysUnavailable).is_none());
    }
}
