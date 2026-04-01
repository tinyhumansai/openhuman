//! QuickJS native ops registered on `globalThis.__ops`.
//!
//! Split by category for readability:
//! - `types`       — shared state structs, constants, helpers
//! - `ops_core`    — console, crypto, performance, platform, timers
//! - `ops_net`     — fetch, WebSocket, net bridge
//! - `ops_storage` — IndexedDB, DB bridge, Store bridge
//! - `ops_state`   — published state, filesystem data

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
