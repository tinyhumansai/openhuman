//! Deterministic task-card dispatcher.
//!
//! Turns a [`TaskBoardCard`] into work: it **claims** the card via a
//! compare-and-set (re-load the board and transition only a `Todo`/`Ready`
//! card to `in_progress`, so a stale/concurrent re-dispatch of the same card
//! is rejected), runs a single **autonomous agent turn** toward the card's
//! objective, and **writes the outcome back** to the board (`done` + evidence
//! on success, `blocked` + reason on failure).
//!
//! This is the one executor both dispatch paths converge on:
//! - the **board poller** (cards that arrived without a proactive trigger), and
//! - the **proactive triage** arm (`agent::triage::apply_decision`), once it has
//!   decided to act on a task-board card.
//!
//! The runner mirrors `skills::spawn_workflow_run_background`: build the
//! `orchestrator` agent fresh inside a detached task, cap tool iterations, and
//! run `agent.run_single` under `with_autonomous_iter_cap`. PR-4 generalises the
//! executor from the default agent to a resolved personality/skill; this module
//! keeps the default-agent path so the pipeline runs end-to-end first.

mod dispatch;
mod executor;
mod poller;
mod prompt;
mod registry;
mod types;

#[cfg(test)]
#[path = "task_dispatcher_tests.rs"]
mod tests;

// ── Public API ────────────────────────────────────────────────────────────────

pub use dispatch::dispatch_card;
pub use poller::start_board_poller;
pub use prompt::build_task_prompt;
pub use registry::{cancel_session, cancel_session_scoped};
pub use types::DispatchOutcome;

/// Run a one-off **system** agent turn on an existing chat thread, streaming the
/// result into it like a normal assistant turn (the same web-channel bridge
/// cron / welcome agents use). Used by the background-completion delivery
/// subsystem to surface a finished detached sub-agent's result back into the
/// chat. Best-effort: returns the final response text or an error string.
pub async fn run_system_turn_on_thread(
    thread_id: String,
    prompt: String,
) -> Result<String, String> {
    let config = crate::config::Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e:#}"))?;
    run_delivery_turn(config, thread_id, prompt).await
}

/// How many of the thread's latest messages a delivery turn is given: enough
/// to include an answer that superseded the result, without the whole thread.
const DELIVERY_THREAD_CONTEXT_MESSAGES: usize = 8;

/// [`run_system_turn_on_thread`] with the config supplied.
///
/// The delivery agent is built fresh, so on its own it sees only the notice. It
/// then cannot tell that the user was already answered by another route, and
/// posts a stale result — a failure, typically — as the thread's last word
/// (#6345). Seeding it with the thread's recent messages lets it merge or
/// supersede instead.
async fn run_delivery_turn(
    config: crate::config::Config,
    thread_id: String,
    prompt: String,
) -> Result<String, String> {
    let executor = executor::resolve_executor(&config.workspace_dir, None);
    let run_id = format!("bgdeliver-{}", uuid::Uuid::new_v4());
    let thread_context = match crate::memory::conversations::blocking::get_messages(
        config.workspace_dir.clone(),
        thread_id.clone(),
    )
    .await
    {
        Ok(messages) => {
            let skip = messages
                .len()
                .saturating_sub(DELIVERY_THREAD_CONTEXT_MESSAGES);
            messages
                .into_iter()
                .skip(skip)
                .map(|m| (m.sender, m.content))
                .collect()
        }
        Err(err) => {
            log::warn!(
                "[background_delivery] could not read thread {thread_id} for delivery \
                 context — delivering without it: {err}"
            );
            Vec::new()
        }
    };
    log::debug!(
        "[background_delivery] delivery turn run_id={run_id} thread_id={thread_id} \
         context_messages={}",
        thread_context.len()
    );
    executor::run_autonomous(
        config,
        &executor,
        &prompt,
        &run_id,
        Some(thread_id),
        thread_context,
    )
    .await
}
