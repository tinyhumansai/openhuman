//! OpenHuman's adapter for TinyAgents' caller-owned thread reasoning store.

use std::sync::Arc;

use tinyagents_harness::ids::ThreadId;
use tinyagents_harness::thread_reasoning::ThreadReasoningStore;
use tinyinference_llm::model::{ReasoningConfig, ReasoningEffort};

use crate::core::runtime::DispatchCtx;

/// The reasoning store belongs to this dispatch context/profile. Context
/// clones share the slot; separately derived profiles remain isolated.
pub(crate) fn store(ctx: &DispatchCtx) -> Arc<ThreadReasoningStore> {
    crate::core::runtime::slot_for::<ThreadReasoningStore>(ctx)
}

/// Applies a requested effort through the owner store and preserves the
/// existing RPC validation message for unknown values.
pub(crate) fn apply_requested_effort(
    ctx: &DispatchCtx,
    thread_id: &str,
    raw: Option<&str>,
) -> Result<(), String> {
    let Some(raw) = raw else {
        return Ok(());
    };

    let thread_id = ThreadId::new(thread_id);
    let store = store(ctx);
    let previous = store
        .apply_requested(Some(&thread_id), Some(raw))
        .map_err(|_| format!("unknown reasoning_effort '{}'", raw.trim()))?;
    let requested = ReasoningEffort::parse(raw);
    if previous != requested {
        log::info!(
            "[agent][reasoning] thread reasoning effort changed thread_id={} {:?} -> {:?}",
            thread_id.as_str(),
            previous.map(ReasoningEffort::as_str),
            requested.map(ReasoningEffort::as_str)
        );
    }
    Ok(())
}

/// Snapshots root-turn reasoning, keeping child turns at the provider default.
pub(crate) fn turn_reasoning_for(
    ctx: &crate::agent::tinyagents::host::OpenHumanRunContext,
    config: Option<&crate::config::Config>,
    max_output_tokens: Option<u32>,
) -> Option<ReasoningConfig> {
    if ctx.spawn_depth > 0 {
        return None;
    }

    let dispatch = ctx.dispatch_ctx();
    let reasoning = if let Some(thread_id) = ctx.thread_id.as_deref() {
        store(&dispatch).reasoning_for_turn(Some(&ThreadId::new(thread_id)), max_output_tokens)
    } else {
        None
    }
    .or_else(|| {
        crate::inference::reasoning::reasoning_for_config(config?)
            .map(|reasoning| reasoning.with_output_room(max_output_tokens))
    });

    if let Some(reasoning) = reasoning.as_ref() {
        log::debug!(
            "[agent][reasoning] turn reasoning effort={:?} budget_tokens={:?} thread_id={:?}",
            reasoning.effort,
            reasoning.budget_tokens,
            ctx.thread_id
        );
    }
    reasoning
}

#[cfg(test)]
pub(crate) fn thread_effort(ctx: &DispatchCtx, thread_id: &str) -> Option<ReasoningEffort> {
    store(ctx).get(&ThreadId::new(thread_id))
}

#[cfg(test)]
pub(crate) fn set_thread_effort(
    ctx: &DispatchCtx,
    thread_id: &str,
    effort: Option<ReasoningEffort>,
) {
    let thread_id = ThreadId::new(thread_id);
    match effort {
        Some(effort) => {
            store(ctx).set_effort(&thread_id, effort);
        }
        None => {
            store(ctx).clear(&thread_id);
        }
    }
}
