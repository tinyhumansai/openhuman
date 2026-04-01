use super::*;

use parking_lot::RwLock;
use rquickjs::{Ctx, Object, Result as JsResult};
use std::sync::Arc;

use super::types::SkillContext as SC;
use crate::openhuman::skills::quickjs_libs::storage::IdbStorage;

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
    ops_webhook::register(ctx, &ops, skill_context)?;

    globals.set("__ops", ops)?;
    Ok(())
}
