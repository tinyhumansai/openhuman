//! Keyring consent domain — explicit user consent before falling back to local
//! encrypted storage when the OS keyring is unavailable.

pub mod ops;
pub mod policy;
mod schemas;
pub mod types;

pub use schemas::{
    all_keyring_consent_controller_schemas, all_keyring_consent_registered_controllers,
};
pub use types::{ConsentPreference, KeyringStatus, PolicyDecision, StorageMode};
