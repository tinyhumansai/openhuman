//! Mobile device pairing domain.
//!
//! Provides X25519 key agreement + XChaCha20-Poly1305 tunnel framing between
//! the Rust core and iOS clients, brokered by the tinyhumans backend tunnel.

pub mod bus;
pub mod crypto;
pub mod rpc;
pub mod schemas;
pub mod store;
pub mod tunnel_client;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_devices_controller_schemas,
    all_registered_controllers as all_devices_registered_controllers,
};
pub use types::{
    CreatePairingResponse, ListDevicesResponse, PairedDevice, PairingSession, RevokeDeviceResponse,
};
