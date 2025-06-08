use crate::types::PayloadType;

/// Convert PayloadType to string for backend storage
pub fn convert_payload_to_string(payload: &Option<PayloadType>) -> Option<String> {
    payload.as_ref().map(|p| match p {
        PayloadType::String(s) => s.clone(),
        PayloadType::HashMap(map) => serde_json::to_string(map).unwrap_or_default(),
        PayloadType::CloudEvent(ce) => {
            serde_json::to_string(ce).unwrap_or_else(|_| "{}".to_string())
        }
    })
}

/// Get payload type name for logging
pub fn get_payload_type_name(payload: &Option<PayloadType>) -> Option<&'static str> {
    payload.as_ref().map(|p| p.type_name())
}
