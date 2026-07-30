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
//!
//! ## Marketplace scope (seller-side writes are web-only)
//!
//! The identity marketplace **write** handlers here are buyer-side only: buy a
//! listing, bid, and make an offer. Seller-side *writes* — listing a handle for
//! sale and accepting / rejecting offers — are **web-only on tiny.place** (the
//! backend exposes no seller write routes). Seller-side *reads* do exist:
//! `marketplace_list_offers` takes a `name` filter for offers targeting a handle
//! you own. The desktop Trading tab links sellers to the web app for the actions
//! it cannot perform. See #4920.
//!
//! ## Feed scope (post media is web-only)
//!
//! Feed posts here are **text-only**. Attaching images / GIFs to a post is
//! **web-only on tiny.place**: the backend serves no media field — neither the
//! write-side `PostCreate` nor the read-side `Post` / `GqlPost` in the vendored
//! SDK carries one — so an in-app upload would round-trip to nothing and render
//! nowhere. The desktop feed composer links users to the web app to attach
//! media instead of dead-ending. See #4924.

pub(crate) mod agent;
mod agent_tools;
mod manifest;
pub(crate) use manifest::{
    acknowledge_message, decrypt_envelope, ensure_signal_keys_published,
    handle_tinyplace_contacts_list, handle_tinyplace_directory_get_agent,
    handle_tinyplace_directory_reverse, handle_tinyplace_signal_key_status,
    handle_tinyplace_signal_provision, handle_tinyplace_signal_register_encryption_key,
    handle_tinyplace_signal_send_message,
};
pub(crate) mod ops;
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
