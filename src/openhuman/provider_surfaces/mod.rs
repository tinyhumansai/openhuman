//! Local assistive surfaces for third-party provider apps.
//!
//! This domain will own the normalized event model, respond queue, local
//! draft shelf, and provider-specific assistive actions that sit above
//! embedded webviews and future API-first integrations.
//!
//! The initial scaffold is intentionally minimal so the namespace can be
//! wired into the controller registry before behavioral work begins.

pub mod ops;
pub mod rpc;
pub mod schemas;
pub mod store;
pub mod types;

pub use schemas::{
    all_provider_surfaces_controller_schemas, all_provider_surfaces_registered_controllers,
};
