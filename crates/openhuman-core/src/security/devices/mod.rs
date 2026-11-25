//! Mobile device pairing domain.
//!
//! Uses TinySecurity's X25519 identity and tunnel APIs between the Rust core
//! and iOS clients, brokered by the tinyhumans backend tunnel.

pub mod bus;
mod owner;
pub mod rpc;
pub mod schemas;
mod storage_schema;
pub mod store;
mod store_documents;
#[cfg(feature = "legacy-import")]
mod store_import;
mod tunnel_rpc;
pub(crate) use storage_schema::{collection_specs, upgrade_records};
#[cfg(feature = "legacy-import")]
pub(crate) use store_documents::import_source;
pub mod tunnel_client;
pub mod types;

/// Lock device state even after a prior panicking handler poisoned its mutex.
/// Poisoning means an interrupted update may have left state incomplete; it
/// should not make every subsequent tunnel frame panic as well.
pub(super) fn lock_unpoisoned<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) use schemas::all_controller_entries;
#[cfg(test)]
pub(crate) use schemas::devices_parameter_contracts;
pub use schemas::{
    all_controller_schemas as all_devices_controller_schemas,
    all_registered_controllers as all_devices_registered_controllers,
};
pub use types::{
    CreatePairingResponse, ImportLegacyResponse, ListDevicesResponse, PairedDevice, PairingSession,
    RevokeDeviceResponse,
};
