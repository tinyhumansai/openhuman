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
}

impl RemovalSlot<'_> {
    fn purge_home(&mut self) -> Result<(), AgentError> {
        if !std::mem::take(&mut self.purge) {
            return Ok(());
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
        self.inner.teardown("agent_removed");
        if let Err(error) = self.purge_home() {
            log::warn!("[embed][runtime] cancelled removal purge failed: {error}");
        }
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
    /// state slots and MCP host are dropped and its context deregistered, so
    /// its cron jobs stay dormant. The id is free for reuse once cleanup ends.
    /// Cancelling an already-started removal still tears down the old instance
    /// and performs a requested purge before releasing its id.
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
        };
        let inner = &removal.inner;
        log::debug!("[embed][runtime] removing agent id={id} purge={purge}");
        if !inner.lifecycle.wait_idle(REMOVE_IDLE_WAIT).await {
            log::warn!(
                "[embed][runtime] agent id={id} still had turns unwinding after {}s",
                REMOVE_IDLE_WAIT.as_secs()
            );
        }
        inner.teardown("agent_removed");
        removal.purge_home()?;
        log::debug!("[embed][runtime] agent removed id={id}");
        Ok(())
    }
}
