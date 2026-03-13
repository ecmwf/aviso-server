//! OpenAPI documentation configuration for the Aviso notification service

use crate::routes::admin;
use crate::routes::health_check;
use crate::routes::home;
use crate::routes::notify;
use crate::routes::replay;
use crate::routes::schema;
use crate::routes::watch;
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
use utoipa::{Modify, OpenApi};

/// Registers Bearer (JWT) and Basic authentication schemes in the OpenAPI spec.
/// Swagger UI renders these as an "Authorize" button so users can test protected endpoints.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_default();
        components.add_security_scheme(
            "bearer_jwt",
            SecurityScheme::Http({
                let mut http = Http::new(HttpAuthScheme::Bearer);
                http.bearer_format = Some("JWT".to_string());
                http
            }),
        );
        components.add_security_scheme(
            "basic",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Basic)),
        );
    }
}

/// OpenAPI specification for the Aviso notification service
#[derive(OpenApi)]
#[openapi(
    modifiers(&SecurityAddon),
    paths(
        health_check::health_check,
        home::homepage,
        schema::get_notification_schema,
        schema::get_event_schema,
        notify::notify,
        watch::watch,
        replay::replay,
        admin::wipe_stream,
        admin::wipe_all,
        admin::delete_notification,
    ),
    components(
        schemas(
            // Health endpoints
            health_check::HealthResponse,

            // Core request/response types
            crate::types::request::NotificationRequest,
            crate::types::NotificationResponse,

            crate::configuration::ApiEventSchema,
            crate::configuration::PayloadConfig,

            // Admin endpoints
            admin::WipeStreamRequest,
            admin::WipeResponse,
            admin::DeleteNotificationResponse,
        )
    ),
    info(
        title = "Aviso Notification Service API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Real-time notification service developed by ECMWF",
        contact(
            name = "Aviso API Support",
            url = "https://www.ecmwf.int/en/about/contact-us"
        ),
        license(
            name = "Apache 2.0",
            url = "https://www.apache.org/licenses/LICENSE-2.0"
        )
    ),
    tags(
        (name = "health", description = "Service health and status monitoring"),
        (name = "general", description = "General application endpoints"),
        (name = "schema", description = "API schema discovery and validation rules"),
        (name = "notification", description = "Send notifications to the system"),
        (name = "streaming", description = "Real-time streaming and replay functionality"),
        (name = "admin", description = "⚠️ Administrative operations - use with extreme caution")
    )
)]
pub struct ApiDoc;
