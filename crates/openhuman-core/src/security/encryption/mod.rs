//! `encryption.*` controller surface. Argon2id/AES-256-GCM password crypt is
//! owned by the tinysecurity module (`PasswordCrypt2`, see
//! `modules/security_crypto.rs`); the former host `EncryptionKey` is gone.

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
