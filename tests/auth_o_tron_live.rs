// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use aviso_server::auth::{User, client::AuthClient, validate_jwt};

// Uses only the bundled local fixture, never real credentials or identity providers.
#[tokio::test]
async fn bundled_auth_o_tron_users_produce_compatible_tokens() {
    let Ok(url) = std::env::var("AVISO_TEST_AUTH_O_TRON_URL") else {
        return;
    };
    let client = AuthClient::new(&url, 5000).unwrap();
    for role in ["admin", "reader", "producer"] {
        let request = reqwest::Client::new()
            .get(&url)
            .basic_auth(format!("{role}-user"), Some(format!("{role}-pass")))
            .build()
            .unwrap();
        let token = client
            .authenticate(
                request.headers()[reqwest::header::AUTHORIZATION]
                    .to_str()
                    .unwrap(),
            )
            .await
            .unwrap();
        let claims = validate_jwt(&token, "your-shared-secret").unwrap();
        let user = User::try_from(claims).unwrap();
        assert_eq!(user.username, format!("{role}-user"));
        assert_eq!(user.realm.as_deref(), Some("localrealm"));
        assert!(user.roles.iter().any(|value| value == role));
    }
    let request = reqwest::Client::new()
        .get(&url)
        .basic_auth("admin-user", Some("wrong-password"))
        .build()
        .unwrap();
    assert!(matches!(
        client
            .authenticate(
                request.headers()[reqwest::header::AUTHORIZATION]
                    .to_str()
                    .unwrap()
            )
            .await,
        Err(aviso_server::auth::client::AuthClientError::Unauthorized)
    ));
}
