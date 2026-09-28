// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Validation of the replay end point: `to_id` and `to_date`.
//!
//! The end point mirrors the start point. `to_id` is an inclusive sequence
//! and `to_date` an inclusive time, parsed exactly like `from_id` and
//! `from_date`. At most one may be given, and an end of the same kind as the
//! start must not come before it.

use anyhow::{Result, bail};

use super::NotificationRequest;
use crate::notification_backend::replay::{EndAt, StartAt};

impl NotificationRequest {
    /// True when the request carries `to_id` or `to_date`.
    pub fn has_end_point(&self) -> bool {
        self.to_id.is_some() || self.to_date.is_some()
    }

    /// Rejects an end point sent to an endpoint other than /replay.
    /// `endpoint` is the path it was sent to, such as "/watch".
    pub fn reject_end_point(&self, endpoint: &str) -> Result<()> {
        if self.has_end_point() {
            bail!("to_id and to_date are only supported for the replay endpoint, not {endpoint}");
        }
        Ok(())
    }

    /// Validate the end point against the already validated start point.
    ///
    /// Examples, with `from_id` "100":
    /// - valid: no end point, `to_id` "100", `to_id` "250", any `to_date`
    /// - invalid: `to_id` "99", both `to_id` and `to_date`
    pub fn validate_end_at(&self, start_at: StartAt) -> Result<EndAt> {
        let to_id = Self::parse_sequence_field("to_id", self.to_id.as_deref())?;
        let to_date = Self::parse_date_field("to_date", self.to_date.as_deref())?;
        match (to_id, to_date) {
            (Some(_), Some(_)) => bail!(
                "Cannot specify both to_id and to_date. Please provide only one replay end \
                 parameter. Use to_id for a sequence-based end or to_date for a time-based end."
            ),
            (Some(id), None) => {
                if let StartAt::Sequence(from_id) = start_at
                    && id < from_id
                {
                    bail!("to_id ({id}) must not be lower than from_id ({from_id})");
                }
                Ok(EndAt::Sequence(id))
            }
            (None, Some(date)) => {
                if let StartAt::Date(from_date) = start_at
                    && date < from_date
                {
                    bail!(
                        "to_date ({}) must not be earlier than from_date ({})",
                        date.to_rfc3339(),
                        from_date.to_rfc3339()
                    );
                }
                Ok(EndAt::Date(date))
            }
            (None, None) => Ok(EndAt::Latest),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use std::collections::HashMap;

    fn request(to_id: Option<&str>, to_date: Option<&str>) -> NotificationRequest {
        NotificationRequest {
            event_type: "test_polygon".to_string(),
            identifier: HashMap::new(),
            from_id: None,
            from_date: None,
            to_id: to_id.map(str::to_string),
            to_date: to_date.map(str::to_string),
            payload: None,
        }
    }

    fn date(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    #[test]
    fn no_end_point_ends_at_the_latest_message() {
        let end = request(None, None).validate_end_at(StartAt::Sequence(1));
        assert_eq!(end.unwrap(), EndAt::Latest);
        assert!(!request(None, None).has_end_point());
    }

    #[test]
    fn to_id_is_an_inclusive_sequence_not_below_from_id() {
        let at_start = request(Some("100"), None).validate_end_at(StartAt::Sequence(100));
        assert_eq!(at_start.unwrap(), EndAt::Sequence(100));
        let later = request(Some("250"), None).validate_end_at(StartAt::Sequence(100));
        assert_eq!(later.unwrap(), EndAt::Sequence(250));
        let error = request(Some("99"), None)
            .validate_end_at(StartAt::Sequence(100))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "to_id (99) must not be lower than from_id (100)"
        );
    }

    #[test]
    fn to_date_accepts_the_from_date_formats_and_is_not_before_from_date() {
        for text in ["2025-06-09T13:15:00Z", "2025-06-09 13:15:00", "1749474900"] {
            let end = request(None, Some(text)).validate_end_at(StartAt::Sequence(1));
            assert_eq!(
                end.unwrap(),
                EndAt::Date(date("2025-06-09T13:15:00Z")),
                "{text}"
            );
        }
        let error = request(None, Some("2025-06-09T13:14:59Z"))
            .validate_end_at(StartAt::Date(date("2025-06-09T13:15:00Z")))
            .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("to_date (2025-06-09T13:14:59+00:00) must not be earlier"),
            "{error}"
        );
    }

    #[test]
    fn an_end_of_the_other_kind_is_not_compared_with_the_start() {
        let end = request(None, Some("2000-01-01T00:00:00Z")).validate_end_at(StartAt::Sequence(5));
        assert_eq!(end.unwrap(), EndAt::Date(date("2000-01-01T00:00:00Z")));
        let end =
            request(Some("0"), None).validate_end_at(StartAt::Date(date("2030-01-01T00:00:00Z")));
        assert_eq!(end.unwrap(), EndAt::Sequence(0));
    }

    #[test]
    fn invalid_end_points_are_rejected() {
        let both = request(Some("5"), Some("2025-06-09T13:15:00Z"));
        assert!(both.has_end_point());
        let error = both.validate_end_at(StartAt::Sequence(1)).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("Cannot specify both to_id and to_date")
        );

        let cases = [
            (Some(""), None, "to_id cannot be empty"),
            (Some("-3"), None, "to_id must be a valid positive integer"),
            (None, Some("  "), "to_date cannot be empty"),
            (
                None,
                Some("yesterday"),
                "to_date must be a valid datetime/timestamp",
            ),
        ];
        for (to_id, to_date, expected) in cases {
            let error = request(to_id, to_date)
                .validate_end_at(StartAt::Sequence(1))
                .unwrap_err();
            assert!(error.to_string().starts_with(expected), "{error}");
        }
    }
}
