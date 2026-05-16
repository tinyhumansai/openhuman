//! Connectivity domain — diagnostics for the local core sidecar's
//! reachability and current backend Socket.IO state.
//!
//! The frontend has three independent connectivity channels (browser internet,
//! backend Socket.IO websocket, local core sidecar HTTP). Issue #1527 split
//! them in the UI so users see *which* channel is broken instead of a single
//! "Disconnected" pill that conflated all three.
//!
//! This Rust-side module exposes a cheap `openhuman.connectivity_diag` RPC that
//! lets the frontend (and future tooling) read the live backend-socket state
//! plus the local sidecar's process id and listening port. The endpoint is
//! intentionally lightweight — no I/O, just snapshots from in-memory state and
//! a single TCP probe — so the UI can poll it as a health-check ping without
//! adding significant load.

pub mod ops;
pub mod rpc;
mod schemas;

pub use schemas::{
    all_controller_schemas as all_connectivity_controller_schemas,
    all_registered_controllers as all_connectivity_registered_controllers,
};
