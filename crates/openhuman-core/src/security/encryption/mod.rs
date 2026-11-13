//! Compatibility RPC namespace for secret encryption operations.
//!
//! The `encryption.encrypt_secret` / `encryption.decrypt_secret` handlers
//! delegate to [`crate::credentials::ops`], which routes ciphertext operations
//! through TinySecurity. This module does not own cipher code or key material,
//! and it does not encrypt anything by itself: only values a caller passes to
//! these RPCs are encrypted.

mod schemas;

pub(crate) use schemas::all_controller_entries as all_encryption_controller_entries;
pub use schemas::{
    all_controller_schemas as all_encryption_controller_schemas,
    all_registered_controllers as all_encryption_registered_controllers,
};

#[cfg(test)]
pub(crate) use schemas::encryption_parameter_contracts;
