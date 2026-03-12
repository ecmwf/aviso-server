use actix_web::http::header::HeaderValue;
use jsonwebtoken::{DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

pub mod client;
pub mod middleware;

/// Authenticated user extracted from JWT claims.
///
/// Contains the username, assigned roles, and optional attributes.
/// Roles are used for authorization checks against required permissions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    pub username: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub attributes: HashMap<String, String>,
}

impl User {
    /// Returns true if the user has any of the required roles.
    ///
    /// An empty `required_roles` list means any user is allowed.
    pub fn has_any_role(&self, required_roles: &[String]) -> bool {
        if required_roles.is_empty() {
            return true;
        }

        self.roles
            .iter()
            .any(|role| required_roles.iter().any(|required| required == role))
    }

    /// Returns true if the user has any of the admin roles.
    ///
    /// This is a convenience wrapper around `has_any_role`.
    pub fn is_admin(&self, admin_roles: &[String]) -> bool {
        self.has_any_role(admin_roles)
    }
}

/// JWT claims structure as returned by auth-o-tron.
///
/// Supports both direct username claim and subject fallback.
/// Audience validation is disabled by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    #[serde(default)]
    pub sub: Option<String>,
    #[serde(default)]
    pub iss: Option<String>,
    pub exp: usize,
    #[serde(default)]
    pub iat: Option<usize>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub attributes: HashMap<String, String>,
}

/// Error when extracting user from JWT claims.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum UserExtractionError {
    #[error("token is missing username and subject claims")]
    MissingUsername,
}

impl TryFrom<JwtClaims> for User {
    type Error = UserExtractionError;

    fn try_from(claims: JwtClaims) -> Result<Self, Self::Error> {
        let username = claims
            .username
            .or(claims.sub)
            .ok_or(UserExtractionError::MissingUsername)?;

        Ok(Self {
            username,
            roles: claims.roles,
            attributes: claims.attributes,
        })
    }
}

pub fn extract_bearer_token_from_str(value: &str) -> Option<&str> {
    let value = value.trim();
    let mut parts = value.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;

    if !scheme.eq_ignore_ascii_case("Bearer") || parts.next().is_some() {
        return None;
    }

    Some(token)
}

pub fn extract_bearer_token(header_value: &HeaderValue) -> Option<&str> {
    extract_bearer_token_from_str(header_value.to_str().ok()?)
}

/// Validates a JWT token using the provided secret.
///
/// Returns the decoded claims if the token is valid and not expired.
/// Audience validation is disabled to match auth-o-tron behavior.
pub fn validate_jwt(token: &str, secret: &str) -> Result<JwtClaims, jsonwebtoken::errors::Error> {
    let mut validation = Validation::default();
    validation.validate_aud = false;

    let token_data = decode::<JwtClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;
    Ok(token_data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use jsonwebtoken::{EncodingKey, Header, encode};

    fn claims(username: Option<&str>, sub: Option<&str>) -> JwtClaims {
        JwtClaims {
            sub: sub.map(ToString::to_string),
            iss: Some("auth-o-tron".to_string()),
            exp: (Utc::now().timestamp() + 3_600) as usize,
            iat: Some(Utc::now().timestamp() as usize),
            username: username.map(ToString::to_string),
            roles: vec!["reader".to_string(), "admin".to_string()],
            attributes: HashMap::from([(String::from("team"), String::from("ops"))]),
        }
    }

    fn token_for(claims: &JwtClaims, secret: &str) -> String {
        encode(
            &Header::default(),
            claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("token must encode")
    }

    #[test]
    fn extract_bearer_token_accepts_standard_header() {
        let header = HeaderValue::from_static("Bearer test-token-123");
        assert_eq!(extract_bearer_token(&header), Some("test-token-123"));
    }

    #[test]
    fn extract_bearer_token_rejects_non_bearer_headers() {
        let basic = HeaderValue::from_static("Basic dXNlcjpwYXNz");
        assert_eq!(extract_bearer_token(&basic), None);

        let missing = HeaderValue::from_static("test-token");
        assert_eq!(extract_bearer_token(&missing), None);
    }

    #[test]
    fn validate_jwt_returns_expected_claims() {
        let secret = "test-secret";
        let source_claims = claims(Some("test-user"), Some("subject"));
        let token = token_for(&source_claims, secret);

        let parsed_claims = validate_jwt(&token, secret).expect("token must validate");
        assert_eq!(parsed_claims.username.as_deref(), Some("test-user"));
        assert_eq!(parsed_claims.roles, vec!["reader", "admin"]);
    }

    #[test]
    fn user_conversion_falls_back_to_subject_claim() {
        let user = User::try_from(claims(None, Some("fallback-user"))).expect("user must parse");
        assert_eq!(user.username, "fallback-user");
    }

    #[test]
    fn user_conversion_rejects_missing_username_and_subject() {
        let result = User::try_from(claims(None, None));
        assert_eq!(result.unwrap_err(), UserExtractionError::MissingUsername);
    }

    #[test]
    fn has_any_role_returns_true_for_empty_required_roles() {
        let user = User {
            username: "reader".to_string(),
            roles: vec!["reader".to_string()],
            attributes: HashMap::new(),
        };
        assert!(user.has_any_role(&[]));
    }

    #[test]
    fn has_any_role_returns_true_when_user_has_matching_role() {
        let user = User {
            username: "reader".to_string(),
            roles: vec!["reader".to_string(), "ops".to_string()],
            attributes: HashMap::new(),
        };
        let required = vec!["admin".to_string(), "ops".to_string()];
        assert!(user.has_any_role(&required));
    }

    #[test]
    fn has_any_role_returns_false_when_no_roles_match() {
        let user = User {
            username: "reader".to_string(),
            roles: vec!["reader".to_string()],
            attributes: HashMap::new(),
        };
        let required = vec!["admin".to_string(), "operator".to_string()];
        assert!(!user.has_any_role(&required));
    }

    #[test]
    fn has_any_role_returns_false_for_empty_user_roles() {
        let user = User {
            username: "reader".to_string(),
            roles: Vec::new(),
            attributes: HashMap::new(),
        };
        let required = vec!["admin".to_string()];
        assert!(!user.has_any_role(&required));
    }

    #[test]
    fn validate_jwt_rejects_wrong_secret() {
        let claims = claims(Some("test-user"), Some("subject"));
        let token = token_for(&claims, "correct-secret");

        let error = validate_jwt(&token, "wrong-secret").expect_err("token must not validate");
        assert!(matches!(
            error.kind(),
            jsonwebtoken::errors::ErrorKind::InvalidSignature
        ));
    }

    #[test]
    fn validate_jwt_rejects_expired_token() {
        let claims = JwtClaims {
            sub: Some("subject".to_string()),
            iss: Some("auth-o-tron".to_string()),
            exp: (Utc::now().timestamp() - 3_600) as usize,
            iat: Some((Utc::now().timestamp() - 3_660) as usize),
            username: Some("test-user".to_string()),
            roles: vec!["reader".to_string()],
            attributes: HashMap::new(),
        };
        let token = token_for(&claims, "test-secret");

        let error = validate_jwt(&token, "test-secret").expect_err("expired token must not pass");
        assert!(matches!(
            error.kind(),
            jsonwebtoken::errors::ErrorKind::ExpiredSignature
        ));
    }
}
