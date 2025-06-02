//! Operation validation for endpoint handlers
//!
//! Provides validation functions for ensuring operations are allowed
//! on specific endpoints. This module centralizes operation validation
//! logic that can be reused across different endpoints with different
//! operation restrictions.

use crate::notification::OperationType;
use anyhow::anyhow;

/// Validate that an operation is allowed for a specific endpoint
///
/// This function provides centralized operation validation logic that can be
/// reused across different endpoints with different operation restrictions.
/// It ensures that each endpoint only processes the operations it's designed
/// to handle, providing clear error messages when operations are used on
/// the wrong endpoints.
///
/// # Arguments
/// * `operation` - The operation type extracted from the CloudEvent
/// * `allowed_operations` - List of operations allowed for this endpoint
/// * `endpoint_name` - Name of the endpoint for error messages
///
/// # Returns
/// * `Ok(())` - Operation is allowed for this endpoint
/// * `Err(anyhow::Error)` - Operation not allowed with helpful error message
pub fn validate_operation_for_endpoint(
    operation: OperationType,
    allowed_operations: &[OperationType],
    endpoint_name: &str,
) -> Result<(), anyhow::Error> {
    if allowed_operations.contains(&operation) {
        tracing::debug!(
            operation = ?operation,
            endpoint = endpoint_name,
            allowed_operations = ?allowed_operations,
            "Operation validation passed"
        );
        Ok(())
    } else {
        let operation_str = format!("{:?}", operation).to_lowercase();
        let allowed_strs: Vec<String> = allowed_operations
            .iter()
            .map(|op| format!("{:?}", op).to_lowercase())
            .collect();

        let error_message = format!(
            "The {} endpoint only supports {} operations. \
             Got '{}' operation. Please use the appropriate endpoint for this operation.",
            endpoint_name,
            format_operation_list(&allowed_strs),
            operation_str
        );

        tracing::warn!(
            operation = ?operation,
            endpoint = endpoint_name,
            allowed_operations = ?allowed_operations,
            error = %error_message,
            "Operation validation failed"
        );

        Err(anyhow!(error_message))
    }
}

/// Format a list of operations for error messages
///
/// This helper function creates human-readable lists of operations
/// for use in error messages, handling proper grammar for different
/// list lengths.
///
/// # Arguments
/// * `operations` - List of operation names as strings
///
/// # Returns
/// * `String` - Formatted list with proper grammar
///
/// # Examples
/// * `["notify"]` → `"'notify'"`
/// * `["watch", "replay"]` → `"'watch' and 'replay'"`
/// * `["notify", "watch", "replay"]` → `"'notify', 'watch' and 'replay'"`
fn format_operation_list(operations: &[String]) -> String {
    match operations.len() {
        0 => "no".to_string(),
        1 => format!("'{}'", operations[0]),
        2 => format!("'{}' and '{}'", operations[0], operations[1]),
        _ => {
            let (last, rest) = operations.split_last().unwrap();
            format!("'{}' and '{}'", rest.join("', '"), last)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_operation_list() {
        assert_eq!(format_operation_list(&[]), "no");
        assert_eq!(format_operation_list(&["notify".to_string()]), "'notify'");
        assert_eq!(
            format_operation_list(&["watch".to_string(), "replay".to_string()]),
            "'watch' and 'replay'"
        );
        assert_eq!(
            format_operation_list(&[
                "notify".to_string(),
                "watch".to_string(),
                "replay".to_string()
            ]),
            "'notify', 'watch' and 'replay'"
        );
    }

    #[test]
    fn test_validate_operation_for_endpoint_success() {
        // Test single allowed operation
        let result = validate_operation_for_endpoint(
            OperationType::Notify,
            &[OperationType::Notify],
            "notification",
        );
        assert!(result.is_ok());

        // Test multiple allowed operations
        let result = validate_operation_for_endpoint(
            OperationType::Watch,
            &[OperationType::Watch, OperationType::Replay],
            "watch",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_operation_for_endpoint_failure() {
        // Test operation not in allowed list
        let result = validate_operation_for_endpoint(
            OperationType::Watch,
            &[OperationType::Notify],
            "notification",
        );
        assert!(result.is_err());

        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("notification endpoint only supports 'notify' operations"));
        assert!(error_msg.contains("Got 'watch' operation"));
    }
}
