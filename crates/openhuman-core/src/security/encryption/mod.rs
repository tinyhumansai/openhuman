//! `encryption.*` controller surface. Argon2id/AES-256-GCM password crypt is
//! owned by the tinysecurity module (`PasswordCrypt2`, see
//! `modules/security_crypto.rs`); the former host `EncryptionKey` is gone.

pub mod ops;
mod schemas;

pub use ops as rpc;
pub use ops::*;
pub use schemas::{
    all_controller_schemas as all_encryption_controller_schemas,
    all_registered_controllers as all_encryption_registered_controllers,
};
