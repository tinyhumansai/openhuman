//! The per-spawn handle a detached sub-agent records its completion through.
//!
//! Built once at spawn so every terminal path (completed, incomplete, failed,
//! awaiting input) records the same way, in the same workspace router, for the
//! same parent thread. The queue itself is [`super::background_completions`].

use std::path::Path;
use std::path::PathBuf;

use super::background_completions::state;

use super::background_completions::{record_outcome_with_parent_epoch, BackgroundAgentOutcome};

#[cfg(test)]
#[path = "completion_target_tests.rs"]
pub(crate) mod tests;

/// Completion-router key prefix for one task's awaiting-input notices.
pub(super) fn awaiting_input_record_prefix(task_id: &str) -> String {
    let encoded_task_id: String = task_id
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!(
        "{}{}:",
        super::completion_notice::AWAITING_INPUT_RECORD_PREFIX,
        encoded_task_id
    )
}

/// A pause is an independently delivered record, so each pause of a resumed
/// task needs its own key while leaving the terminal task id free.
pub(super) fn awaiting_input_record_id(task_id: &str) -> String {
    format!(
        "{}{}",
        awaiting_input_record_prefix(task_id),
        uuid::Uuid::new_v4().simple()
    )
}

/// Where one detached child's completion lands: its workspace (which router),
/// the parent session (the idle gate) and the parent chat thread (the router's
/// parent key). Built once at spawn so every terminal path records the same way.
#[derive(Clone, Debug)]
pub(crate) struct CompletionTarget {
    /// The dispatch context the child's completions are recorded under.
    ctx: crate::core::runtime::DispatchCtx,
    workspace_dir: PathBuf,
    parent_session: String,
    parent_thread_id: Option<String>,
    parent_epoch: Option<u64>,
}

impl CompletionTarget {
    pub(crate) fn new(
        ctx: crate::core::runtime::DispatchCtx,
        workspace_dir: PathBuf,
        parent_session: String,
        parent_thread_id: Option<String>,
    ) -> anyhow::Result<Self> {
        let parent_epoch = if let Some(thread_id) = parent_thread_id.as_deref() {
            let router =
                super::background_completions::router_for_workspace_ctx(&ctx, &workspace_dir);
            Some(
                router
                    .admit_child_generation(thread_id)?
                    .ok_or_else(|| anyhow::anyhow!("parent thread is stopped or deleted"))?,
            )
        } else {
            None
        };
        Ok(Self {
            ctx,
            workspace_dir,
            parent_session,
            parent_thread_id,
            parent_epoch,
        })
    }

    pub(crate) fn parent_epoch(&self) -> Option<u64> {
        self.parent_epoch
    }

    /// Queue a finished result. See [`record_completion`].
    pub(crate) async fn completed(&self, task_id: &str, agent_id: &str, summary: String) {
        record_outcome_with_parent_epoch(
            &self.ctx,
            &self.workspace_dir,
            &self.parent_session,
            task_id,
            agent_id,
            summary,
            self.parent_thread_id.clone(),
            self.parent_epoch,
            BackgroundAgentOutcome::Completed,
        )
        .await;
    }

    /// Queue a failure. See [`record_failure`].
    pub(crate) async fn failed(&self, task_id: &str, agent_id: &str, error: &str) {
        record_outcome_with_parent_epoch(
            &self.ctx,
            &self.workspace_dir,
            &self.parent_session,
            task_id,
            agent_id,
            format!(
                "[SUBAGENT_FAILED] the async sub-agent errored before producing a result: {error}"
            ),
            self.parent_thread_id.clone(),
            self.parent_epoch,
            BackgroundAgentOutcome::Failed,
        )
        .await;
    }

    /// Queue an awaiting-input pause. See [`record_awaiting_input`].
    pub(crate) async fn awaiting_input(
        &self,
        task_id: &str,
        task_key: &str,
        agent_id: &str,
        question: &str,
        checkpointed: bool,
    ) {
        let record_id = awaiting_input_record_id(task_id);
        let summary = tinyagents_orchestration::subagent::SubagentAwaitingUser {
            task_id: task_id.to_owned(),
            task_key: task_key.to_owned(),
            agent_id: agent_id.to_owned(),
            worker_thread_id: None,
            question: question.to_owned(),
            checkpointed,
        }
        .render();
        record_outcome_with_parent_epoch(
            &self.ctx,
            &self.workspace_dir,
            &self.parent_session,
            record_id,
            agent_id,
            summary,
            self.parent_thread_id.clone(),
            self.parent_epoch,
            BackgroundAgentOutcome::AwaitingInput,
        )
        .await;
    }
}

/// Claim `workspace_dir`'s boot recovery for this process. `true` exactly once
/// per workspace, so the host can scan every workspace it opens (the bootstrap
/// one, then any other a spawn later opens) without rescanning.
pub(crate) fn claim_recovery_ctx(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &Path,
) -> bool {
    state()
        .recovered_workspaces
        .insert(super::background_completions::router_scope_for(
            ctx,
            workspace_dir,
        ))
}

#[cfg(test)]
pub(crate) fn claim_recovery(workspace_dir: &Path) -> bool {
    claim_recovery_ctx(&crate::core::runtime::DispatchCtx::ambient(), workspace_dir)
}

/// Let `workspace_dir` be recovered again (a profile re-leased after release),
/// and reopen its durable store in the cached router before the next scan. The
/// owner migrates records under the router's lease lock, so an active holder
/// can continue through the same router without losing records or leases.
pub(crate) fn forget_recovery_ctx(ctx: &crate::core::runtime::DispatchCtx, workspace_dir: &Path) {
    let scope = super::background_completions::router_scope_for(ctx, workspace_dir);
    let degraded = {
        let mut st = state();
        st.recovered_workspaces.remove(&scope);
        st.degraded_scopes.contains(&scope)
    };
    let Ok(Some(router)) = super::background_completions::router_pool().get(&scope) else {
        return;
    };
    if !degraded {
        // The cached router sits on a durable store opened under the previous
        // lease epoch. Records live in the backend, so there is nothing to
        // migrate: drop the router and let the next lookup open a fresh one at
        // the current epoch. (`replace_store_if_current` would try to migrate
        // between two stores over the same backend, whose per-parent gates are
        // the same durable rows, and would wait on its own source gates; with a
        // fenced previous store it fails with `lease fence: key lost`.)
        // Admissions an old router already handed out stay valid through the
        // pool's parent lifecycle authority.
        let _ = super::background_completions::router_pool().remove(&scope);
        state().stores.remove(&scope);
        log::debug!(
            "[background_completions] dropped cached router for reopen workspace_dir={}",
            workspace_dir.display()
        );
        return;
    }
    let Ok(store) = super::background_completions::open_driver_store(ctx, workspace_dir) else {
        return;
    };
    let previous = router.store();
    match router.replace_store_if_current(&previous, store) {
        Ok(tinyagents_tasks::CompletionStoreReplaceOutcome::Replaced(store))
        | Ok(tinyagents_tasks::CompletionStoreReplaceOutcome::Current(store)) => {
            let mut st = state();
            st.degraded_scopes.remove(&scope);
            st.stores.insert(scope, store);
        }
        Err(error) => {
            log::error!(
                "[background_completions] could not reopen durable completion store workspace_dir={} error={error:#}",
                workspace_dir.display()
            );
        }
    }
}
