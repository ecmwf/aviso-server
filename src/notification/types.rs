use std::collections::HashMap;

/// Operation type for different validation modes
///
/// The notification system supports three distinct operation modes:
/// - **Notify**: Strict validation where all schema fields are required
/// - **Watch**: Flexible validation where only required fields are validated,
///   missing optional fields are filled with "*" for wildcard matching
/// - **Replay**: Similar to watch but for retrieving historical messages
#[derive(Debug, Clone, Copy)]
pub enum OperationType {
    /// All schema fields must be present and valid
    /// Used when storing notifications in the backend
    Notify,

    /// Only required fields must be present and valid
    /// Missing optional fields get "*" for pattern matching
    /// Used when setting up watches/subscriptions
    Watch,

    /// Only required fields must be present and valid
    /// Missing optional fields get "*" for pattern matching
    /// Used when retrieving historical messages
    Replay,
}

impl OperationType {
    /// Convert string operation to OperationType
    pub fn from_str(operation: &str) -> Result<Self, anyhow::Error> {
        match operation {
            "notify" => Ok(OperationType::Notify),
            "watch" => Ok(OperationType::Watch),
            "replay" => Ok(OperationType::Replay),
            _ => anyhow::bail!("Invalid operation type: {}", operation),
        }
    }

    /// Convert OperationType to string
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationType::Notify => "notify",
            OperationType::Watch => "watch",
            OperationType::Replay => "replay",
        }
    }

    /// Get all supported operation types
    pub fn all_operations() -> Vec<Self> {
        vec![
            OperationType::Notify,
            OperationType::Watch,
            OperationType::Replay,
        ]
    }

    /// Get all supported operation type strings
    pub fn all_operation_strings() -> Vec<&'static str> {
        Self::all_operations()
            .iter()
            .map(|op| op.as_str())
            .collect()
    }
}

/// Aviso CloudEvent type utilities
pub struct AvisoCloudEventTypes;

impl AvisoCloudEventTypes {
    /// Expected prefix for all Aviso CloudEvent types
    pub const AVISO_TYPE_PREFIX: &'static str = "int.ecmwf.aviso";

    /// Get all supported Aviso CloudEvent types dynamically
    pub fn get_supported_types() -> Vec<String> {
        OperationType::all_operations()
            .iter()
            .map(|op| format!("{}.{}", Self::AVISO_TYPE_PREFIX, op.as_str()))
            .collect()
    }

    /// Get a formatted error message for unsupported types
    pub fn get_unsupported_type_error(actual_type: &str) -> String {
        let supported_types = Self::get_supported_types();
        format!(
            "Only Aviso CloudEvent types are supported. Got: '{}'. Expected one of: [{}]",
            actual_type,
            supported_types.join(", ")
        )
    }

    /// Check if a CloudEvent type is an Aviso type (without validating operation)
    pub fn is_aviso_type(cloudevent_type: &str) -> bool {
        cloudevent_type.starts_with(Self::AVISO_TYPE_PREFIX)
    }

    /// Validate CloudEvent type and extract operation type
    pub fn validate_and_extract_operation(
        cloudevent_type: &str,
    ) -> Result<OperationType, anyhow::Error> {
        // Check if the type starts with the expected Aviso prefix
        if !cloudevent_type.starts_with(Self::AVISO_TYPE_PREFIX) {
            anyhow::bail!(
                "Invalid Aviso CloudEvent type '{}'. Must start with '{}'",
                cloudevent_type,
                Self::AVISO_TYPE_PREFIX
            );
        }

        // Extract the operation suffix after the prefix
        let operation_part = cloudevent_type
            .strip_prefix(Self::AVISO_TYPE_PREFIX)
            .and_then(|s| s.strip_prefix('.'))
            .unwrap_or("");

        // Try to parse the operation using the dynamic OperationType
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

/// Result of notification processing
///
/// Contains all the information needed to store or route a notification:
/// - **topic**: The routing topic for the backend (e.g., "diss.FOO.E1.od.0001")
/// - **payload**: Optional payload data extracted from the request
/// - **canonicalized_params**: All request parameters in their canonical form
#[derive(Debug, Clone)]
pub struct ProcessingResult {
    /// The event type for this notification (e.g., "dissemination", "mars")
    pub event_type: String,
    /// The topic string used for routing in the notification backend
    pub topic: String,
    /// Optional payload data if configured in the schema
    pub payload: Option<String>,
    /// All request parameters after validation and canonicalization
    pub canonicalized_params: HashMap<String, String>,
}
