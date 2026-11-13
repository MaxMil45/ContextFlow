//! ContextFlow core domain model.
//!
//! This crate has no I/O and no platform dependencies. Everything else in the
//! workspace (storage, adapters, platform integration, UI) depends on it, and it
//! depends on nothing ContextFlow-specific.

pub mod error;
pub mod geometry;
pub mod hash;
pub mod model;
pub mod redact;
pub mod validate;

pub use error::{CoreError, Result};
pub use model::*;
