// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::notification::spatial::SpatialMetadata;
use aviso_validators::{EnumConstraint, NumericConstraint};
use serde::Serialize;
use std::collections::HashMap;
use std::str::FromStr;

/// Processing mode for schema validation.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub enum OperationType {
    /// All schema fields must be present.
    Notify,

    /// Required fields are enforced; optional fields may be wildcarded.
    Watch,

    /// Same field rules as watch; used for historical retrieval.
    Replay,
}

impl FromStr for OperationType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "notify" => Ok(OperationType::Notify),
            "watch" => Ok(OperationType::Watch),
            "replay" => Ok(OperationType::Replay),
            _ => anyhow::bail!("Invalid operation type: {}", s),
        }
    }
}

impl OperationType {
    /// Return operation as static string.
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationType::Notify => "notify",
            OperationType::Watch => "watch",
            OperationType::Replay => "replay",
        }
    }

    /// List all operation variants.
    pub fn all_operations() -> Vec<Self> {
        vec![
            OperationType::Notify,
            OperationType::Watch,
            OperationType::Replay,
        ]
    }

    /// List all operation strings.
    pub fn all_operation_strings() -> Vec<&'static str> {
        Self::all_operations()
            .iter()
            .map(|op| op.as_str())
            .collect()
    }
}

/// Aviso CloudEvent type helpers.
pub struct AvisoCloudEventTypes;

impl AvisoCloudEventTypes {
    /// Prefix for Aviso CloudEvent types.
    pub const AVISO_TYPE_PREFIX: &'static str = "int.ecmwf.aviso";

    /// Build all supported Aviso CloudEvent type strings.
    pub fn get_supported_types() -> Vec<String> {
        OperationType::all_operations()
            .iter()
            .map(|op| format!("{}.{}", Self::AVISO_TYPE_PREFIX, op.as_str()))
            .collect()
    }

    /// Build error text for unsupported CloudEvent types.
    pub fn get_unsupported_type_error(actual_type: &str) -> String {
        let supported_types = Self::get_supported_types();
        format!(
            "Only Aviso CloudEvent types are supported. Got: '{}'. Expected one of: [{}]",
            actual_type,
            supported_types.join(", ")
        )
    }

    /// Check only prefix, not operation suffix validity.
    pub fn is_aviso_type(cloudevent_type: &str) -> bool {
        cloudevent_type.starts_with(Self::AVISO_TYPE_PREFIX)
    }

    /// Validate type and parse operation suffix.
    pub fn validate_and_extract_operation(
        cloudevent_type: &str,
    ) -> Result<OperationType, anyhow::Error> {
        if !cloudevent_type.starts_with(Self::AVISO_TYPE_PREFIX) {
            anyhow::bail!(
                "Invalid Aviso CloudEvent type '{}'. Must start with '{}'",
                cloudevent_type,
                Self::AVISO_TYPE_PREFIX
            );
        }

        let operation_part = cloudevent_type
            .strip_prefix(Self::AVISO_TYPE_PREFIX)
            .and_then(|s| s.strip_prefix('.'))
            .unwrap_or("");

        match OperationType::from_str(operation_part) {
            Ok(operation) => {
                tracing::debug!(
                    cloudevent_type = cloudevent_type,
                    operation = operation_part,
                    "Valid Aviso CloudEvent type with operation"
                );
                Ok(operation)
            }
            Err(_) => {
                anyhow::bail!(
                    "Invalid Aviso CloudEvent operation '{}' in type '{}'. \
                     Supported operations: [{}]",
                    operation_part,
                    cloudevent_type,
                    OperationType::all_operation_strings().join(", ")
                );
            }
        }
    }
}

/// Result returned by notification processing.
#[derive(Debug, Clone)]
pub struct ProcessingResult {
    /// Event type name.
    pub event_type: String,
    /// Routed topic subject.
    pub topic: String,
    /// Canonicalized request parameters.
    pub canonicalized_params: HashMap<String, String>,
    /// Optional identifier constraints for watch/replay fine-grained filtering.
    pub identifier_constraints: HashMap<String, IdentifierConstraint>,
    /// Optional spatial metadata from polygon fields.
    pub spatial_metadata: Option<SpatialMetadata>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdentifierConstraint {
    Int(NumericConstraint<i64>),
    Enum(EnumConstraint),
    /// Floating-point identifier constraints for schema fields using FloatHandler.
    Float(NumericConstraint<f64>),
}
