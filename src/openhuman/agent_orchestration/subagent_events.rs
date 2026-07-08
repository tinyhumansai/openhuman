//! Single typed owner for sub-agent lifecycle broadcast events.
//!
//! Sub-agent lifecycle (`SubagentSpawned` / `SubagentCompleted` /
//! `SubagentFailed` / `SubagentAwaitingUser`) used to be hand-constructed and
//! `publish_global`-ed inline across every turn/orchestration path
//! (`tools/dispatch.rs`, `tools/spawn_subagent.rs`,
//! `tools/spawn_async_subagent.rs`, `tools/continue_subagent.rs`,
//! `tools/agent_prepare_context.rs`, `spawn_parallel_graph.rs`). That scattered
//! the ordering/rate-limiting decision across many call sites.
//!
//! This module centralizes construction + publish so there is exactly one place
//! future ordering / rate-limiting / journal-mirroring will hook. It is a pure
//! single-ownership refactor: the emitted [`DomainEvent`] variants and their
//! field values are IDENTICAL to the previous inline publishes, so
//! `RunLedgerFinalizeSubscriber` and every other subscriber observe no change.
//!
//! Part of `docs/tinyagents-full-migration-plan/05-events/02-bridge-consolidation.md`
//! step 3 (the lifecycle-publish sweep).

use crate::core::event_bus::{publish_global, DomainEvent};

/// A sub-agent was dispatched. Mirrors [`DomainEvent::SubagentSpawned`].
pub(crate) fn publish_subagent_spawned(
    parent_session: String,
    agent_id: String,
    mode: String,
    task_id: String,
    prompt_chars: usize,
) {
    tracing::debug!(
        parent_session = %parent_session,
        task_id = %task_id,
        agent_id = %agent_id,
        mode = %mode,
        prompt_chars,
        "[subagent-events] spawned"
    );
    publish_global(DomainEvent::SubagentSpawned {
        parent_session,
        agent_id,
        mode,
        task_id,
        prompt_chars,
    });
}

/// A sub-agent finished successfully (or stopped incomplete but
/// lifecycle-completed). Mirrors [`DomainEvent::SubagentCompleted`].
pub(crate) fn publish_subagent_completed(
    parent_session: String,
    task_id: String,
    agent_id: String,
    elapsed_ms: u64,
    output_chars: usize,
    iterations: usize,
) {
    tracing::debug!(
        parent_session = %parent_session,
        task_id = %task_id,
        agent_id = %agent_id,
        elapsed_ms,
        output_chars,
        iterations,
        "[subagent-events] completed"
    );
    publish_global(DomainEvent::SubagentCompleted {
        parent_session,
        task_id,
        agent_id,
        elapsed_ms,
        output_chars,
        iterations,
    });
}

/// A sub-agent failed. Mirrors [`DomainEvent::SubagentFailed`].
pub(crate) fn publish_subagent_failed(
    parent_session: String,
    task_id: String,
    agent_id: String,
    error: String,
) {
    tracing::debug!(
        parent_session = %parent_session,
        task_id = %task_id,
        agent_id = %agent_id,
        error = %error,
        "[subagent-events] failed"
    );
    publish_global(DomainEvent::SubagentFailed {
        parent_session,
        task_id,
        agent_id,
        error,
    });
}

/// A sub-agent paused awaiting user input. Mirrors
/// [`DomainEvent::SubagentAwaitingUser`].
pub(crate) fn publish_subagent_awaiting_user(
    parent_session: String,
    task_id: String,
    agent_id: String,
    question: String,
) {
    tracing::debug!(
        parent_session = %parent_session,
        task_id = %task_id,
        agent_id = %agent_id,
        "[subagent-events] awaiting_user"
    );
    publish_global(DomainEvent::SubagentAwaitingUser {
        parent_session,
        task_id,
        agent_id,
        question,
    });
}
