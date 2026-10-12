// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! MCP access token verification against a local issuer key set.

#[path = "common/mcp_issuer.rs"]
mod mcp_issuer;

use aviso_server::mcp::auth::{TokenError, TokenVerifier};
use aviso_server::mcp::jwks::{JwksCache, JwksTiming};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use mcp_issuer::*;
use serde_json::json;
use std::time::Duration;

fn verifier(server: &JwksServer, min_refetch: Duration) -> TokenVerifier {
    let timing = JwksTiming {
        ttl: Duration::from_secs(300),
        min_refetch_interval: min_refetch,
        fetch_timeout: Duration::from_secs(2),
    };
    let jwks = JwksCache::new(&server.uri, timing).unwrap();
    TokenVerifier::new(jwks, &settings(&server.uri))
}

async fn check(claims: serde_json::Value) -> Result<(), TokenError> {
    let server = JwksServer::start(vec![jwk(&key_a(), KID_A)]);
    let result = verifier(&server, Duration::from_secs(60))
        .verify(&mint(&claims, &key_a(), KID_A))
        .await
        .map(|_| ());
    server.stop().await;
    result
}

fn with(key: &str, value: serde_json::Value) -> serde_json::Value {
    let mut claims = good_claims();
    claims[key] = value;
    claims
}

#[tokio::test]
async fn valid_token_maps_to_a_non_admin_mcp_user() {
    let server = JwksServer::start(vec![jwk(&key_a(), KID_A)]);
    let verified = verifier(&server, Duration::from_secs(60))
        .verify(&good_token())
        .await
        .unwrap();
    assert_eq!(verified.user.username, "f00dbabe");
    assert_eq!(verified.user.realm.as_deref(), Some("mcp"));
    assert_eq!(verified.user.roles, vec!["aviso-mcp"]);
    assert!(!verified.subject_hash.contains("f00dbabe"));
    server.stop().await;
}

#[tokio::test]
async fn audience_may_be_an_array_containing_the_resource() {
    let claims = with("aud", json!(["https://other.example.int", RESOURCE]));
    assert!(check(claims).await.is_ok());
}

#[tokio::test]
async fn wrong_issuer_audience_or_expiry_is_rejected() {
    let cases = [
        with("iss", json!("https://auth.example.int")),
        with("aud", json!("https://aviso.example.int/mcp/")),
        with("aud", json!(["https://other.example.int"])),
        with("exp", json!(chrono::Utc::now().timestamp() - 120)),
        with("nbf", json!(chrono::Utc::now().timestamp() + 600)),
    ];
    for claims in cases {
        let result = check(claims.clone()).await;
        assert!(
            matches!(result, Err(TokenError::Rejected(_))),
            "{claims}: {result:?}"
        );
    }
}

#[tokio::test]
async fn future_iat_and_missing_claims_are_rejected() {
    let future = with("iat", json!(chrono::Utc::now().timestamp() + 600));
    assert!(matches!(check(future).await, Err(TokenError::Claims(_))));
    let mut no_sub = good_claims();
    no_sub.as_object_mut().unwrap().remove("sub");
    assert!(check(no_sub).await.is_err());
    let too_many_roles = with("roles", json!(vec!["r"; 65]));
    assert!(matches!(
        check(too_many_roles).await,
        Err(TokenError::Claims(_))
    ));
}

#[tokio::test]
async fn missing_scope_is_insufficient_scope() {
    for scope in [json!("aviso:write"), json!(""), json!("aviso:readx")] {
        let result = check(with("scope", scope)).await;
        assert!(matches!(result, Err(TokenError::InsufficientScope)));
    }
    let mut claims = good_claims();
    claims.as_object_mut().unwrap().remove("scope");
    assert!(matches!(
        check(claims).await,
        Err(TokenError::InsufficientScope)
    ));
    assert!(
        check(with("scope", json!("openid aviso:read")))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn header_type_and_algorithm_are_enforced() {
    let server = JwksServer::start(vec![jwk(&key_a(), KID_A)]);
    let verifier = verifier(&server, Duration::from_secs(60));

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID_A.to_string());
    header.typ = Some("JWT".to_string());
    let plain = encode(&header, &good_claims(), &key_a()).unwrap();
    assert!(matches!(
        verifier.verify(&plain).await,
        Err(TokenError::Type)
    ));

    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some(KID_A.to_string());
    header.typ = Some("at+jwt".to_string());
    let hmac = encode(&header, &good_claims(), &EncodingKey::from_secret(b"s")).unwrap();
    assert!(matches!(
        verifier.verify(&hmac).await,
        Err(TokenError::Algorithm)
    ));

    let mut header = Header::new(Algorithm::RS256);
    header.typ = Some("at+jwt".to_string());
    let no_kid = encode(&header, &good_claims(), &key_a()).unwrap();
    assert!(matches!(
        verifier.verify(&no_kid).await,
        Err(TokenError::MissingKeyId)
    ));

    assert!(matches!(
        verifier.verify("not-a-jwt").await,
        Err(TokenError::Malformed)
    ));
    server.stop().await;
}

#[tokio::test]
async fn signature_from_another_key_under_a_known_kid_fails() {
    let forged = mint(&good_claims(), &key_b(), KID_A);
    let server = JwksServer::start(vec![jwk(&key_a(), KID_A)]);
    let result = verifier(&server, Duration::from_secs(60))
        .verify(&forged)
        .await;
    assert!(matches!(result, Err(TokenError::Rejected(_))));
    server.stop().await;
}

#[tokio::test]
async fn unknown_kid_refetches_at_most_once_per_interval() {
    let server = JwksServer::start(vec![jwk(&key_a(), KID_A)]);
    let verifier = verifier(&server, Duration::from_secs(60));
    assert!(verifier.verify(&good_token()).await.is_ok());
    assert_eq!(server.hits(), 1);

    let rotated = mint(&good_claims(), &key_b(), KID_B);
    for _ in 0..3 {
        assert!(matches!(
            verifier.verify(&rotated).await,
            Err(TokenError::UnknownKey)
        ));
    }
    assert_eq!(server.hits(), 1, "refetch must wait for the interval");
    server.stop().await;
}

#[tokio::test]
async fn unknown_kid_picks_up_a_rotated_key() {
    let server = JwksServer::start(vec![jwk(&key_a(), KID_A)]);
    let verifier = verifier(&server, Duration::ZERO);
    assert!(verifier.verify(&good_token()).await.is_ok());
    server.set_keys(vec![jwk(&key_a(), KID_A), jwk(&key_b(), KID_B)]);
    let rotated = mint(&good_claims(), &key_b(), KID_B);
    assert!(verifier.verify(&rotated).await.is_ok());
    assert_eq!(server.hits(), 2);
    assert!(verifier.verify(&rotated).await.is_ok());
    assert_eq!(server.hits(), 2, "known keys are served from cache");
    server.stop().await;
}

#[tokio::test]
async fn unreachable_key_set_is_a_server_side_failure() {
    let server = JwksServer::start(vec![]);
    let uri = server.uri.clone();
    server.stop().await;
    let jwks = JwksCache::new(&uri, JwksTiming::with_ttl(Duration::from_secs(300))).unwrap();
    let verifier = TokenVerifier::new(jwks, &settings(&uri));
    let error = verifier.verify(&good_token()).await.unwrap_err();
    assert!(matches!(error, TokenError::KeysUnavailable));
    assert_eq!(error.oauth_error(), None);
}
