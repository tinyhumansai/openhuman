//! QuickJS native operations registered on `globalThis.__ops`.
//!
//! This module acts as the hub for all native Rust functions exposed to the
//! JavaScript environment. Operations are categorized into submodules:
//! - `types`       — shared state structs, constants, and error helpers.
//! - `ops_core`    — fundamental APIs like console, crypto, and timers.
//! - `ops_net`     — networking APIs like fetch and WebSockets.
//! - `ops_storage` — persistence APIs like IndexedDB and SQL bridges.
//! - `ops_state`   — skill-specific state and memory bridge.
//! - `ops_webhook` — webhook registration and management.

pub mod ops;
mod ops_core;
mod ops_net;
mod ops_state;
mod ops_storage;
mod ops_webhook;
pub mod types;

// Re-export public API used by `qjs_skill_instance`
pub use ops::*;
pub use types::{poll_timers, SkillContext, SkillState, TimerState, WebSocketState};
