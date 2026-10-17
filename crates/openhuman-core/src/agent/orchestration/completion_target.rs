//! The per-spawn handle a detached sub-agent records its completion through.
//!
//! Built once at spawn so every terminal path (completed, incomplete, failed,
//! awaiting input) records the same way, in the same workspace router, for the
//! same parent thread. The queue itself is [`super::background_completions`].

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(test)]
use tinyagents_tasks::CompletionStore;

use super::background_completions::state;
#[cfg(test)]
use super::background_completions::{new_router, Entry};

use super::background_completions::{record_awaiting_input, record_completion, record_failure};

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
}

impl CompletionTarget {
    pub(crate) fn new(
        ctx: crate::core::runtime::DispatchCtx,
        workspace_dir: PathBuf,
        parent_session: String,
        parent_thread_id: Option<String>,
    ) -> Self {
        Self {
            ctx,
            workspace_dir,
            parent_session,
            parent_thread_id,
        }
    }

    /// Queue a finished result. See [`record_completion`].
    pub(crate) async fn completed(&self, task_id: &str, agent_id: &str, summary: String) {
        record_completion(
            &self.ctx,
            &self.workspace_dir,
            &self.parent_session,
            task_id,
            agent_id,
            summary,
            self.parent_thread_id.clone(),
        )
        .await;
    }

    /// Queue a failure. See [`record_failure`].
    pub(crate) async fn failed(&self, task_id: &str, agent_id: &str, error: &str) {
        record_failure(
            &self.ctx,
            &self.workspace_dir,
            &self.parent_session,
            task_id,
            agent_id,
            error,
            self.parent_thread_id.clone(),
        )
        .await;
    }

    /// Queue an awaiting-input pause. See [`record_awaiting_input`].
    pub(crate) async fn awaiting_input(
        &self,
        task_id: &str,
        agent_id: &str,
        question: &str,
        checkpointed: bool,
    ) {
        record_awaiting_input(
            &self.ctx,
            &self.workspace_dir,
            &self.parent_session,
            task_id,
            agent_id,
            question,
            checkpointed,
            self.parent_thread_id.clone(),
        )
        .await;
    }
}

/// A temporary workspace that forgets its router when dropped, so the
/// process-wide registry does not accumulate entries for deleted directories.
#[cfg(test)]
pub(crate) struct TestWorkspace(tempfile::TempDir);

#[cfg(test)]
impl TestWorkspace {
    pub(crate) fn new() -> Self {
        Self(tempfile::tempdir().expect("tempdir"))
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        self.0.path()
    }
}

#[cfg(test)]
impl Drop for TestWorkspace {
    fn drop(&mut self) {
        forget_workspace_for_test(self.0.path());
    }
}

/// Register `store` as `workspace_dir`'s completion store, so a test can inject
/// one that fails.
#[cfg(test)]
pub(crate) fn install_store_for_test(workspace_dir: &Path, store: Arc<dyn CompletionStore>) {
    let entry = Arc::new(Entry {
        router: Arc::new(new_router(store.clone())),
        store,
    });
    state().routers.insert(workspace_dir.to_path_buf(), entry);
}

/// Forget everything this process knows about `workspace_dir`, as a restart
/// would: the router (and its open log handle) and every thread/session mapping
/// that points at it. The on-disk log is untouched.
#[cfg(test)]
pub(crate) fn forget_workspace_for_test(workspace_dir: &Path) {
    let mut st = state();
    st.routers.remove(workspace_dir);
    let gone: Vec<String> = st
        .thread_workspaces
        .iter()
        .filter(|(_, ws)| ws.as_path() == workspace_dir)
        .map(|(thread, _)| thread.clone())
        .collect();
    for thread in gone {
        st.thread_workspaces.remove(&thread);
        st.stopped_threads.remove(&thread);
        st.deleted_threads.remove(&thread);
    }
    st.recovered_workspaces.remove(workspace_dir);
    st.session_threads.clear();
    st.session_order.clear();
}

/// Claim `workspace_dir`'s boot recovery for this process. `true` exactly once
/// per workspace, so the host can scan every workspace it opens (the bootstrap
/// one, then any other a spawn later opens) without rescanning.
pub(crate) fn claim_recovery(workspace_dir: &Path) -> bool {
    state()
        .recovered_workspaces
        .insert(workspace_dir.to_path_buf())
}

/// Let `workspace_dir` be recovered again (a profile re-leased after release),
/// and drop its cached router when nothing is using it so the next scan replays
/// the log another holder may have appended to. A router still in use stays,
/// so a log never has two writers.
pub(crate) fn forget_recovery(workspace_dir: &Path) {
    let mut st = state();
    st.recovered_workspaces.remove(workspace_dir);
    let idle = st
        .routers
        .get(workspace_dir)
        .is_some_and(|e| Arc::strong_count(e) == 1 && Arc::strong_count(&e.router) == 1);
    if idle {
        st.routers.remove(workspace_dir);
    }
}
