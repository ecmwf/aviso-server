// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use super::Settings;
use anyhow::{Result, bail};
use aviso_validators::ValidationRules;
use std::collections::HashSet;

/// Validate that configured topics retain the identifiers needed for routing.
pub fn validate_topic_schema_settings(settings: &Settings) -> Result<()> {
    for (event_type, schema) in settings.notification_schema.iter().flatten() {
        let Some(topic) = &schema.topic else {
            continue;
        };
        if topic.key_order.is_empty() {
            bail!("Schema '{event_type}' topic.key_order must not be empty");
        }
        let mut seen = HashSet::new();
        for key in &topic.key_order {
            if key == "point_cloud" {
                bail!(
                    "Schema '{event_type}' topic.key_order must not contain reserved spatial key 'point_cloud'"
                );
            }
            if !schema.identifier.contains_key(key) {
                bail!(
                    "Schema '{event_type}' topic.key_order contains undeclared identifier '{key}'"
                );
            }
            if !seen.insert(key) {
                bail!(
                    "Schema '{event_type}' topic.key_order contains duplicate identifier '{key}'"
                );
            }
        }
        for (key, field) in &schema.identifier {
            // Spatial geometry is persisted as metadata rather than subject tokens.
            if !seen.contains(key)
                && !matches!(
                    field.rule,
                    ValidationRules::PolygonHandler { .. }
                        | ValidationRules::PointCloudHandler { .. }
                )
            {
                bail!("Schema '{event_type}' topic.key_order must include identifier '{key}'");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings(order: serde_json::Value, identifier: serde_json::Value) -> Settings {
        serde_json::from_value(json!({
            "application": {"host": "127.0.0.1", "port": 0},
            "notification_backend": {"kind": "in_memory"},
            "notification_schema": {"weather": {
                "topic": {"base": "weather", "key_order": order},
                "identifier": identifier
            }}
        }))
        .unwrap()
    }

    #[test]
    fn validates_order_entries() {
        let identifier = json!({"region": {"type": "StringHandler", "required": true}});
        for (order, message) in [
            (json!([]), "must not be empty"),
            (
                json!(["unknown"]),
                "contains undeclared identifier 'unknown'",
            ),
            (
                json!(["region", "region"]),
                "contains duplicate identifier 'region'",
            ),
            (
                json!(["point_cloud"]),
                "must not contain reserved spatial key 'point_cloud'",
            ),
        ] {
            let error =
                validate_topic_schema_settings(&settings(order, identifier.clone())).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Schema 'weather' topic.key_order {message}")
            );
        }
        validate_topic_schema_settings(&settings(json!(["region"]), identifier)).unwrap();
    }

    #[test]
    fn requires_every_ordinary_handler_even_when_optional() {
        for rule in [
            json!({"type": "StringHandler", "required": false}),
            json!({"type": "DateHandler", "canonical_format": "%Y%m%d", "required": false}),
            json!({"type": "EnumHandler", "values": ["a", "b"], "required": false}),
            json!({"type": "ExpverHandler", "required": false}),
            json!({"type": "IntHandler", "required": false}),
            json!({"type": "FloatHandler", "required": false}),
            json!({"type": "TimeHandler", "required": false}),
        ] {
            let identifier = json!({
                "region": {"type": "StringHandler", "required": true},
                "value": rule
            });
            let invalid = settings(json!(["region"]), identifier.clone());
            assert_eq!(
                validate_topic_schema_settings(&invalid)
                    .unwrap_err()
                    .to_string(),
                "Schema 'weather' topic.key_order must include identifier 'value'"
            );
            validate_topic_schema_settings(&settings(json!(["value", "region"]), identifier))
                .unwrap();
        }
    }

    #[test]
    fn retains_spatial_metadata_and_legacy_polygon_routing() {
        for (field, handler) in [
            ("polygon", "PolygonHandler"),
            ("point_cloud", "PointCloudHandler"),
        ] {
            let identifier = json!({
                "region": {"type": "StringHandler", "required": true},
                field: {"type": handler, "required": true}
            });
            let config = settings(json!(["region"]), identifier.clone());
            validate_topic_schema_settings(&config).unwrap();
            super::super::validate_spatial_schema_settings(&config).unwrap();
            let routed = settings(json!(["region", field]), identifier);
            assert_eq!(
                validate_topic_schema_settings(&routed).is_ok(),
                field == "polygon"
            );
        }
        let config = settings(
            json!(["region"]),
            json!({
                "region": {"type": "StringHandler", "required": true},
                "polygon": {"type": "StringHandler", "required": false}
            }),
        );
        assert!(validate_topic_schema_settings(&config).is_err());
    }

    #[test]
    fn leaves_no_topic_and_no_schema_behavior_unchanged() {
        let mut config = settings(json!([]), json!({}));
        config
            .notification_schema
            .as_mut()
            .unwrap()
            .get_mut("weather")
            .unwrap()
            .topic = None;
        validate_topic_schema_settings(&config).unwrap();
        config.notification_schema = None;
        validate_topic_schema_settings(&config).unwrap();
    }

    #[test]
    fn omitted_scalar_cannot_restrict_watch_or_replay_delivery() {
        use crate::notification::wildcard_matcher::{
            analyze_watch_pattern, matches_notification_filters, matches_watch_pattern,
        };
        use crate::notification::{NotificationProcessor, NotificationRegistry, OperationType};
        use std::collections::HashMap;

        for routed in [false, true] {
            let order = if routed {
                json!(["region", "destination"])
            } else {
                json!(["region"])
            };
            let config = settings(
                order,
                json!({
                    "region": {"type": "StringHandler", "required": true},
                    "destination": {"type": "StringHandler", "required": true}
                }),
            );
            assert_eq!(validate_topic_schema_settings(&config).is_ok(), routed);
            let registry =
                NotificationRegistry::from_config(config.notification_schema.as_ref().unwrap());
            let processor = NotificationProcessor::new(&registry);
            let published = processor
                .process_request(
                    "weather",
                    &HashMap::from([
                        ("region".into(), "north".into()),
                        ("destination".into(), "denied".into()),
                    ]),
                    &None,
                    OperationType::Notify,
                )
                .unwrap();
            for operation in [OperationType::Watch, OperationType::Replay] {
                let request = processor
                    .process_request(
                        "weather",
                        &HashMap::from([
                            ("region".into(), "north".into()),
                            ("destination".into(), "allowed".into()),
                        ]),
                        &None,
                        operation,
                    )
                    .unwrap();
                let (_, pattern) = analyze_watch_pattern(&request.topic).unwrap();
                let delivered = matches_watch_pattern(&published.topic, &pattern)
                    && matches_notification_filters(
                        &published.topic,
                        &request.canonicalized_params,
                        &request.identifier_constraints,
                        None,
                        "null",
                    );
                assert_eq!(
                    delivered, !routed,
                    "only routing binds delivery to the requested scalar"
                );
            }
        }
    }

    #[tokio::test]
    async fn startup_rejects_invalid_topic_before_backend_initialization() {
        let config = settings(json!([]), json!({}));
        let result =
            crate::startup::Application::build(config, tokio_util::sync::CancellationToken::new())
                .await;
        let error = result.err().expect("invalid topic must prevent startup");
        assert_eq!(
            error.to_string(),
            "Schema 'weather' topic.key_order must not be empty"
        );
    }
}
