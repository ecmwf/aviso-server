//! Field validation library for Aviso notification system
//!
//! This library provides validation handlers for different field types

pub mod date;
pub mod enum_handler;
pub mod expver;
pub mod int;
pub mod polygon;
pub mod point;
pub mod string;
pub mod time;
pub mod types;

// Re-export main types for convenience
pub use types::ValidationRules;

// Re-export handlers
pub use date::DateHandler;
pub use enum_handler::EnumHandler;
pub use expver::ExpverHandler;
pub use int::IntHandler;
pub use polygon::PolygonHandler;
pub use point::PointHandler;
pub use string::StringHandler;
pub use time::TimeHandler;
