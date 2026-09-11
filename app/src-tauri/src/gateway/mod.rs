//! Routing the frontend to a core, wherever that core runs.
//!
//! The app has always been able to reach two cores: the one in this process,
//! and one at a URL somebody else is running. This module adds the ones this
//! app provisions itself — a container here, a machine over SSH, or a container
//! on a machine over SSH — by driving [tinybox], whose model is exactly the
//! distinction that matters: *reach* (which machine) and *confinement* (what
//! contains it) are independent axes, so the third case needs no code of its
//! own.
//!
//! # The seam
//!
//! **A gateway resolves to a URL and a bearer, and nothing else changes.**
//! `core_rpc_url` and `core_rpc_token` answer from the active gateway, so every
//! call site in the renderer — `coreRpcClient`, `relay_http_rpc`, every screen —
//! reaches a remote core through the code that already reached the local one.
//! That is the whole reason this is tractable: the alternative, a transport
//! abstraction threaded through the frontend, would touch every caller.
//!
//! # Layout
//!
//! - [`types`] — what a gateway is
//! - [`store`] — where records live (shell-side, not renderer storage)
//! - [`ops`] — which of those a gateway needs, and what holds it open
//! - [`provision`] — create a box, start a core in it, open a tunnel to it
//! - [`registry`] — which gateway is active, and what is being held open for it
//! - [`commands`] — the Tauri surface
//!
//! [tinybox]: https://github.com/tinyhumansai/tinybox

pub mod commands;
pub mod ops;
pub mod provision;
pub mod registry;
pub mod store;
pub mod types;

// No re-exports: every consumer of this module is either `lib.rs` reaching
// `registry`/`commands`, or one of the submodules reaching a sibling. A
// convenience `pub use` here would be surface nothing asks for.

// Test modules are declared here rather than beside each file so they resolve
// as siblings in this directory. Each is named for the module it covers.
#[cfg(test)]
mod ops_tests;
#[cfg(test)]
mod registry_tests;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod types_tests;
