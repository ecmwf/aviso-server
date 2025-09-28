//! OpenAPI documentation configuration for the Aviso notification service

use utoipa::OpenApi;
use crate::routes::health_check;
use crate::routes::home;
use crate::routes::schema;
use crate::routes::notify;
use crate::routes::watch;
use crate::routes::replay;
use crate::routes::admin;

/// OpenAPI specification for the Aviso notification service
#[derive(OpenApi)]
#[openapi(
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
    ),
    components(
        schemas(
            // Health endpoints
            health_check::HealthResponse,

            // Core request/response types
            crate::types::request::NotificationRequest,
            crate::types::NotificationResponse,

            // Configuration types (add these if they have ToSchema)
            crate::configuration::ApiEventSchema,
            crate::configuration::PayloadConfig,

            // Admin endpoints
            admin::WipeStreamRequest,
            admin::WipeResponse,
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
        (name = "admin", description = "⚠️ Administrative operations - use with extreme caution")  // Add this tag
    )
)]
pub struct ApiDoc;
