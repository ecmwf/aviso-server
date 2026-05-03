use aviso_ecpds::{EcpdsChecker, EcpdsError};
use aviso_ecpds::config::{EcpdsConfig, PartialOutagePolicy};
use std::collections::HashMap;

fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{name}.json"))
        .unwrap_or_else(|e| panic!("fixture {name}.json must exist: {e}"))
}

fn make_config(servers: Vec<String>) -> EcpdsConfig {
    EcpdsConfig {
        username: "u".into(),
        password: "p".into(),
        target_field: "name".into(),
        match_key: "destination".into(),
        cache_ttl_seconds: 300,
        max_entries: 1000,
        request_timeout_seconds: 30,
        connect_timeout_seconds: 5,
        partial_outage_policy: PartialOutagePolicy::AnySuccess,
        servers,
    }
}

fn make_identifier(destination: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("destination".into(), destination.into());
    m
}

#[tokio::test]
async fn populated_user_response_yields_known_destinations() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/ecpds/v1/destination/list")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(read_fixture("populated_user"))
        .create_async()
        .await;

    let checker = EcpdsChecker::new(&make_config(vec![server.url()])).unwrap();
    assert!(
        checker
            .check_access("alice", &make_identifier("CIP"))
            .await
            .is_ok(),
        "CIP must be allowed in populated_user fixture"
    );
    assert!(
        matches!(
            checker
                .check_access("alice", &make_identifier("UNKNOWN"))
                .await,
            Err(EcpdsError::AccessDenied(_))
        ),
        "UNKNOWN must be denied"
    );
}

#[tokio::test]
async fn empty_user_response_denies_all_destinations() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/ecpds/v1/destination/list")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(read_fixture("empty_user"))
        .create_async()
        .await;

    let checker = EcpdsChecker::new(&make_config(vec![server.url()])).unwrap();
    assert!(matches!(
        checker
            .check_access("alice", &make_identifier("CIP"))
            .await,
        Err(EcpdsError::AccessDenied(_))
    ));
}

#[tokio::test]
async fn success_no_fixture_currently_treated_as_empty_list() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/ecpds/v1/destination/list")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(read_fixture("success_no"))
        .create_async()
        .await;

    let checker = EcpdsChecker::new(&make_config(vec![server.url()])).unwrap();
    let result = checker.check_access("alice", &make_identifier("CIP")).await;
    assert!(
        matches!(result, Err(EcpdsError::AccessDenied(_))),
        "Current ECPDS contract assumption: success != \"yes\" responses are parsed \
         as their literal destinationList. If this changes (e.g. ECPDS team confirms \
         that success: \"no\" should be a server-side failure rather than an empty \
         allow-list), update the parser AND this test together."
    );
}

#[tokio::test]
async fn record_missing_target_field_is_silently_skipped() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/ecpds/v1/destination/list")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(read_fixture("record_missing_target_field"))
        .create_async()
        .await;

    let checker = EcpdsChecker::new(&make_config(vec![server.url()])).unwrap();
    assert!(
        checker
            .check_access("alice", &make_identifier("CIP"))
            .await
            .is_ok(),
        "records with the target_field present must still produce allows"
    );
    assert!(
        checker
            .check_access("alice", &make_identifier("FOO"))
            .await
            .is_ok(),
        "records with the target_field present must still produce allows"
    );
    assert!(
        matches!(
            checker
                .check_access("alice", &make_identifier("no-name-field"))
                .await,
            Err(EcpdsError::AccessDenied(_))
        ),
        "records missing target_field must be silently skipped, not surface as a destination"
    );
}
