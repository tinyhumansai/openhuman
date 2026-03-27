//! QuickJS native ops registered on `globalThis.__ops`.
//!
//! Split by category for readability:
//! - `types`       — shared state structs, constants, helpers
//! - `ops_core`    — console, crypto, performance, platform, timers
//! - `ops_net`     — fetch, WebSocket, net bridge
//! - `ops_storage` — IndexedDB, DB bridge, Store bridge
//! - `ops_state`   — published state, filesystem data

mod ops_core;
mod ops_net;
mod ops_state;
mod ops_storage;
pub mod types;

// Re-export public API used by qjs_skill_instance.rs
pub use types::{poll_timers, SkillContext, SkillState, TimerState, WebSocketState};

use parking_lot::RwLock;
use rquickjs::{Ctx, Object, Result as JsResult};
use std::sync::Arc;

use crate::runtime::quickjs_libs::storage::IdbStorage;
use types::SkillContext as SC;

/// Register all ops on `globalThis.__ops`.
pub fn register_ops(
    ctx: &Ctx<'_>,
    storage: IdbStorage,
    skill_context: SC,
    skill_state: Arc<RwLock<SkillState>>,
    timer_state: Arc<RwLock<TimerState>>,
    ws_state: Arc<RwLock<WebSocketState>>,
) -> JsResult<()> {
    let globals = ctx.globals();
    let ops = Object::new(ctx.clone())?;

    ops_core::register(ctx, &ops, timer_state)?;
    ops_net::register(ctx, &ops, ws_state)?;
    ops_storage::register(ctx, &ops, storage, skill_context.clone())?;
    ops_state::register(ctx, &ops, skill_state, skill_context.clone())?;

    globals.set("__ops", ops)?;
    Ok(())
}
