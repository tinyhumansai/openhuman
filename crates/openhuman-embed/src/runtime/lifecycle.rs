//! Removing an agent from a running [`Runtime`].

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use super::Runtime;
use crate::agent::AgentError;

/// How long [`Runtime::remove_agent`] waits for in-flight turns to unwind.
const REMOVE_IDLE_WAIT: Duration = Duration::from_secs(10);

// Reserve the public id until all old-instance cleanup finishes, including
// when a caller cancels the removal future after it has started.
struct RemovalSlot<'a> {
    runtime: &'a Runtime,
    inner: Arc<crate::agent::AgentInner>,
    purge: bool,
    mcp_cleanup_confirmed: bool,
}

impl RemovalSlot<'_> {
    fn purge_home(&mut self) -> Result<(), AgentError> {
        if !std::mem::take(&mut self.purge) {
            return Ok(());
        }
        if !self.mcp_cleanup_confirmed {
            return Err(AgentError::CleanupPending);
        }
        let home = &self.inner.layout.home;
        if home.exists() {
            std::fs::remove_dir_all(home).map_err(|source| AgentError::Workspace {
                what: "delete the removed agent's home",
                source,
            })?;
        }
        Ok(())
    }
}

impl Drop for RemovalSlot<'_> {
    fn drop(&mut self) {
        if self.purge && !self.mcp_cleanup_confirmed {
            self.inner.teardown_without_mcp_cleanup("agent_removed");
            let home = self.inner.layout.home.clone();
            // Only the workspace outlives this removal: the retained callback
            // must not hold the runtime's process slot (see
            // `CoreGuard::workspace_keepalive`).
            let workspace = self.inner._runtime_guard.workspace_keepalive();
            let cleanup = Arc::new(move || match std::fs::remove_dir_all(&home) {
                Ok(()) => {
                    let _ = &workspace;
                    true
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let _ = &workspace;
                    true
                }
                Err(error) => {
                    log::warn!("[embed][runtime] deferred agent purge failed: {error}");
                    let _ = &workspace;
                    false
                }
            });
            openhuman_core::mcp::host::schedule_agent_close_with_cleanup(
                &self.inner.config.workspace_dir,
                &self.inner.id,
                cleanup,
            );
            // Ownership of the requested purge moved to the deferred cleanup
            // reservation, which also fences this agent id until it completes.
            self.purge = false;
        } else {
            self.inner.teardown("agent_removed");
            if let Err(error) = self.purge_home() {
                log::warn!("[embed][runtime] cancelled removal purge failed: {error}");
            }
        }
        // Teardown may drop user-owned state whose destructor blocks or
        // re-enters Runtime. Keep the old Arc in the weak map as the ID
        // reservation while teardown runs, but never hold the map lock across
        // user destructors or cleanup scheduling.
        let mut agents = self
            .runtime
            .agents
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if agents
            .get(&self.inner.id)
            .and_then(std::sync::Weak::upgrade)
            .is_some_and(|current| Arc::ptr_eq(&current, &self.inner))
        {
            agents.remove(&self.inner.id);
        }
    }
}

/// A pending [`Runtime::remove_agent`]. Await it, optionally after
/// [`purge`](Self::purge).
#[must_use = "an agent is removed only when this is awaited"]
pub struct RemoveAgent<'a> {
    runtime: &'a Runtime,
    id: String,
    purge: bool,
}

impl RemoveAgent<'_> {
    /// Also delete the agent's home, `<workspace>/agents/<id>/`, with its
    /// transcripts, skills, job store and MCP store. Kept by default.
    pub fn purge(mut self) -> Self {
        self.purge = true;
        self
    }
}

impl<'a> IntoFuture for RemoveAgent<'a> {
    type Output = Result<(), AgentError>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.runtime.remove(&self.id, self.purge).await })
    }
}

impl Runtime {
    /// Remove agent `id` from this runtime.
    ///
    /// New turns on any handle to it are refused with
    /// [`CoreError::AgentRemoved`](crate::CoreError::AgentRemoved); its parked
    /// approvals are denied with resolution `agent_removed`; turns in
    /// flight end with the same error, waited on for up to ten seconds; its
    /// state slots are cleared and its configured MCP client is closed before
    /// its context is deregistered, so its cron jobs stay dormant. The id is
    /// free for reuse once cleanup ends.
    /// Cancelling an already-started removal still tears down the old instance.
    /// A requested purge transfers to the cleanup task, which keeps the id
    /// reserved until module close and home deletion both finish.
    ///
    /// Cancellation is cooperative: a tool already executing when the agent
    /// is removed, or a sub-agent it detached, may finish after this returns.
    pub fn remove_agent(&self, id: &str) -> RemoveAgent<'_> {
        RemoveAgent {
            runtime: self,
            id: id.to_string(),
            purge: false,
        }
    }

    async fn remove(&self, id: &str, purge: bool) -> Result<(), AgentError> {
        let inner = {
            let agents = self.agents.lock().unwrap_or_else(|e| e.into_inner());
            agents
                .get(id)
                .and_then(|weak| weak.upgrade())
                .ok_or_else(|| AgentError::UnknownId(id.to_string()))?
        };
        if !inner
            .lifecycle
            .mark_removed_with("agent_removed", || inner.deny_approvals("agent_removed"))
        {
            return Err(AgentError::UnknownId(id.to_string()));
        }
        let mut removal = RemovalSlot {
            runtime: self,
            inner,
            purge,
            mcp_cleanup_confirmed: false,
        };
        let inner = &removal.inner;
        log::debug!("[embed][runtime] removing agent id={id} purge={purge}");
        if !inner.lifecycle.wait_idle(REMOVE_IDLE_WAIT).await {
            log::warn!(
                "[embed][runtime] agent id={id} still had turns unwinding after {}s",
                REMOVE_IDLE_WAIT.as_secs()
            );
        }
        removal.mcp_cleanup_confirmed =
            openhuman_core::mcp::host::close_agent(&inner.config.workspace_dir, &inner.id).await;
        if !removal.mcp_cleanup_confirmed {
            return Err(AgentError::CleanupPending);
        }
        inner.teardown("agent_removed");
        removal.purge_home()?;
        log::debug!("[embed][runtime] agent removed id={id}");
        Ok(())
    }
}
