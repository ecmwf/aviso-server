use std::collections::HashMap;

/// Operation type for different validation modes
///
/// The notification system supports two distinct operation modes:
/// - **Notify**: Strict validation where all schema fields are required
/// - **Listen**: Flexible validation where only required fields are validated,
///   missing optional fields are filled with "*" for wildcard matching
#[derive(Debug, Clone, Copy)]
pub enum OperationType {
    /// All schema fields must be present and valid
    /// Used when storing notifications in the backend
    Notify,

    /// Only required fields must be present and valid
    /// Missing optional fields get "*" for pattern matching
    /// Used when setting up watches/subscripts
    Listen,
}

impl OperationType {
    /// Convert string operation to OperationType
    pub fn from_str(operation: &str) -> Result<Self, anyhow::Error> {
        match operation {
            "notify" => Ok(OperationType::Notify),
            "listen" => Ok(OperationType::Listen),
            _ => anyhow::bail!("Invalid operation type: {}", operation),
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
