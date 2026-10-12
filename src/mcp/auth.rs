// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Access token verification for the MCP endpoint.
//!
//! Tokens are RFC 9068 JWT access tokens signed with RS256 by the configured
//! issuer. A verified token maps to an Aviso [`User`] in the configured MCP
//! realm. The mapping never grants admin rights: callers check stream access
//! with an empty admin role map.

use crate::auth::User;
use crate::configuration::McpSettings;
use crate::mcp::jwks::{JwksCache, JwksError, JwksTiming};
use jsonwebtoken::{Algorithm, Validation, decode, decode_header};
use serde::Deserialize;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;
use thiserror::Error;

/// Clock skew tolerated on `exp`, `nbf`, and `iat` (seconds).
const LEEWAY_SECONDS: u64 = 30;
/// Longest bearer token accepted before any parsing.
const MAX_TOKEN_BYTES: usize = 16 * 1024;
/// Most roles taken from one token.
const MAX_ROLES: usize = 64;
/// Longest role or subject accepted.
const MAX_CLAIM_CHARS: usize = 256;

/// Why a bearer token was refused.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TokenError {
    /// Not a decodable JWT, or larger than the size limit.
    #[error("token is malformed")]
    Malformed,
    /// Header `alg` is not RS256.
    #[error("token algorithm is not RS256")]
    Algorithm,
    /// Header `typ` is not `at+jwt`.
    #[error("token type is not at+jwt")]
    Type,
    /// Header has no `kid`.
    #[error("token has no key id")]
    MissingKeyId,
    /// No current signing key has the token's `kid`.
    #[error("token signing key is unknown")]
    UnknownKey,
    /// Signature, issuer, audience, or time checks failed.
    #[error("token is invalid: {0}")]
    Rejected(&'static str),
    /// Required claims are missing or out of bounds.
    #[error("token claims are invalid: {0}")]
    Claims(&'static str),
    /// The token lacks one of the required scopes.
    #[error("token lacks a required scope")]
    InsufficientScope,
    /// Signing keys could not be loaded, so nothing can be verified.
    #[error("token signing keys are unavailable")]
    KeysUnavailable,
}

impl TokenError {
    /// The OAuth error code for the `WWW-Authenticate` header, or `None`
    /// when the failure is on the server side.
    pub fn oauth_error(&self) -> Option<&'static str> {
        match self {
            Self::InsufficientScope => Some("insufficient_scope"),
            Self::KeysUnavailable => None,
            _ => Some("invalid_token"),
        }
    }
}

/// A verified token mapped onto an Aviso user.
#[derive(Debug, Clone)]
pub struct VerifiedToken {
    /// User for stream access checks; never an admin.
    pub user: User,
    /// Short pseudonym of the subject, for debug logs only. See
    /// [`subject_hash`] for how long it stays the same.
    pub subject_hash: String,
}

#[derive(Deserialize)]
struct AccessClaims {
    sub: String,
    iat: u64,
    #[serde(default)]
    jti: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    roles: Option<Vec<String>>,
}

/// Verifies bearer tokens against one issuer, audience, and scope set.
pub struct TokenVerifier {
    jwks: JwksCache,
    validation: Validation,
    required_scopes: Vec<String>,
    realm: String,
}

impl TokenVerifier {
    /// Build a verifier from validated MCP settings.
    pub fn from_settings(settings: &McpSettings) -> Result<Self, JwksError> {
        let ttl = Duration::from_secs(settings.oauth.jwks_cache_ttl_seconds);
        let jwks = JwksCache::new(&settings.oauth.jwks_uri, JwksTiming::with_ttl(ttl))?;
        Ok(Self::new(jwks, settings))
    }

    /// Build a verifier around an existing key cache.
    pub fn new(jwks: JwksCache, settings: &McpSettings) -> Self {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&settings.oauth.issuer]);
        validation.set_audience(&[&settings.resource_url]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.validate_nbf = true;
        validation.leeway = LEEWAY_SECONDS;
        Self {
            jwks,
            validation,
            required_scopes: settings.oauth.required_scopes.clone(),
            realm: settings.identity.realm.clone(),
        }
    }

    /// Scopes a token must carry, space separated, for challenges.
    pub fn required_scope_string(&self) -> String {
        self.required_scopes.join(" ")
    }

    /// Verify `token` and map it to a user.
    pub async fn verify(&self, token: &str) -> Result<VerifiedToken, TokenError> {
        if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
            return Err(TokenError::Malformed);
        }
        let header = decode_header(token).map_err(|_| TokenError::Malformed)?;
        if header.alg != Algorithm::RS256 {
            return Err(TokenError::Algorithm);
        }
        if !header.typ.as_deref().is_some_and(is_access_token_type) {
            return Err(TokenError::Type);
        }
        let kid = header.kid.ok_or(TokenError::MissingKeyId)?;
        let key = self.jwks.key(&kid).await.map_err(|error| match error {
            JwksError::UnknownKey => TokenError::UnknownKey,
            JwksError::Unavailable(_) | JwksError::ClientBuild(_) => TokenError::KeysUnavailable,
        })?;
        let claims = decode::<AccessClaims>(token, &key, &self.validation)
            .map_err(|error| TokenError::Rejected(rejection_reason(&error)))?
            .claims;
        self.check_claims(claims)
    }

    fn check_claims(&self, claims: AccessClaims) -> Result<VerifiedToken, TokenError> {
        let now = jsonwebtoken::get_current_timestamp();
        if claims.iat > now.saturating_add(LEEWAY_SECONDS) {
            return Err(TokenError::Claims("iat is in the future"));
        }
        if !bounded(&claims.sub) {
            return Err(TokenError::Claims("sub is empty or too long"));
        }
        // RFC 9068 section 2.2 makes both claims required in access tokens.
        if !claims.jti.as_deref().is_some_and(bounded) {
            return Err(TokenError::Claims("jti is missing, empty, or too long"));
        }
        if !claims.client_id.as_deref().is_some_and(bounded) {
            return Err(TokenError::Claims(
                "client_id is missing, empty, or too long",
            ));
        }
        let roles = claims.roles.unwrap_or_default();
        if roles.len() > MAX_ROLES || !roles.iter().all(|role| bounded(role)) {
            return Err(TokenError::Claims("roles are out of bounds"));
        }
        let granted: Vec<&str> = claims
            .scope
            .as_deref()
            .map(|scope| scope.split(' ').filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        if !self
            .required_scopes
            .iter()
            .all(|required| granted.contains(&required.as_str()))
        {
            return Err(TokenError::InsufficientScope);
        }
        Ok(VerifiedToken {
            subject_hash: subject_hash(&claims.sub),
            user: User {
                username: claims.sub,
                realm: Some(self.realm.clone()),
                roles,
                attributes: HashMap::new(),
            },
        })
    }
}

/// RFC 9068 section 4: `at+jwt`, or the full media type, any case.
///
/// - valid: `at+jwt`, `application/at+jwt`, `AT+JWT`
/// - invalid: `JWT`, `at+jwt; x=1`
fn is_access_token_type(typ: &str) -> bool {
    typ.eq_ignore_ascii_case("at+jwt") || typ.eq_ignore_ascii_case("application/at+jwt")
}

fn bounded(value: &str) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= MAX_CLAIM_CHARS
        && !value.chars().any(char::is_control)
}

/// Short pseudonym of a subject for correlating debug logs.
///
/// `DefaultHasher::new()` uses fixed keys, so a subject maps to the same
/// value across requests and restarts of one server build. The standard
/// library does not promise the algorithm, so the value may change with
/// the Rust toolchain; do not store it or compare it across builds. It is
/// not a security boundary; subjects are already pseudonymous.
fn subject_hash(sub: &str) -> String {
    let mut hasher = DefaultHasher::new();
    sub.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Name the failed check without echoing any token content.
fn rejection_reason(error: &jsonwebtoken::errors::Error) -> &'static str {
    use jsonwebtoken::errors::ErrorKind;
    match error.kind() {
        ErrorKind::InvalidSignature => "bad signature",
        ErrorKind::ExpiredSignature => "expired",
        ErrorKind::ImmatureSignature => "not yet valid",
        ErrorKind::InvalidIssuer => "wrong issuer",
        ErrorKind::InvalidAudience => "wrong audience",
        ErrorKind::MissingRequiredClaim(_) => "missing required claim",
        ErrorKind::InvalidAlgorithm => "algorithm mismatch",
        _ => "verification failed",
    }
}

#[cfg(test)]
mod tests {
    use super::{bounded, is_access_token_type, subject_hash};

    #[test]
    fn access_token_type_accepts_rfc_9068_forms_only() {
        for typ in ["at+jwt", "AT+JWT", "application/at+jwt"] {
            assert!(is_access_token_type(typ), "{typ}");
        }
        for typ in ["JWT", "jwt", "at+jwt; x=1", ""] {
            assert!(!is_access_token_type(typ), "{typ}");
        }
    }

    #[test]
    fn claim_values_are_bounded() {
        assert!(bounded("aviso-mcp"));
        assert!(!bounded(" "));
        assert!(!bounded("role\nwith-newline"));
        assert!(!bounded(&"r".repeat(257)));
    }

    #[test]
    fn subject_hash_is_deterministic_and_opaque() {
        assert_eq!(subject_hash("alice"), subject_hash("alice"));
        assert_ne!(subject_hash("alice"), subject_hash("bob"));
        assert!(!subject_hash("alice").contains("alice"));
    }
}
