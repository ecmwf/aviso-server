use aviso_ecpds::client::FetchOutcome;
use aviso_ecpds::config::{EcpdsConfig, PartialOutagePolicy};
use aviso_ecpds::{EcpdsChecker, EcpdsError};
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
            .result
            .is_ok(),
        "active CIP must be allowed in populated_user fixture"
    );
    assert!(
        checker
            .check_access("alice", &make_identifier("FOO"))
            .await
            .result
            .is_ok(),
        "active FOO must be allowed in populated_user fixture"
    );
    assert!(
        matches!(
            checker
                .check_access("alice", &make_identifier("BAR"))
                .await
                .result,
            Err(EcpdsError::AccessDenied { .. })
        ),
        "BAR is in the destinationList but with active=false; the contract \
         requires inactive destinations to deny access"
    );
    assert!(
        matches!(
            checker
                .check_access("alice", &make_identifier("UNKNOWN"))
                .await
                .result,
            Err(EcpdsError::AccessDenied { .. })
        ),
        "UNKNOWN must be denied (not present in any record)"
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
            .await
            .result,
        Err(EcpdsError::AccessDenied { .. })
    ));
}

#[tokio::test]
async fn success_field_not_yes_surfaces_as_service_unavailable() {
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
    let access = checker.check_access("alice", &make_identifier("CIP")).await;
    assert!(
        matches!(
            access.result,
            Err(EcpdsError::ServiceUnavailable {
                fetch_outcome: FetchOutcome::InvalidResponse,
            })
        ),
        "ECPDS responses with success != \"yes\" indicate a server-side failure \
         and must surface as ServiceUnavailable / InvalidResponse, not as a \
         silent allow/deny based on whatever destinationList happened to contain. \
         Treating them as a normal empty list would hide the upstream outage from \
         aviso_ecpds_fetch_total. Got: {:?}",
        access.result
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
            .result
            .is_ok(),
        "records with the target_field present must still produce allows"
    );
    assert!(
        checker
            .check_access("alice", &make_identifier("FOO"))
            .await
            .result
            .is_ok(),
        "records with the target_field present must still produce allows"
    );
    assert!(
        matches!(
            checker
                .check_access("alice", &make_identifier("no-name-field"))
                .await
                .result,
            Err(EcpdsError::AccessDenied { .. })
        ),
        "records missing target_field must be silently skipped, not surface as a destination"
    );
}
