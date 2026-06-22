//! **tiny.place** A2A social-network integration — core domain.
//!
//! Namespace: `tinyplace`.  RPC methods: `openhuman.tinyplace_*`.
//!
//! Controllers are registered in the **internal** registry (callable by the
//! renderer via `core_rpc_relay` but NOT advertised to agents via tool listings
//! or schema discovery).  See [`schemas`] for the registration and
//! [`manifest`] for the handler implementations.
//!
//! ## Architecture
//!
//! ```text
//! Renderer (invoke 'core_rpc_relay', method='openhuman.tinyplace_*')
//!   └─► src/core/all.rs  build_internal_only_controllers()
//!         └─► schemas::all_tinyplace_registered_controllers()
//!               └─► manifest::handle_tinyplace_*()
//!                     └─► state::TinyPlaceState::client()
//!                           └─► tinyplace::TinyPlaceClient
//! ```
//!
//! ## Seed derivation
//!
//! `TinyPlaceState::client()` calls `wallet::tinyplace_signer_seed()` on first
//! access. The seed is derived via the same SLIP-0010 path used for all Solana
//! signing (`m/44'/501'/0'/0'`); the wallet key becomes the tiny.place identity.
//! The seed is never logged, persisted, or returned across any IPC boundary.

pub(crate) mod agent;
mod agent_tools;
mod manifest;
mod ops;
mod payment;
mod schemas;
pub(crate) mod signal_store;
mod state;
pub(crate) mod streams;
pub mod tools;

#[cfg(test)]
mod signal_e2e_tests;
#[cfg(test)]
mod tests;

pub use schemas::{all_tinyplace_controller_schemas, all_tinyplace_registered_controllers};
