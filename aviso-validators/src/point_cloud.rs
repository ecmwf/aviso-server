// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Point-cloud identifier validation.

use crate::coordinate::{Coordinate, coordinates_to_json, parse_coordinate};
use serde_json::Value;
use thiserror::Error;

/// Default and current hard limit for points in one cloud.
pub const DEFAULT_MAX_POINTS: usize = 10_000;
/// Current hard upper limit for configured point-cloud sizes.
pub const HARD_MAX_POINTS: usize = 10_000;
/// Conservative Aviso interoperability limit for canonical point-cloud JSON.
///
/// Point clouds are carried in backend metadata, including NATS headers for the
/// JetStream backend. The limit is based on tested round trips with room for
/// Aviso's other metadata, not a protocol-wide NATS header-size limit.
pub const MAX_SERIALIZED_POINT_CLOUD_BYTES: usize = 60 * 1024;

/// Errors produced by point-cloud validation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PointCloudError {
    /// The configured maximum was outside the supported range.
    #[error("field '{field}' max_points must be between 1 and {hard_maximum}, got {configured}")]
    InvalidMaxPoints {
        field: String,
        configured: usize,
        hard_maximum: usize,
    },
    /// The value was not a JSON array.
    #[error("field '{field}' must be a JSON array of [lat,lon] points")]
    NotArray { field: String },
    /// The cloud contained no points.
    #[error("field '{field}' point cloud must contain at least one point")]
    Empty { field: String },
    /// The cloud exceeded its configured point count.
    #[error("field '{field}' point cloud contains {actual} points, maximum is {maximum}")]
    TooManyPoints {
        field: String,
        actual: usize,
        maximum: usize,
    },
    /// One point was malformed or outside the coordinate ranges.
    #[error("field '{field}' point {index} is invalid: {source}")]
    InvalidPoint {
        field: String,
        index: usize,
        #[source]
        source: crate::coordinate::CoordinateError,
    },
    /// The canonical serialized cloud exceeded the storage safety limit.
    #[error(
        "field '{field}' point cloud canonical JSON is {actual} bytes, maximum is {maximum} bytes"
    )]
    SerializedSizeExceeded {
        field: String,
        actual: usize,
        maximum: usize,
    },
    /// Canonical coordinate conversion failed.
    #[error("field '{field}' point cloud canonicalization failed: {source}")]
    Canonicalization {
        field: String,
        #[source]
        source: crate::coordinate::CoordinateError,
    },
}

/// Point-cloud coordinate validator.
pub struct PointCloudHandler;

/// Parsed coordinates and canonical JSON from one point-cloud validation pass.
#[derive(Debug)]
pub struct ValidatedPointCloud {
    coordinates: Vec<Coordinate>,
    canonical: Value,
}

impl ValidatedPointCloud {
    /// Consume the validated cloud without reparsing its canonical JSON.
    pub fn into_parts(self) -> (Vec<Coordinate>, Value) {
        (self.coordinates, self.canonical)
    }
}

impl PointCloudHandler {
    /// Validate once and retain both parsed coordinates and canonical JSON.
    pub fn validate(
        value: &Value,
        max_points: usize,
        field_name: &str,
    ) -> Result<ValidatedPointCloud, PointCloudError> {
        let coordinates = Self::parse_coordinates(value, max_points, field_name)?;
        let canonical = coordinates_to_json(&coordinates).map_err(|source| {
            PointCloudError::Canonicalization {
                field: field_name.to_string(),
                source,
            }
        })?;
        validate_serialized_size(field_name, canonical.to_string().len())?;
        Ok(ValidatedPointCloud {
            coordinates,
            canonical,
        })
    }

    /// Validate a non-empty JSON point array and return canonical JSON.
    pub fn validate_and_canonicalize(
        value: &Value,
        max_points: usize,
        field_name: &str,
    ) -> Result<Value, PointCloudError> {
        let (_, canonical) = Self::validate(value, max_points, field_name)?.into_parts();
        Ok(canonical)
    }

    /// Parse a point-cloud JSON array while preserving order and duplicates.
    pub fn parse_coordinates(
        value: &Value,
        max_points: usize,
        field_name: &str,
    ) -> Result<Vec<Coordinate>, PointCloudError> {
        if max_points == 0 || max_points > HARD_MAX_POINTS {
            return Err(PointCloudError::InvalidMaxPoints {
                field: field_name.to_string(),
                configured: max_points,
                hard_maximum: HARD_MAX_POINTS,
            });
        }
        let points = value.as_array().ok_or_else(|| PointCloudError::NotArray {
            field: field_name.to_string(),
        })?;
        if points.is_empty() {
            return Err(PointCloudError::Empty {
                field: field_name.to_string(),
            });
        }
        if points.len() > max_points {
            return Err(PointCloudError::TooManyPoints {
                field: field_name.to_string(),
                actual: points.len(),
                maximum: max_points,
            });
        }

        points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                parse_coordinate(point).map_err(|source| PointCloudError::InvalidPoint {
                    field: field_name.to_string(),
                    index,
                    source,
                })
            })
            .collect()
    }
}

fn validate_serialized_size(field_name: &str, actual: usize) -> Result<(), PointCloudError> {
    if actual > MAX_SERIALIZED_POINT_CLOUD_BYTES {
        return Err(PointCloudError::SerializedSizeExceeded {
            field: field_name.to_string(),
            actual,
            maximum: MAX_SERIALIZED_POINT_CLOUD_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MAX_SERIALIZED_POINT_CLOUD_BYTES, PointCloudError, PointCloudHandler};
    use serde_json::{Value, json};

    #[test]
    fn accepts_one_point_and_preserves_duplicates_and_order() {
        let value = json!([[1.0, 2.0], [1.0, 2.0], [-3.0, 4.0]]);
        let canonical = PointCloudHandler::validate_and_canonicalize(&value, 10, "point_cloud")
            .expect("valid cloud");
        assert_eq!(canonical, json!([[1.0, 2.0], [1.0, 2.0], [-3.0, 4.0]]));

        let one =
            PointCloudHandler::validate_and_canonicalize(&json!([[52.55, 13.5]]), 1, "point_cloud");
        assert!(one.is_ok());
    }

    #[test]
    fn rejects_strings_empty_clouds_and_malformed_points() {
        assert!(matches!(
            PointCloudHandler::validate_and_canonicalize(&json!("1,2,3,4"), 10, "point_cloud"),
            Err(PointCloudError::NotArray { .. })
        ));
        assert!(matches!(
            PointCloudHandler::validate_and_canonicalize(&json!([]), 10, "point_cloud"),
            Err(PointCloudError::Empty { .. })
        ));
        for malformed in [json!([[1.0]]), json!([[1.0, 2.0, 3.0]]), json!([["1", 2]])] {
            assert!(matches!(
                PointCloudHandler::validate_and_canonicalize(&malformed, 10, "point_cloud"),
                Err(PointCloudError::InvalidPoint { .. })
            ));
        }
    }

    #[test]
    fn rejects_out_of_range_and_too_many_points() {
        for invalid in [json!([[91.0, 0.0]]), json!([[0.0, 181.0]])] {
            assert!(matches!(
                PointCloudHandler::validate_and_canonicalize(&invalid, 10, "point_cloud"),
                Err(PointCloudError::InvalidPoint { .. })
            ));
        }
        assert!(matches!(
            PointCloudHandler::validate_and_canonicalize(
                &json!([[1.0, 2.0], [3.0, 4.0]]),
                1,
                "point_cloud"
            ),
            Err(PointCloudError::TooManyPoints { .. })
        ));
    }

    #[test]
    fn serialized_size_boundary_is_inclusive() {
        assert!(
            super::validate_serialized_size("point_cloud", MAX_SERIALIZED_POINT_CLOUD_BYTES)
                .is_ok()
        );
        assert!(matches!(
            super::validate_serialized_size(
                "point_cloud",
                MAX_SERIALIZED_POINT_CLOUD_BYTES + 1
            ),
            Err(PointCloudError::SerializedSizeExceeded {
                actual,
                maximum,
                ..
            }) if actual == MAX_SERIALIZED_POINT_CLOUD_BYTES + 1
                && maximum == MAX_SERIALIZED_POINT_CLOUD_BYTES
        ));
    }

    #[test]
    fn custom_max_points_accepts_exact_limit_and_rejects_next_point() {
        assert!(
            PointCloudHandler::validate_and_canonicalize(
                &json!([[1.0, 2.0], [3.0, 4.0]]),
                2,
                "point_cloud"
            )
            .is_ok()
        );
        assert!(matches!(
            PointCloudHandler::validate_and_canonicalize(
                &json!([[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]]),
                2,
                "point_cloud"
            ),
            Err(PointCloudError::TooManyPoints {
                actual: 3,
                maximum: 2,
                ..
            })
        ));
    }

    #[test]
    fn rejects_oversized_canonical_json_independently_of_point_count() {
        let point = json!([-89.12345678901234, -179.12345678901235]);
        let cloud = Value::Array(vec![point; 5_000]);
        assert!(matches!(
            PointCloudHandler::validate_and_canonicalize(&cloud, 10_000, "point_cloud"),
            Err(PointCloudError::SerializedSizeExceeded { .. })
        ));
    }

    #[test]
    fn rejects_configured_maximum_above_hard_limit() {
        assert!(matches!(
            PointCloudHandler::validate_and_canonicalize(
                &json!([[1.0, 2.0]]),
                10_001,
                "point_cloud"
            ),
            Err(PointCloudError::InvalidMaxPoints { .. })
        ));
    }
}
