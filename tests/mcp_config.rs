// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The `mcp` block: loading, environment overrides, startup validation, and
//! route registration.

use aviso_server::configuration::Settings;
use aviso_server::startup::Application;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn base() -> Value {
    json!({
        "application": {"host": "127.0.0.1", "port": 0, "static_files_path": "/tmp"},
        "notification_backend": {"kind": "in_memory"}
    })
}

fn load(file: Value, env: &[(&str, &str)]) -> Settings {
    let source = env
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect();
    config::Config::builder()
        .add_source(config::File::from_str(
            &file.to_string(),
            config::FileFormat::Json,
        ))
        .add_source(
            config::Environment::with_prefix("AVISOSERVER")
                .prefix_separator("_")
                .separator("__")
                .source(Some(source)),
        )
        .build()
        .unwrap()
        .try_deserialize()
        .unwrap()
}

#[test]
fn environment_overrides_the_mcp_block() {
    let settings = load(
        base(),
        &[
            ("AVISOSERVER_MCP__ENABLED", "true"),
            ("AVISOSERVER_MCP__MAX_RESULTS", "50"),
            (
                "AVISOSERVER_MCP__OAUTH__ISSUER",
                "https://auth.example.int/",
            ),
            ("AVISOSERVER_MCP__IDENTITY__REALM", "demo"),
        ],
    );
    assert!(settings.mcp.enabled);
    assert_eq!(settings.mcp.max_results, 50);
    assert_eq!(settings.mcp.oauth.issuer, "https://auth.example.int/");
    assert_eq!(settings.mcp.oauth.required_scopes, vec!["aviso:read"]);
    assert_eq!(settings.mcp.identity.realm, "demo");
}

#[test]
fn missing_block_is_disabled() {
    let settings = load(base(), &[]);
    assert!(!settings.mcp.enabled);
}

#[tokio::test]
async fn enabled_block_with_missing_fields_fails_startup() {
    let settings = load(base(), &[("AVISOSERVER_MCP__ENABLED", "true")]);
    let result = Application::build(settings, CancellationToken::new()).await;
    let Err(error) = result else {
        panic!("startup must fail");
    };
    assert!(error.to_string().contains("mcp.resource_url"), "{error}");
}

#[tokio::test]
async fn enabled_block_with_plain_http_issuer_fails_startup() {
    let mut file = base();
    file["mcp"] = json!({
        "enabled": true,
        "resource_url": "https://aviso.example.int/mcp",
        "oauth": {
            "issuer": "http://auth.example.int/",
            "jwks_uri": "https://auth.example.int/jwks"
        }
    });
    let result = Application::build(load(file, &[]), CancellationToken::new()).await;
    let Err(error) = result else {
        panic!("startup must fail");
    };
    assert!(error.to_string().contains("mcp.oauth.issuer"), "{error}");
}

async fn status(port: u16, method: reqwest::Method, path: &str) -> u16 {
    reqwest::Client::new()
        .request(method, format!("http://127.0.0.1:{port}{path}"))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn statuses(settings: Settings) -> [u16; 3] {
    let shutdown = CancellationToken::new();
    let application = Application::build(settings, shutdown.clone())
        .await
        .unwrap();
    let port = application.port();
    let server = tokio::spawn(application.run_until_stopped());
    let result = [
        status(port, reqwest::Method::POST, "/mcp").await,
        status(port, reqwest::Method::GET, "/mcp").await,
        status(
            port,
            reqwest::Method::GET,
            "/.well-known/oauth-protected-resource/mcp",
        )
        .await,
    ];
    shutdown.cancel();
    server.await.unwrap().unwrap();
    result
}

#[tokio::test]
async fn disabled_block_registers_no_routes() {
    assert_eq!(statuses(load(base(), &[])).await, [404, 404, 404]);
}

#[tokio::test]
async fn enabled_block_registers_mcp_and_metadata_routes() {
    let mut file = base();
    file["mcp"] = json!({
        "enabled": true,
        "resource_url": "https://aviso.example.int/mcp",
        "oauth": {
            "issuer": "https://auth.example.int/",
            "jwks_uri": "https://auth.example.int/jwks"
        }
    });
    assert_eq!(statuses(load(file, &[])).await, [401, 405, 200]);
}
