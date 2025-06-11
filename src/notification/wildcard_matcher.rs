//! Wildcard pattern analysis and matching for watch endpoint subscriptions
//!
//! This module provides intelligent wildcard pattern analysis that optimizes
//! backend subscriptions while maintaining flexible application-level filtering.
//! It implements a hybrid approach where the backend handles coarse filtering
//! and the application handles fine-grained pattern matching.

use tracing::debug;

/// Analyze a watch pattern to determine optimal backend subscription and application filter
///
/// This function implements the hybrid wildcard strategy:
/// 1. Find the first wildcard position in the watch pattern
/// 2. Generate the most specific backend pattern possible (up to first wildcard)
/// 3. Return the full watch pattern for application-level filtering
///
/// # Arguments
/// * `watch_topic` - The topic pattern from the watch request (e.g., "diss.FOO.*.od.*.*.*.*.*.*")
///
/// # Returns
/// * `(String, Vec<String>)` - (backend subscription pattern, full watch pattern as Vec)
pub fn analyze_watch_pattern(watch_topic: &str) -> (String, Vec<String>) {
    let parts: Vec<&str> = watch_topic.split('.').collect();

    // Convert to owned strings for the full pattern
    let full_watch_pattern: Vec<String> = parts.iter().map(|s| s.to_string()).collect();

    // Find the first wildcard position
    let first_wildcard_pos = parts.iter().position(|&part| part == "*");

    let backend_subscription_pattern = match first_wildcard_pos {
        Some(pos) if pos > 1 => {
            // Use JetStream '>' wildcard for everything after first wildcard position
            let specific_parts = &parts[..pos];
            format!("{}.>", specific_parts.join(".")) // Use > instead of .*
        }
        Some(_) => {
            // Wildcard at position 0 or 1, use broad pattern with just the base
            let base = parts.first().map_or("unknown", |v| *v);
            format!("{}.>", base) // Use > instead of .*
        }
        None => {
            // No wildcards present, use specific pattern with > for potential sub-topics
            if parts.len() > 1 {
                let without_last = &parts[..parts.len() - 1];
                format!("{}.>", without_last.join(".")) // Use > instead of .*
            } else {
                // Single part topic, use > wildcard
                format!("{}.>", watch_topic) // Use > instead of .*
            }
        }
    };

    debug!(
        watch_topic = %watch_topic,
        backend_subscription_pattern = %backend_subscription_pattern,
        first_wildcard_pos = ?first_wildcard_pos,
        pattern_parts = parts.len(),
        "Analyzed watch pattern for hybrid filtering"
    );

    (backend_subscription_pattern, full_watch_pattern)
}

/// Check if a notification topic matches a watch pattern
///
/// This function performs position-based pattern matching where:
/// - Non-wildcard parts must match exactly
/// - Wildcard parts ("*") match any value
/// - Both topic and pattern must have the same number of parts
///
/// # Arguments
/// * `notification_topic` - The actual topic from a notification (e.g., "diss.FOO.E1.od.0001.g.20260706.0000.enfo.1")
/// * `watch_pattern` - The watch pattern as a Vec of parts (e.g., ["diss", "FOO", "*", "od", "*", "*", "*", "*", "*", "*"])
///
/// # Returns
/// * `bool` - true if the notification topic matches the watch pattern
pub fn matches_watch_pattern(notification_topic: &str, watch_pattern: &[String]) -> bool {
    let notification_parts: Vec<&str> = notification_topic.split('.').collect();

    // Must have the same number of parts
    if notification_parts.len() != watch_pattern.len() {
        debug!(
            notification_topic = %notification_topic,
            notification_parts = notification_parts.len(),
            pattern_parts = watch_pattern.len(),
            "Topic part count mismatch"
        );
        return false;
    }

    // Check each position
    for (i, (notif_part, pattern_part)) in notification_parts
        .iter()
        .zip(watch_pattern.iter())
        .enumerate()
    {
        if pattern_part != "*" && pattern_part != notif_part {
            debug!(
                notification_topic = %notification_topic,
                position = i,
                notification_part = %notif_part,
                pattern_part = %pattern_part,
                "Pattern mismatch at position"
            );
            return false;
        }
    }

    debug!(
        notification_topic = %notification_topic,
        "Topic matches watch pattern"
    );

    true
}

/// Generate backend-compatible wildcard pattern from a topic pattern
///
/// This is a convenience function that extracts just the backend subscription pattern
/// from the analysis, useful when you only need the subscription pattern.
///
/// # Arguments
/// * `watch_topic` - The topic pattern from the watch request
///
/// # Returns
/// * `String` - The backend subscription pattern
pub fn generate_backend_subscription_pattern(watch_topic: &str) -> String {
    let (backend_pattern, _) = analyze_watch_pattern(watch_topic);
    backend_pattern
}

/// Create a pattern matcher function for a specific watch pattern
///
/// This function returns a closure that can be used to efficiently test
/// multiple notification topics against the same watch pattern.
///
/// # Arguments
/// * `watch_pattern` - The watch pattern as a Vec of parts
///
/// # Returns
/// * `impl Fn(&str) -> bool` - A closure that tests notification topics
pub fn create_pattern_matcher(watch_pattern: Vec<String>) -> impl Fn(&str) -> bool {
    move |notification_topic: &str| matches_watch_pattern(notification_topic, &watch_pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_watch_pattern_early_wildcard() {
        let (backend_pattern, app_pattern) = analyze_watch_pattern("diss.FOO.*.od.*.*.*.*.*.*");
        assert_eq!(backend_pattern, "diss.FOO.>");
        assert_eq!(
            app_pattern,
            vec!["diss", "FOO", "*", "od", "*", "*", "*", "*", "*", "*"]
        );
    }

    #[test]
    fn test_analyze_watch_pattern_late_wildcard() {
        let (backend_pattern, app_pattern) = analyze_watch_pattern("diss.FOO.E1.od.*.*.*.*.*.*");
        assert_eq!(backend_pattern, "diss.FOO.E1.od.>");
        assert_eq!(
            app_pattern,
            vec!["diss", "FOO", "E1", "od", "*", "*", "*", "*", "*", "*"]
        );
    }

    #[test]
    fn test_analyze_watch_pattern_immediate_wildcard() {
        let (backend_pattern, app_pattern) = analyze_watch_pattern("diss.*.*.*.*.*.*.*.*.*");
        assert_eq!(backend_pattern, "diss.>");
        assert_eq!(
            app_pattern,
            vec!["diss", "*", "*", "*", "*", "*", "*", "*", "*", "*"]
        );
    }

    #[test]
    fn test_analyze_watch_pattern_no_wildcards() {
        let (backend_pattern, app_pattern) =
            analyze_watch_pattern("diss.FOO.E1.od.0001.g.20260706.0000.enfo.1");
        assert_eq!(
            backend_pattern,
            "diss.FOO.E1.od.0001.g.20260706.0000.enfo.>"
        );
        assert_eq!(
            app_pattern,
            vec![
                "diss", "FOO", "E1", "od", "0001", "g", "20260706", "0000", "enfo", "1"
            ]
        );
    }

    #[test]
    fn test_matches_watch_pattern_exact_match() {
        let pattern = vec!["diss".to_string(), "FOO".to_string(), "E1".to_string()];
        assert!(matches_watch_pattern("diss.FOO.E1", &pattern));
    }

    #[test]
    fn test_matches_watch_pattern_with_wildcards() {
        let pattern = vec![
            "diss".to_string(),
            "FOO".to_string(),
            "*".to_string(),
            "od".to_string(),
        ];
        assert!(matches_watch_pattern("diss.FOO.E1.od", &pattern));
        assert!(matches_watch_pattern("diss.FOO.E2.od", &pattern));
        assert!(!matches_watch_pattern("diss.FOO.E1.mars", &pattern));
    }

    #[test]
    fn test_matches_watch_pattern_length_mismatch() {
        let pattern = vec!["diss".to_string(), "FOO".to_string()];
        assert!(!matches_watch_pattern("diss.FOO.E1", &pattern));
    }

    #[test]
    fn test_matches_watch_pattern_complex() {
        let pattern = vec![
            "diss".to_string(),
            "FOO".to_string(),
            "*".to_string(),
            "od".to_string(),
            "*".to_string(),
            "*".to_string(),
            "*".to_string(),
            "*".to_string(),
            "*".to_string(),
            "*".to_string(),
        ];

        assert!(matches_watch_pattern(
            "diss.FOO.E1.od.0001.g.20260706.0000.enfo.1",
            &pattern
        ));
        assert!(matches_watch_pattern(
            "diss.FOO.E2.od.0002.g.20260707.1200.enfo.2",
            &pattern
        ));
        assert!(!matches_watch_pattern(
            "diss.BAR.E1.od.0001.g.20260706.0000.enfo.1",
            &pattern
        ));
        assert!(!matches_watch_pattern(
            "mars.FOO.E1.od.0001.g.20260706.0000.enfo.1",
            &pattern
        ));
    }

    #[test]
    fn test_create_pattern_matcher() {
        let pattern = vec!["diss".to_string(), "*".to_string(), "E1".to_string()];
        let matcher = create_pattern_matcher(pattern);

        assert!(matcher("diss.FOO.E1"));
        assert!(matcher("diss.BAR.E1"));
        assert!(!matcher("diss.FOO.E2"));
        assert!(!matcher("mars.FOO.E1"));
    }
}
