// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use anyhow::{Result, bail};
use tracing::debug;

/// Point coordinate validator.
///
/// Accepts `lat,lon` and `(lat,lon)` input.
pub struct PointHandler;

impl PointHandler {
    pub fn validate_and_canonicalize(value: &str, field_name: &str) -> Result<String> {
        let (lat, lon) = Self::parse_point_coordinates(value)?;
        debug!(
            field = field_name,
            lat = lat,
            lon = lon,
            "Point validated successfully"
        );
        Ok(format!("{},{}", lat, lon))
    }

    pub fn parse_point_coordinates(value: &str) -> Result<(f64, f64)> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("Point coordinate string cannot be empty");
        }

        let inner = trimmed
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .unwrap_or(trimmed)
            .trim();

        let mut parts = inner.split(',').map(str::trim);
        let lat_str = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("Point must be in 'lat,lon' format"))?;
        let lon_str = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("Point must be in 'lat,lon' format"))?;

        if parts.next().is_some() {
            bail!("Point must contain exactly two values: 'lat,lon'");
        }

        let lat: f64 = lat_str
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid latitude value: {}", lat_str))?;
        let lon: f64 = lon_str
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid longitude value: {}", lon_str))?;

        if !(-90.0..=90.0).contains(&lat) {
            bail!("Latitude out of range [-90, 90]: {}", lat);
        }
        if !(-180.0..=180.0).contains(&lon) {
            bail!("Longitude out of range [-180, 180]: {}", lon);
        }

        Ok((lat, lon))
    }
}

#[cfg(test)]
mod tests {
    use super::PointHandler;

    #[test]
    fn parse_point_coordinates_accepts_basic_format() {
        let (lat, lon) = PointHandler::parse_point_coordinates("52.55,13.5").unwrap();
        assert_eq!(lat, 52.55);
        assert_eq!(lon, 13.5);
    }

    #[test]
    fn parse_point_coordinates_accepts_parenthesized_format() {
        let (lat, lon) = PointHandler::parse_point_coordinates("(52.55,13.5)").unwrap();
        assert_eq!(lat, 52.55);
        assert_eq!(lon, 13.5);
    }

    #[test]
    fn parse_point_coordinates_rejects_bad_format() {
        assert!(PointHandler::parse_point_coordinates("52.55").is_err());
        assert!(PointHandler::parse_point_coordinates("52.55,13.5,1.0").is_err());
    }

    #[test]
    fn parse_point_coordinates_rejects_out_of_range() {
        assert!(PointHandler::parse_point_coordinates("91,13.5").is_err());
        assert!(PointHandler::parse_point_coordinates("52.5,181").is_err());
    }

    #[test]
    fn validate_and_canonicalize_returns_canonical_form() {
        let canonical = PointHandler::validate_and_canonicalize("(52.5500,13.5000)", "p").unwrap();
        assert_eq!(canonical, "52.55,13.5");
    }
}
