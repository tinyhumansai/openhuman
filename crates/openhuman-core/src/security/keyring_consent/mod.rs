//! Keyring consent domain — explicit user consent before falling back to local
//! encrypted storage when the OS keyring is unavailable.

pub(crate) mod operation_approval;
pub mod ops;
pub mod policy;
mod schemas;
pub mod types;

pub(crate) use schemas::all_keyring_consent_controller_entries;
#[cfg(test)]
pub(crate) use schemas::keyring_consent_parameter_contracts;
pub use schemas::{
    all_keyring_consent_controller_schemas, all_keyring_consent_registered_controllers,
};
pub use types::{ConsentPreference, KeyringStatus, PolicyDecision, StorageMode};
