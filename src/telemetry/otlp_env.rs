// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Startup checks for the OpenTelemetry environment variables that the OTLP
//! log exporter reads on its own, outside the server's configuration.

use super::otlp::OtlpInitError;
use std::ffi::OsString;

/// The compression variables the OTLP log exporter reads, most specific
/// first: a non-empty logs-specific value takes precedence over the generic
/// one.
const COMPRESSION_VARIABLES: [&str; 2] = [
    "OTEL_EXPORTER_OTLP_LOGS_COMPRESSION",
    "OTEL_EXPORTER_OTLP_COMPRESSION",
];

/// Refuses a compression setting the exporter cannot honour.
///
/// The server is built without the exporter's compression features, so the
/// only usable setting is no compression. The exporter itself ignores any
/// other value and exports uncompressed without a visible warning; the
/// server refuses to start instead, so that the misconfiguration is seen.
/// As in the exporter, an empty value counts as unset and `none` (in any
/// case) explicitly selects no compression.
pub(super) fn check_compression(
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<(), OtlpInitError> {
    for variable in COMPRESSION_VARIABLES {
        let Some(raw) = lookup(variable) else {
            continue;
        };
        let Some(value) = raw.to_str() else {
            return Err(OtlpInitError::UnsupportedCompression {
                variable,
                value: raw.to_string_lossy().into_owned(),
            });
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if value.eq_ignore_ascii_case("none") {
            return Ok(());
        }
        return Err(OtlpInitError::UnsupportedCompression {
            variable,
            value: value.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const LOGS: &str = "OTEL_EXPORTER_OTLP_LOGS_COMPRESSION";
    const GENERIC: &str = "OTEL_EXPORTER_OTLP_COMPRESSION";

    fn check(vars: &[(&str, &str)]) -> Result<(), OtlpInitError> {
        let vars: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), OsString::from(*v)))
            .collect();
        check_compression(|name| vars.get(name).cloned())
    }

    fn rejected_variable(result: Result<(), OtlpInitError>) -> (&'static str, String) {
        match result {
            Err(OtlpInitError::UnsupportedCompression { variable, value }) => (variable, value),
            other => panic!("expected UnsupportedCompression, got {other:?}"),
        }
    }

    #[test]
    fn no_compression_variable_is_accepted() {
        assert!(check(&[]).is_ok());
    }

    #[test]
    fn empty_and_blank_values_count_as_unset() {
        assert!(check(&[(GENERIC, "")]).is_ok());
        assert!(check(&[(LOGS, "  "), (GENERIC, "")]).is_ok());
    }

    #[test]
    fn none_is_accepted_in_any_case() {
        assert!(check(&[(GENERIC, "none")]).is_ok());
        assert!(check(&[(LOGS, " NONE ")]).is_ok());
    }

    #[test]
    fn a_compression_algorithm_is_rejected() {
        assert_eq!(
            rejected_variable(check(&[(GENERIC, "gzip")])),
            (GENERIC, "gzip".to_string())
        );
        assert_eq!(
            rejected_variable(check(&[(LOGS, "zstd")])),
            (LOGS, "zstd".to_string())
        );
    }

    #[test]
    fn an_unknown_value_is_rejected() {
        assert_eq!(
            rejected_variable(check(&[(GENERIC, "bogus")])),
            (GENERIC, "bogus".to_string())
        );
    }

    #[test]
    fn the_logs_specific_value_takes_precedence() {
        assert!(check(&[(LOGS, "none"), (GENERIC, "gzip")]).is_ok());
        assert_eq!(
            rejected_variable(check(&[(LOGS, "gzip"), (GENERIC, "none")])),
            (LOGS, "gzip".to_string())
        );
    }

    #[test]
    fn an_empty_logs_specific_value_falls_back_to_the_generic_one() {
        assert_eq!(
            rejected_variable(check(&[(LOGS, ""), (GENERIC, "gzip")])),
            (GENERIC, "gzip".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_value_that_is_not_unicode_is_rejected() {
        use std::os::unix::ffi::OsStringExt;
        let raw = OsString::from_vec(vec![b'g', 0xff]);
        let result = check_compression(|name| (name == GENERIC).then(|| raw.clone()));
        assert_eq!(rejected_variable(result).0, GENERIC);
    }

    #[test]
    fn the_error_names_the_variable_and_the_way_out() {
        let message = check(&[(GENERIC, "gzip")]).unwrap_err().to_string();
        assert!(message.contains(GENERIC), "{message}");
        assert!(message.contains("\"gzip\""), "{message}");
        assert!(message.contains("none"), "{message}");
    }
}
