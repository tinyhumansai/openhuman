//! Mobile device pairing domain.
//!
//! Provides X25519 key agreement + XChaCha20-Poly1305 tunnel framing between
//! the Rust core and iOS clients, brokered by the tinyhumans backend tunnel.

pub mod bus;
pub mod crypto;
mod owner;
pub mod rpc;
pub mod schemas;
mod storage_schema;
pub mod store;
mod store_documents;
mod store_import;
pub(crate) use storage_schema::storage_schema;
pub(crate) use store_documents::import_source;
pub mod tunnel_client;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_devices_controller_schemas,
    all_registered_controllers as all_devices_registered_controllers,
};
pub use types::{
    CreatePairingResponse, ListDevicesResponse, PairedDevice, PairingSession, RevokeDeviceResponse,
};
