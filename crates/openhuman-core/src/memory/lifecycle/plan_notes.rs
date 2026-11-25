//! A finished plan's notes, kept in long-term memory.
//!
//! The agent writes facts and blockers to its scratchpad through the `todo`
//! tool while it works (TinyAgents `scratchpad`). After the final reply of a
//! turn in which the plan finished, the session host takes them
//! (`scratchpad::store::close_finished_plan`) and this module queues each one
//! as a learning on the agent's node, through the same durable queue as the
//! `memory` tool's `learn`: facts as `fact`, blockers as `correction`. The
//! queue scrubs and confines them exactly as it does a model's own write, and
//! nothing here waits on the engine.

use std::sync::Arc;

use serde_json::json;
use tinyagents_harness_tools::scratchpad::ClosedPlan;
use tinymemory_tools::layout::identity::ResolvedIdentity;
use tinymemory_tools::{CallContext, SingleTool};

use crate::config::Config;
use crate::memory::tool_writes::HostWriteSink;
use crate::memory::tools::{CallFacts, MEMORY_TOOL_NAME};

/// Longest task label carried as a learning's evidence, in characters.
const MAX_TASK_CHARS: usize = 160;

/// One learning per note: `(text, kind)`.
fn learnings(plan: &ClosedPlan) -> Vec<(String, &'static str)> {
    let facts = plan.notes.facts.iter().map(|fact| (fact.clone(), "fact"));
    let blockers = plan
        .notes
        .blockers
        .iter()
        .map(|blocker| (format!("Blocker: {blocker}"), "correction"));
    facts.chain(blockers).collect()
}

/// What the notes came from: the goal, else the plan's first step.
fn task_label(plan: &ClosedPlan, goal: Option<&str>) -> String {
    let task = goal
        .or_else(|| plan.items.first().map(|item| item.content.as_str()))
        .unwrap_or_default();
    let line = task.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= MAX_TASK_CHARS {
        return line;
    }
    let mut cut: String = line.chars().take(MAX_TASK_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// Queues `plan`'s notes as learnings of `identity`, with the thread `goal`
/// (else the plan's first step) as their evidence, returning how many were
/// queued. Best-effort: a refused write (memory off, queue full) is logged and
/// skipped.
pub async fn learn_plan_notes(
    config: Arc<Config>,
    identity: &ResolvedIdentity,
    workspace: Option<String>,
    plan: &ClosedPlan,
    goal: Option<&str>,
) -> usize {
    let scope = CallFacts {
        workspace,
        thread_id: Some(plan.thread_id.clone()),
        ..CallFacts::of_resolved(identity)
    }
    .native_scope(&config);
    let tool = SingleTool::queued(
        MEMORY_TOOL_NAME,
        scope,
        Arc::new(HostWriteSink::new(config.clone())),
    );
    let evidence = format!(
        "noted by the agent while working on: {}",
        task_label(plan, goal)
    );
    let notes = learnings(plan);
    let total = notes.len();
    // The queue opens its storage under the current profile scope. Like the
    // memory tool's writes, the work runs on a task that carries the caller's
    // scope (the session host calls this inside the profile's scope): a bare
    // spawn would run unscoped, and in SaaS the queue refuses every write.
    let dispatch = crate::core::runtime::DispatchCtx::ambient();
    let queued = dispatch
        .spawn(async move {
            let mut queued = 0;
            for (text, kind) in notes {
                let args = json!({
                    "action": "learn",
                    "text": text,
                    "kind": kind,
                    "evidence": evidence,
                });
                match tool
                    .call(
                        args,
                        CallContext {
                            thread_window: None,
                        },
                    )
                    .await
                {
                    Ok(_) => queued += 1,
                    Err(error) => {
                        tracing::debug!(%error, "[memory] plan_notes: learning not queued");
                    }
                }
            }
            queued
        })
        .await
        .unwrap_or(0);
    tracing::debug!(
        thread_id = %plan.thread_id,
        queued,
        total,
        "[memory] plan_notes: finished plan's notes queued as learnings"
    );
    queued
}

#[cfg(test)]
#[path = "plan_notes_tests.rs"]
mod tests;
