//! OpenHuman's host adapter for the TinyBox module.
//!
//! The host runtime policy is available in every build. The bus-facing
//! operations remain behind the `modules` feature.

#[path = "tinybox/host_runtime.rs"]
pub mod host_runtime;

#[cfg(feature = "modules")]
#[path = "tinybox/operations.rs"]
mod operations;

#[cfg(feature = "modules")]
pub use operations::*;
