//! HTTP request handling orchestration
//!
//! This module contains functions that orchestrate between different
//! domain modules (cloudevents, notification) and HTTP concerns

pub mod backend;
pub mod cloudevent;
pub mod notification;

pub use backend::save_to_backend;
pub use cloudevent::process_cloudevent;
pub use notification::process_aviso;
