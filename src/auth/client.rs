use crate::auth::extract_bearer_token_from_str;
use reqwest::{Client, StatusCode, header};
use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;

#[derive(Clone)]
pub struct AuthClient {
    http_client: Client,
    authenticate_url: String,
}

#[derive(Debug, Error)]
pub enum AuthClientError {
    #[error("failed to build auth client: {0}")]
    ClientBuild(#[source] reqwest::Error),
    #[error("auth service request failed: {0}")]
    Transport(#[source] reqwest::Error),
    #[error("auth service rejected credentials")]
    Unauthorized,
    #[error("auth service denied access")]
    Forbidden,
    #[error("auth service returned status {0}")]
    UnexpectedStatus(u16),
    #[error("auth service response did not contain a JWT")]
    MissingToken,
}

#[derive(Deserialize)]
struct AuthenticateResponse {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    jwt: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
}

impl AuthClient {
    pub fn new(base_url: &str, timeout_ms: u64) -> Result<Self, AuthClientError> {
        let http_client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(AuthClientError::ClientBuild)?;

        Ok(Self {
            http_client,
            authenticate_url: format!("{}/authenticate", base_url.trim_end_matches('/')),
        })
    }

    pub async fn authenticate(&self, authorization_value: &str) -> Result<String, AuthClientError> {
        let response = self
            .http_client
            .get(&self.authenticate_url)
            .header(header::AUTHORIZATION, authorization_value)
            .send()
            .await
            .map_err(AuthClientError::Transport)?;

        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            return Err(AuthClientError::Unauthorized);
        }
        if status == StatusCode::FORBIDDEN {
            return Err(AuthClientError::Forbidden);
        }
        if !status.is_success() {
            return Err(AuthClientError::UnexpectedStatus(status.as_u16()));
        }

        if let Some(jwt) = response
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_jwt_from_authorization_header)
        {
            return Ok(jwt);
        }

        let body = response.text().await.map_err(AuthClientError::Transport)?;
        parse_jwt_from_response(&body).ok_or(AuthClientError::MissingToken)
    }
}

fn parse_jwt_from_authorization_header(value: &str) -> Option<String> {
    let token = extract_bearer_token_from_str(value)?;
    if !is_jwt_like(token) {
        return None;
    }
    Some(token.to_string())
}

fn parse_jwt_from_response(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(parsed) = serde_json::from_str::<AuthenticateResponse>(trimmed)
        && let Some(token) = parsed
            .token
            .or(parsed.jwt)
            .or(parsed.access_token)
            .map(|value| value.trim().to_string())
            .filter(|value| is_jwt_like(value))
    {
        return Some(token);
    }

    if let Ok(raw_token) = serde_json::from_str::<String>(trimmed) {
        let raw_token = raw_token.trim();
        if is_jwt_like(raw_token) {
            return Some(raw_token.to_string());
        }
    }

    if is_jwt_like(trimmed) {
        return Some(trimmed.to_string());
    }

    None
}

/// Lightweight JWT-shape check used for auth-o-tron response parsing.
///
/// This is intentionally structural only (three base64url-like segments), not
/// cryptographic verification. Real validation happens in `validate_jwt`.
pub fn is_jwt_like(value: &str) -> bool {
    let mut parts = value.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(first), Some(second), Some(third), None)
            if is_base64url_segment(first)
                && is_base64url_segment(second)
                && is_base64url_segment(third)
    )
}

fn is_base64url_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::{parse_jwt_from_authorization_header, parse_jwt_from_response};

    const JWT: &str = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.signature";

    #[test]
    fn parse_jwt_from_json_object() {
        let body = format!(r#"{{"token":"{JWT}"}}"#);
        assert_eq!(parse_jwt_from_response(&body), Some(JWT.to_string()));
    }

    #[test]
    fn parse_jwt_from_json_string() {
        let body = format!(r#""{JWT}""#);
        assert_eq!(parse_jwt_from_response(&body), Some(JWT.to_string()));
    }

    #[test]
    fn parse_jwt_from_raw_body() {
        assert_eq!(parse_jwt_from_response(JWT), Some(JWT.to_string()));
    }

    #[test]
    fn parse_jwt_rejects_non_jwt_responses() {
        assert_eq!(parse_jwt_from_response("{}"), None);
        assert_eq!(parse_jwt_from_response("ok"), None);
        assert_eq!(parse_jwt_from_response(""), None);
        assert_eq!(parse_jwt_from_response("a.b+c"), None);
    }

    #[test]
    fn parse_jwt_from_authorization_header_accepts_bearer_token() {
        assert_eq!(
            parse_jwt_from_authorization_header(&format!("Bearer {JWT}")),
            Some(JWT.to_string())
        );
    }

    #[test]
    fn parse_jwt_from_authorization_header_rejects_non_bearer_or_invalid_tokens() {
        assert_eq!(parse_jwt_from_authorization_header("Basic abc"), None);
        assert_eq!(
            parse_jwt_from_authorization_header("Bearer not-a-jwt"),
            None
        );
        assert_eq!(parse_jwt_from_authorization_header("Bearer a..b"), None);
        assert_eq!(parse_jwt_from_authorization_header("Bearer a.b.c.d"), None);
        assert_eq!(parse_jwt_from_authorization_header("Bearer a.b=.c"), None);
    }
}
