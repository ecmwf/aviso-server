//! Validation rule types for field validation

use serde::Serialize;

/// Validation rules for different field types
#[derive(serde::Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum ValidationRules {
    /// String field validation with optional length constraints
    StringHandler {
        max_length: Option<usize>,
        required: bool,
    },
    /// Date field validation with configurable output format
    DateHandler {
        canonical_format: String,
        required: bool,
    },
    /// Enumerated value validation against allowed options
    EnumHandler { values: Vec<String>, required: bool },
    /// Experiment version field validation with default values
    ExpverHandler {
        default: Option<String>,
        required: bool,
    },
    /// Integer validation with optional range constraints
    IntHandler {
        range: Option<[i64; 2]>,
        required: bool,
    },
    /// Time field validation supporting multiple input formats
    TimeHandler { required: bool },
    /// Polygon coordinate validation for spatial data
    PolygonHandler { required: bool },
}

impl ValidationRules {
    /// Check if this validation rule requires the field to be present
    pub fn is_required(&self) -> bool {
        match self {
            ValidationRules::StringHandler { required, .. } => *required,
            ValidationRules::DateHandler { required, .. } => *required,
            ValidationRules::EnumHandler { required, .. } => *required,
            ValidationRules::ExpverHandler { required, .. } => *required,
            ValidationRules::IntHandler { required, .. } => *required,
            ValidationRules::TimeHandler { required } => *required,
            ValidationRules::PolygonHandler { required } => *required,
        }
    }
}
