// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use aviso_server::configuration::{EventSchema, WatchEndpointSettings};
use serde_json::json;

#[test]
fn zero_and_invalid_caps_are_rejected() {
    for invalid in [json!(0), json!(-1), json!(1.5), json!("unlimited")] {
        let mut watch = serde_json::to_value(WatchEndpointSettings::default()).unwrap();
        watch["max_historical_notifications"] = invalid.clone();
        assert!(serde_json::from_value::<WatchEndpointSettings>(watch).is_err());
        assert!(
            serde_json::from_value::<EventSchema>(json!({
                "identifier": {}, "max_historical_notifications": invalid
            }))
            .is_err()
        );
    }
}

#[test]
fn defaults_and_optional_override() {
    let watch = WatchEndpointSettings::default();
    assert_eq!(watch.max_historical_notifications, 10_000);
    assert_eq!(watch.replay_batch_size, 100);
    for cap in [None, Some(1), Some(20_000)] {
        let schema: EventSchema = serde_json::from_value(json!({
            "identifier": {}, "max_historical_notifications": cap
        }))
        .unwrap();
        assert_eq!(
            schema
                .max_historical_notifications
                .map(std::num::NonZeroUsize::get),
            cap
        );
    }
}

#[test]
fn yaml_caps_validate_through_the_configuration_loader() {
    for (value, valid) in [
        ("1", true),
        ("20000", true),
        ("0", false),
        ("unlimited", false),
    ] {
        let yaml = format!("identifier: {{}}\nmax_historical_notifications: {value}\n");
        let schema = config::Config::builder()
            .add_source(config::File::from_str(&yaml, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize::<EventSchema>();
        assert_eq!(schema.is_ok(), valid, "{yaml}");
        let yaml = format!(
            "sse_heartbeat_interval_sec: 30\nconnection_max_duration_sec: 3600\n\
             replay_batch_size: 100\nreplay_batch_delay_ms: 100\n\
             concurrent_notification_processing: 15\nmax_historical_notifications: {value}\n"
        );
        let watch = config::Config::builder()
            .add_source(config::File::from_str(&yaml, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize::<WatchEndpointSettings>();
        assert_eq!(watch.is_ok(), valid, "{yaml}");
    }
}
