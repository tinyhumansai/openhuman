//! Compatibility RPC namespace for secret encryption operations.
//!
//! The handlers delegate to the credentials domain, which routes ciphertext
//! operations through TinySecurity. This module does not own cipher code or
//! key material.

pub mod ops;
mod schemas;

pub use ops as rpc;
pub use ops::*;
pub(crate) use schemas::all_controller_entries as all_encryption_controller_entries;
pub use schemas::{
    all_controller_schemas as all_encryption_controller_schemas,
    all_registered_controllers as all_encryption_registered_controllers,
};

#[cfg(test)]
pub(crate) use schemas::encryption_parameter_contracts;
