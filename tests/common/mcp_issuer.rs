// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Test issuer for MCP tokens: fixed RSA keys, a token minter, and a local
//! JWKS server whose key set can be swapped and whose hits are counted.
//! The server can answer after a fixed delay to model a slow issuer.
//!
//! The PKCS#1 DER files under `tests/fixtures/mcp/` are throwaway keys generated
//! for these tests only.

use actix_web::dev::ServerHandle;
use actix_web::{App, HttpResponse, HttpServer, web};
use aviso_server::configuration::McpSettings;
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const ISSUER: &str = "https://auth.example.int/";
pub const RESOURCE: &str = "https://aviso.example.int/mcp";
pub const KID_A: &str = "key-a";
pub const KID_B: &str = "key-b";

pub fn key_a() -> EncodingKey {
    EncodingKey::from_rsa_der(include_bytes!("../fixtures/mcp/signing_key_a.der"))
}

pub fn key_b() -> EncodingKey {
    EncodingKey::from_rsa_der(include_bytes!("../fixtures/mcp/signing_key_b.der"))
}

/// Public JWK for `key` under `kid`.
pub fn jwk(key: &EncodingKey, kid: &str) -> Jwk {
    let mut jwk = Jwk::from_encoding_key(key, Algorithm::RS256).unwrap();
    jwk.common.key_id = Some(kid.to_string());
    jwk
}

/// Claims Auth-O-Tron issues for a demo user, valid for an hour.
pub fn good_claims() -> Value {
    let now = chrono::Utc::now().timestamp();
    json!({
        "iss": ISSUER,
        "sub": "f00dbabe",
        "aud": RESOURCE,
        "iat": now,
        "exp": now + 3600,
        "jti": "test",
        "client_id": "mcp-inspector",
        "scope": "aviso:read",
        "roles": ["aviso-mcp"]
    })
}

/// Sign `claims` with RS256, `typ: at+jwt`, and `kid`.
pub fn mint(claims: &Value, key: &EncodingKey, kid: &str) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.typ = Some("at+jwt".to_string());
    header.kid = Some(kid.to_string());
    encode(&header, claims, key).unwrap()
}

/// A token for [`good_claims`] signed with key A.
pub fn good_token() -> String {
    mint(&good_claims(), &key_a(), KID_A)
}

/// Enabled MCP settings that trust the given JWKS URI.
pub fn settings(jwks_uri: &str) -> McpSettings {
    serde_json::from_value(json!({
        "enabled": true,
        "resource_url": RESOURCE,
        "allowed_origins": ["http://localhost:6274"],
        "max_results": 5,
        "oauth": {"issuer": ISSUER, "jwks_uri": jwks_uri}
    }))
    .unwrap()
}

/// Local JWKS endpoint at `/jwks`.
pub struct JwksServer {
    pub uri: String,
    keys: Arc<Mutex<JwkSet>>,
    hits: Arc<AtomicUsize>,
    handle: ServerHandle,
}

impl JwksServer {
    pub fn start(keys: Vec<Jwk>) -> Self {
        Self::start_with_delay(keys, Duration::ZERO)
    }

    /// Like [`JwksServer::start`], but every response waits `delay`.
    pub fn start_with_delay(keys: Vec<Jwk>, delay: Duration) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let uri = format!("http://{}/jwks", listener.local_addr().unwrap());
        let keys = Arc::new(Mutex::new(JwkSet { keys }));
        let hits = Arc::new(AtomicUsize::new(0));
        let (served, counted) = (keys.clone(), hits.clone());
        let server = HttpServer::new(move || {
            let (served, counted) = (served.clone(), counted.clone());
            App::new().route(
                "/jwks",
                web::get().to(move || {
                    counted.fetch_add(1, Ordering::SeqCst);
                    let body = serde_json::to_string(&*served.lock().unwrap()).unwrap();
                    async move {
                        tokio::time::sleep(delay).await;
                        HttpResponse::Ok()
                            .content_type("application/json")
                            .body(body)
                    }
                }),
            )
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        tokio::spawn(server);
        Self {
            uri,
            keys,
            hits,
            handle,
        }
    }

    pub fn set_keys(&self, keys: Vec<Jwk>) {
        *self.keys.lock().unwrap() = JwkSet { keys };
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub async fn stop(self) {
        self.handle.stop(true).await;
    }
}
