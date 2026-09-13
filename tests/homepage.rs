use aviso_server::configuration::{ApplicationSettings, HomepageSettings, Settings};
use aviso_server::startup::Application;
use tokio_util::sync::CancellationToken;

#[test]
fn homepage_defaults_and_partial_configuration() {
    let application: ApplicationSettings =
        serde_json::from_value(serde_json::json!({"host": "127.0.0.1", "port": 0})).unwrap();
    let defaults = HomepageSettings::default();
    assert_eq!(application.homepage, defaults);
    assert_eq!(
        defaults.client_documentation_url,
        "https://sites.ecmwf.int/docs/aviso-client/main/"
    );
    assert_eq!(
        defaults.client_repository_url,
        "https://github.com/ecmwf/aviso-client"
    );
    assert_eq!(
        defaults.server_documentation_url,
        "https://sites.ecmwf.int/docs/aviso-server/main/"
    );
    assert_eq!(
        defaults.server_repository_url,
        "https://github.com/ecmwf/aviso-server"
    );
    defaults.validate().unwrap();
    let partial: HomepageSettings = serde_json::from_value(serde_json::json!({
        "client_documentation_url": "http://localhost/docs"
    }))
    .unwrap();
    assert_eq!(
        partial,
        HomepageSettings {
            client_documentation_url: "http://localhost/docs".into(),
            ..defaults
        }
    );
    partial.validate().unwrap();
}

#[actix_web::test]
async fn homepage_rejects_unsafe_links_before_boot() {
    for field in [
        "client_documentation_url",
        "client_repository_url",
        "server_documentation_url",
        "server_repository_url",
    ] {
        for value in [
            "",
            "/docs",
            "javascript:alert(1)",
            "data:text/html,hello",
            "https://",
            "https://user:password@example.org",
            "https://user@example.org",
        ] {
            let mut settings = test_settings();
            let mut links = serde_json::to_value(&settings.application.homepage).unwrap();
            links[field] = value.into();
            settings.application.homepage = serde_json::from_value(links).unwrap();
            let error = Application::build(settings, CancellationToken::new())
                .await
                .err()
                .unwrap();
            assert!(
                error
                    .to_string()
                    .contains(&format!("application.homepage.{field}"))
            );
            assert!(!error.to_string().contains("password@example.org"));
        }
    }
}

fn test_settings() -> Settings {
    serde_json::from_value(serde_json::json!({
        "application": {"host": "127.0.0.1", "port": 0,
            "static_files_path": concat!(env!("CARGO_MANIFEST_DIR"), "/src/static")},
        "notification_backend": {"kind": "in_memory"}
    }))
    .unwrap()
}

#[actix_web::test]
async fn homepage_http_uses_app_settings_and_environment_overrides() {
    let mut settings = test_settings();
    // Inject an environment source without mutating the test process environment.
    let source = [
        (
            "CLIENT_DOCUMENTATION_URL",
            "https://client.example/docs?a=1&b=\"quoted\"<'test'>",
        ),
        ("CLIENT_REPOSITORY_URL", "https://client.example/repo"),
        ("SERVER_DOCUMENTATION_URL", "https://server.example/docs"),
        ("SERVER_REPOSITORY_URL", "https://server.example/repo"),
    ]
    .into_iter()
    .map(|(key, value)| {
        (
            format!("AVISOSERVER_APPLICATION__HOMEPAGE__{key}"),
            value.to_string(),
        )
    })
    .collect();
    settings = config::Config::builder()
        .add_source(config::File::from_str(
            &serde_json::to_string(&settings).unwrap(),
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
        .unwrap();
    let shutdown = CancellationToken::new();
    let application = Application::build(settings, shutdown.clone())
        .await
        .unwrap();
    let port = application.port();
    let server = tokio::spawn(application.run_until_stopped());
    let response = reqwest::get(format!("http://127.0.0.1:{port}/"))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    let html = response.text().await.unwrap();
    assert!(html.contains(
        "href=\"https://client.example/docs?a=1&amp;b=&quot;quoted&quot;&lt;&#39;test&#39;&gt;\""
    ));
    for url in [
        "https://client.example/repo",
        "https://server.example/docs",
        "https://server.example/repo",
    ] {
        assert!(html.contains(&format!("href=\"{url}\"")));
    }
    assert!(html.find("id=\"use-aviso\"").unwrap() < html.find("id=\"operate-server\"").unwrap());
    assert_eq!(html.matches("class=\"btn btn-secondary\"").count(), 5);
    assert!(!html.contains("btn-primary"));
    for (id, count) in [("use-aviso", 2), ("operate-server", 3)] {
        let section = html
            .split_once(&format!("<section id=\"{id}\">"))
            .unwrap()
            .1
            .split_once("</section>")
            .unwrap()
            .0;
        assert_eq!(
            section.matches("class=\"btn btn-secondary\"").count(),
            count
        );
    }
    assert!(html.contains("href=\"swagger-ui/\""));
    assert!(!html.contains("{{"));
    shutdown.cancel();
    server.await.unwrap().unwrap();
}
