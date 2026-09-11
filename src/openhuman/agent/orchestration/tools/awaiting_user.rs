//! Shared helper for the sub-agent `AwaitingUser` pause path.
//!
//! When a delegated sub-agent calls `ask_user_clarification`, the runner
//! checkpoints its conversation and returns
//! [`SubagentRunStatus::AwaitingUser`](crate::openhuman::agent::harness::subagent_runner::SubagentRunStatus).
//! Both the asynchronous [`spawn_subagent`](super::spawn_subagent) path and
//! the synchronous delegate
//! [`dispatch_subagent`](super::dispatch::dispatch_subagent) path must surface
//! that pause to the orchestrator as a structured `[SUBAGENT_AWAITING_USER]`
//! envelope so it relays the question and resumes via `continue_subagent`
//! (instead of re-spawning a fresh, stateless sub-agent — the #4291 loop).
//!
//! The envelope is built here, in one pure, side-effect-free, unit-testable
//! place, so the two call sites cannot drift.

/// Build the `[SUBAGENT_AWAITING_USER]` envelope handed back to the
/// orchestrator as a tool result when a delegated sub-agent pauses on
/// `ask_user_clarification`.
///
/// Pure + side-effect-free: callers publish the matching `SubagentAwaitingUser`
/// domain/progress events separately. The envelope carries the `task_id` and
/// `agent_id` the orchestrator needs to call `continue_subagent`, plus the
/// sub-agent's question, and explicitly instructs the model to resume rather
/// than re-spawn.
pub(crate) fn awaiting_user_envelope(
    task_id: &str,
    agent_id: &str,
    worker_thread_id: Option<&str>,
    question: &str,
    checkpointed: bool,
) -> String {
    let wt_display = worker_thread_id.unwrap_or("(none)");
    // `question` is sub-agent-authored free text. Embedding it raw would let a
    // newline or a literal `[/SUBAGENT_AWAITING_USER]` close the envelope early
    // and inject fake fields / resume instructions the orchestrator now trusts.
    // JSON-encode it: stays on one line, newlines/quotes/delimiters escaped, and
    // the value is clearly bounded — only the real terminator line survives.
    let question_json =
        serde_json::to_string(question).unwrap_or_else(|_| "\"<unserializable question>\"".into());
    // The pause could not be written to disk, so `continue_subagent` will not
    // find a checkpoint for this `task_id`. It may still succeed via the
    // durable-session store, so this warns rather than forbids — but the
    // orchestrator must not be told the history is safely parked when it is
    // not (#5928).
    let resume_caveat = if checkpointed {
        ""
    } else {
        " NOTE: this pause could NOT be saved to disk, so resuming may fail. \
         If continue_subagent reports no checkpoint and no durable session, \
         tell the user the sub-agent's progress was lost instead of retrying."
    };
    format!(
        "[SUBAGENT_AWAITING_USER]\n\
         task_id: {task_id}\n\
         agent_id: {agent_id}\n\
         worker_thread_id: {wt_display}\n\
         question: {question_json}\n\
         [/SUBAGENT_AWAITING_USER]\n\n\
         The sub-agent needs clarification before it can continue. \
         Surface the above question to the user. When the user responds, \
         call continue_subagent with the task_id, agent_id, and the \
         user's answer as the message parameter. Do NOT re-spawn or \
         re-delegate the sub-agent — that restarts it from scratch and \
         loses its progress.{resume_caveat}"
    )
}

#[cfg(test)]
#[path = "awaiting_user_tests.rs"]
mod tests;
