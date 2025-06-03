//! Integration tests for the CloudEvents module
//!
//! These tests cover serialization, data extraction, validation, default application,
//! and end-to-end payload processing for CloudEvents.

extern crate aviso_server;

use anyhow::Result;
use aviso_server::cloudevents::{
    CloudEventConverter, CloudEventProcessor, CloudEventValidator, apply_defaults,
};
use cloudevents::AttributesReader;
use cloudevents::Event;
use serde_json::json;

/// Helper to build a CloudEvent from a JSON payload
fn event_from_json(payload: serde_json::Value) -> Event {
    serde_json::from_value::<Event>(payload).expect("Failed to deserialize CloudEvent from JSON")
}

#[test]
/// Ensure that `serialize_for_storage` produces a JSON string containing all core attributes
fn test_serialize_for_storage() {
    // Build a minimal CloudEvent payload
    let payload = json!({
        "specversion": "1.0",
        "id": "test-id",
        "source": "/test/source",
        "type": "test-type",
        "datacontenttype": "application/json",
        "data": { "foo": "bar" }
    });
    let event = event_from_json(payload);
    let serialized =
        CloudEventConverter::serialize_for_storage(&event).expect("Serialization failed");
    // The output must include the id, source, type, and the nested data
    assert!(
        serialized.contains("\"id\":\"test-id\""),
        "Missing id in serialized output"
    );
    assert!(
        serialized.contains("\"source\":\"/test/source\""),
        "Missing source in serialized output"
    );
    assert!(
        serialized.contains("\"foo\":\"bar\""),
        "Missing data in serialized output"
    );
}

#[test]
/// `extract_data_as_json` should return the JSON data when present
fn test_extract_data_as_json_from_json() {
    let payload = json!({
        "specversion": "1.0",
        "id": "data-json",
        "source": "/src",
        "type": "type",
        "datacontenttype": "application/json",
        "data": { "key": 123 }
    });
    let event = event_from_json(payload.clone());
    let extracted = CloudEventConverter::extract_data_as_json(&event);
    assert!(extracted.is_some());
    assert_eq!(extracted.unwrap(), payload.get("data").unwrap().clone());
}

#[test]
/// `extract_data_as_json` should parse string data when it contains valid JSON
fn test_extract_data_as_json_from_string_valid() {
    let json_str = "{ \"num\": 10 }";
    let payload = json!({
        "specversion": "1.0",
        "id": "data-str",
        "source": "/src",
        "type": "type",
        "datacontenttype": "text/plain",
        "data": json_str
    });
    let event = event_from_json(payload);
    let extracted = CloudEventConverter::extract_data_as_json(&event);
    assert!(extracted.is_some());
    assert_eq!(extracted.unwrap()["num"], 10);
}

#[test]
/// `extract_data_as_json` should return None for string data that is not valid JSON
fn test_extract_data_as_json_from_string_invalid() {
    let payload = json!({
        "specversion": "1.0",
        "id": "data-str-bad",
        "source": "/src",
        "type": "type",
        "datacontenttype": "text/plain",
        "data": "not a json"
    });
    let event = event_from_json(payload);
    let extracted = CloudEventConverter::extract_data_as_json(&event);
    assert!(extracted.is_none());
}

#[test]
/// Validation should pass for a minimal valid CloudEvent JSON structure
fn test_validate_json_cloudevent_valid() {
    let payload = json!({
        "specversion": "1.0",
        "id": "valid-1",
        "source": "/ok",
        "type": "ok"
    });
    let result = CloudEventValidator::validate_json_cloudevent(&payload);
    assert!(result.is_ok(), "Expected valid payload to pass validation");
}

#[test]
/// Validation should catch missing required attributes
fn test_validate_missing_fields() {
    let payload = json!({ "id": "no-spec" });
    let result = CloudEventValidator::validate_json_cloudevent(&payload);
    assert!(
        result.is_err(),
        "Expected missing fields to fail validation"
    );
    let err = result.err().unwrap().to_string();
    assert!(err.contains("Missing required attributes"));
}

#[test]
/// Validation should reject unsupported specversion values
fn test_validate_wrong_specversion() {
    let payload = json!({
        "specversion": "0.2",
        "id": "x",
        "source": "/s",
        "type": "t"
    });
    let result = CloudEventValidator::validate_json_cloudevent(&payload);
    // Only need to assert that validation fails
    assert!(
        result.is_err(),
        "Expected unsupported specversion to fail validation"
    );
}

#[test]
/// `apply_defaults` should fill in missing optional attributes
fn test_apply_defaults_missing_optional() -> Result<()> {
    let payload = json!({
        "specversion": "1.0",
        "id": "",
        "source": "",
        "type": "evt",
        // include data so default content type is applied
        "data": { "x": 1 }
    });
    let raw_event = event_from_json(payload);
    let event = apply_defaults(raw_event)?;

    // ID should no longer be empty
    assert!(!event.id().is_empty());
    // Source should be our default
    assert_eq!(event.source(), "/aviso-server");
    // Time must now be set
    assert!(event.time().is_some());
    // Default content type applied for JSON data
    assert_eq!(event.datacontenttype().unwrap(), "application/json");
    Ok(())
}

#[tokio::test]
/// Full processing pipeline should accept valid JSON payload and return an Event
async fn test_process_json_payload_end_to_end() -> Result<()> {
    let payload = json!({
        "specversion": "1.0",
        "id": "proc-1",
        "source": "/src",
        "type": "t",
        // omit time to trigger default
        "data": { "foo": "bar" }
    });
    let event = CloudEventProcessor::process_json_payload(payload)?;
    // Check that defaults have been applied
    assert!(!event.id().is_empty());
    assert_eq!(event.source(), "/src");
    assert_eq!(event.ty(), "t");
    assert!(event.time().is_some());
    Ok(())
}
