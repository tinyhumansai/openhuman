//! The task-id guard for paused sub-agents.
//!
//! The pause itself is a lifecycle record on the storage port
//! (`subagent_host::lifecycle`); a model-authored `task_id` still has to pass
//! [`is_safe_task_id`] before it names one.

/// Whether `task_id` is usable as a single filename component.
///
/// This is untrusted input on its way into a path join. `continue_subagent`
/// takes `task_id` straight from its tool arguments — which the model authors —
/// and passes it back through `SubagentRunOptions::task_id`, so a re-paused
/// child writes its checkpoint under a name the model chose. Without this,
/// `../../../../tmp/pwn` walks the write clean out of the checkpoint directory.
///
/// Deliberately an allow-list rather than a `..`-blocklist: every id this
/// system mints is `sub-{uuid}`, `subsess-{uuid}` or a short test label, so
/// nothing legitimate needs a separator, a dot, or a non-ASCII character, and
/// an allow-list cannot be walked around by an encoding this check did not
/// anticipate.
pub(crate) fn is_safe_task_id(task_id: &str) -> bool {
    !task_id.is_empty()
        && task_id.len() <= 128
        && task_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

#[cfg(test)]
#[path = "runner_checkpoint_tests.rs"]
mod tests;
