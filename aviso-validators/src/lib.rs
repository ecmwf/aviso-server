// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Field validation library for Aviso notification system
//!
//! This library provides validation handlers for different field types

pub mod constraints;
pub mod date;
pub mod enum_handler;
pub mod expver;
pub mod float;
pub mod int;
pub mod point;
pub mod polygon;
pub mod string;
pub mod time;
pub mod types;

// Re-export main types for convenience
pub use types::ValidationRules;

// Re-export handlers
pub use constraints::{
    EnumConstraint, NumericConstraint, parse_enum_constraint, parse_float_constraint,
    parse_int_constraint,
};
pub use date::DateHandler;
pub use enum_handler::EnumHandler;
pub use expver::ExpverHandler;
pub use float::FloatHandler;
pub use int::IntHandler;
pub use point::PointHandler;
pub use polygon::PolygonHandler;
pub use string::StringHandler;
pub use time::TimeHandler;
