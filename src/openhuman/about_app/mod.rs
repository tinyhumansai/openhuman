//! User-facing capability catalog for the OpenHuman app.
//!
//! This module is the single source of truth for what the desktop app exposes
//! to end users, including where a capability lives in the UI and whether it is
//! stable, beta, coming soon, or deprecated.

mod catalog;
mod ops;
mod schemas;
mod types;

pub use catalog::{all_capabilities, capabilities_by_category, lookup, search};
pub use ops::{list_capabilities, lookup_capability, search_capabilities};
pub use schemas::{
    about_app_schemas, all_about_app_controller_schemas, all_about_app_registered_controllers,
};
pub use types::{Capability, CapabilityCategory, CapabilityStatus};
