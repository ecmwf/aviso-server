// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Shared latitude/longitude coordinate parsing.

use serde_json::{Number, Value};
use thiserror::Error;

/// A coordinate in API order: latitude, then longitude.
pub type Coordinate = (f64, f64);

/// Errors produced while parsing a JSON coordinate pair.
#[derive(Debug, Error, PartialEq)]
#[non_exhaustive]
pub enum CoordinateError {
    /// The coordinate was not a two-element JSON array.
    #[error("coordinate must be a two-element array [lat,lon]")]
    InvalidShape,
    /// A coordinate component was not a JSON number.
    #[error("{component} must be a number")]
    NotNumber { component: &'static str },
    /// A coordinate component was not finite.
    #[error("{component} must be finite")]
    NotFinite { component: &'static str },
    /// Latitude was outside its valid range.
    #[error("latitude {value} is outside the valid range [-90, 90]")]
    LatitudeOutOfRange { value: f64 },
    /// Longitude was outside its valid range.
    #[error("longitude {value} is outside the valid range [-180, 180]")]
    LongitudeOutOfRange { value: f64 },
}

/// Parse one JSON `[lat,lon]` pair with finite and range checks.
///
/// Valid: `[52.55,13.5]`. Invalid: `[13.5,52.55,1]`.
pub fn parse_coordinate(value: &Value) -> Result<Coordinate, CoordinateError> {
    let pair = value.as_array().ok_or(CoordinateError::InvalidShape)?;
    if pair.len() != 2 {
        return Err(CoordinateError::InvalidShape);
    }

    let lat = parse_component(&pair[0], "latitude")?;
    let lon = parse_component(&pair[1], "longitude")?;
    validate_coordinate(lat, lon)
}

/// Validate a latitude/longitude pair parsed from a compatible string format.
pub fn validate_coordinate(lat: f64, lon: f64) -> Result<Coordinate, CoordinateError> {
    if !lat.is_finite() {
        return Err(CoordinateError::NotFinite {
            component: "latitude",
        });
    }
    if !lon.is_finite() {
        return Err(CoordinateError::NotFinite {
            component: "longitude",
        });
    }
    if !(-90.0..=90.0).contains(&lat) {
        return Err(CoordinateError::LatitudeOutOfRange { value: lat });
    }
    if !(-180.0..=180.0).contains(&lon) {
        return Err(CoordinateError::LongitudeOutOfRange { value: lon });
    }
    Ok((lat, lon))
}

/// Convert coordinates to their canonical JSON array representation.
pub fn coordinates_to_json(coordinates: &[Coordinate]) -> Result<Value, CoordinateError> {
    coordinates
        .iter()
        .map(|&(lat, lon)| coordinate_to_json(validate_coordinate(lat, lon)?))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

/// Convert one validated coordinate to canonical `[lat,lon]` JSON.
pub fn coordinate_to_json(coordinate: Coordinate) -> Result<Value, CoordinateError> {
    let (lat, lon) = validate_coordinate(coordinate.0, coordinate.1)?;
    let latitude = Number::from_f64(lat).ok_or(CoordinateError::NotFinite {
        component: "latitude",
    })?;
    let longitude = Number::from_f64(lon).ok_or(CoordinateError::NotFinite {
        component: "longitude",
    })?;
    Ok(Value::Array(vec![
        Value::Number(latitude),
        Value::Number(longitude),
    ]))
}

fn parse_component(value: &Value, component: &'static str) -> Result<f64, CoordinateError> {
    let parsed = value
        .as_f64()
        .ok_or(CoordinateError::NotNumber { component })?;
    if !parsed.is_finite() {
        return Err(CoordinateError::NotFinite { component });
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::{CoordinateError, coordinates_to_json, parse_coordinate};
    use serde_json::json;

    #[test]
    fn parses_and_canonicalizes_coordinate() {
        let coordinate = parse_coordinate(&json!([52.5500, 13.5000])).unwrap();
        assert_eq!(coordinate, (52.55, 13.5));
        assert_eq!(
            coordinates_to_json(&[coordinate]).unwrap(),
            json!([[52.55, 13.5]])
        );
    }

    #[test]
    fn rejects_malformed_and_out_of_range_coordinates() {
        assert_eq!(
            parse_coordinate(&json!([1.0])),
            Err(CoordinateError::InvalidShape)
        );
        assert!(matches!(
            parse_coordinate(&json!([91.0, 0.0])),
            Err(CoordinateError::LatitudeOutOfRange { .. })
        ));
        assert!(matches!(
            parse_coordinate(&json!([0.0, 181.0])),
            Err(CoordinateError::LongitudeOutOfRange { .. })
        ));
    }
}
