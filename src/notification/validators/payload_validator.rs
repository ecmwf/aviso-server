use crate::configuration::Settings;
use crate::types::PayloadType;
use anyhow::{Result, anyhow};

/// Validate payload type against schema configuration
pub fn validate_payload_type(event_type: &str, payload: &Option<PayloadType>) -> Result<()> {
    let schema = Settings::get_global_notification_schema();

    let schema_map = schema
        .as_ref()
        .ok_or_else(|| anyhow!("No notification schema configured"))?;

    let event_schema = schema_map
        .get(event_type)
        .ok_or_else(|| anyhow!("Unknown event type: {}", event_type))?;

    // Handle Option<PayloadConfig>
    let payload_config = match &event_schema.payload {
        Some(config) => config,
        None => {
            // If no payload config, payload should not be provided
            if payload.is_some() {
                return Err(anyhow!(
                    "Payload not allowed for event type '{}'",
                    event_type
                ));
            }
            return Ok(());
        }
    };

    match payload {
        Some(payload_value) => {
            let payload_type = payload_value.type_name();
            if !payload_config
                .allowed_types
                .contains(&payload_type.to_string())
            {
                return Err(anyhow!(
                    "Payload type '{}' not allowed for event type '{}'. Allowed types: {:?}",
                    payload_type,
                    event_type,
                    payload_config.allowed_types
                ));
            }
        }
        None => {
            if payload_config.required
                && !payload_config
                    .allowed_types
                    .contains(&"NoneType".to_string())
            {
                return Err(anyhow!(
                    "Payload is required for event type '{}' but not provided",
                    event_type
                ));
            }
        }
    }

    Ok(())
}
