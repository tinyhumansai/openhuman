//! Persisting a paused sub-agent so `continue_subagent` can resume it.
//!
//! Distinct from [`super::checkpoint`], which summarises a *cap-hit* run into
//! resumable prose. This one is the on-disk conversation snapshot written when
//! a child exits early on `ask_user_clarification`.

/// Persist a paused sub-agent's conversation so `continue_subagent` can resume
/// it, returning the path actually written or `None` when it could not be.
///
/// Every failure here is logged at `error`, not `warn`. A checkpoint that was
/// not written is not a degraded nicety: the run is about to be reported as
/// `AwaitingUser`, the orchestrator will relay a `task_id` and ask the user to
/// answer, and the loss only becomes visible after they have — at which point
/// the answer has nowhere to go. That late, invisible failure is what #5928's
/// first acceptance criterion is about, and the `Option` this returns is what
/// lets the caller stop promising a resumable pause it did not achieve.
///
/// Its own module so each failure branch is reachable from a test, and so
/// `runner.rs` stays under the layout gate's line pin: point `checkpoint_dir` at a path that cannot be created, or at one
/// whose target file cannot be written.
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

pub(super) fn write(
    checkpoint_dir: &std::path::Path,
    task_id: &str,
    data: &crate::openhuman::agent::harness::subagent_runner::types::SubagentCheckpointData,
) -> Option<std::path::PathBuf> {
    if !is_safe_task_id(task_id) {
        tracing::error!(
            task_id = %task_id,
            dir = %checkpoint_dir.display(),
            "[subagent_runner] refusing to write a checkpoint under an unsafe task id; \
             this pause will not be resumable from disk"
        );
        return None;
    }

    if let Err(e) = std::fs::create_dir_all(checkpoint_dir) {
        tracing::error!(
            task_id = %task_id,
            dir = %checkpoint_dir.display(),
            error = %e,
            "[subagent_runner] could not create the checkpoint directory; this pause will not be \
             resumable from disk"
        );
        return None;
    }

    let json = match serde_json::to_string_pretty(data) {
        Ok(json) => json,
        Err(e) => {
            tracing::error!(
                task_id = %task_id,
                error = %e,
                "[subagent_runner] could not serialize the checkpoint; this pause will not be \
                 resumable from disk"
            );
            return None;
        }
    };

    let checkpoint_path = checkpoint_dir.join(format!("{task_id}.json"));
    if let Err(e) = std::fs::write(&checkpoint_path, json) {
        tracing::error!(
            task_id = %task_id,
            path = %checkpoint_path.display(),
            error = %e,
            "[subagent_runner] could not write the checkpoint; this pause will not be resumable \
             from disk"
        );
        return None;
    }

    tracing::info!(
        task_id = %task_id,
        path = %checkpoint_path.display(),
        history_len = data.history.len(),
        "[subagent_runner] checkpoint written for awaiting_user"
    );
    Some(checkpoint_path)
}

#[cfg(test)]
#[path = "runner_checkpoint_tests.rs"]
mod tests;
