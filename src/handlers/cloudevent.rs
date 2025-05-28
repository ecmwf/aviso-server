use crate::cloudevents::handler::validate_cloudevent;
use actix_web::web;
use serde_json::Value;
use tracing::info;

/// Process CloudEvent validation and setup tracing context
pub async fn process_cloudevent(
    payload: &web::Json<Value>,
) -> Result<crate::cloudevents::handler::CloudEventResponse, anyhow::Error> {
    let response = validate_cloudevent((*payload).clone()).await?;

    tracing::Span::current().record("event_id", &response.event_id);
    tracing::Span::current().record("event_type", &response.event_type);

    info!(
        event_id = %response.event_id,
        event_type = %response.event_type,
        event_source = %response.event_source,
        "CloudEvent successfully processed"
    );

    Ok(response)
}
