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
    _runtime: &'a Runtime,
    inner: Arc<crate::agent::AgentInner>,
    purge: bool,
    finished: bool,
}

impl Drop for RemovalSlot<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.inner.teardown("agent_removed");
            self.inner.defer_cleanup(self.purge);
        }
    }
}

struct PendingRemoval<'a> {
    runtime: &'a Runtime,
    id: String,
    pending: super::PendingCleanup,
    finished: bool,
}

impl Drop for PendingRemoval<'_> {
    fn drop(&mut self) {
        if !self.finished {
            crate::agent::mcp_cleanup::defer_pending(
                Arc::clone(&self.runtime.agents),
                self.id.clone(),
                self.pending.clone(),
                self.runtime.guard(),
            );
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
    /// state slots are cleared, its original MCP object is closed, and its context deregistered, so
    /// its cron jobs stay dormant. The id is free for reuse once cleanup ends.
    /// Cancelling an already-started removal still tears down the old instance
    /// and performs a requested purge before releasing its id.
    /// A failed close returns [`AgentError::McpCleanup`] and leaves the id and
    /// purge intent reserved. Detached cleanup makes at most three attempts on
    /// the process executor. Call this method again to retry the captured owner,
    /// including after the last agent handle was dropped. Runtime drop does
    /// not execute the process signal's shutdown hooks.
    ///
    /// Each MCP attempt has a 30-second deadline; three detached attempts add
    /// at most 750 ms of backoff. Filesystem purge and executor scheduling are
    /// outside that wait bound. Failed or cancelled cleanup preserves an
    /// ephemeral workspace when the runtime drops and logs its retained path.
    /// That preservation is conservative for the rest of this runtime.
    ///
    /// The directory fence survives in this process if the original runtime's
    /// retry record is lost. A new runtime cannot recover that record through
    /// this method; there is no automatic durable retry or file reclamation.
    /// Retained state then needs explicit host cleanup.
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
        let pending = {
            let mut agents = self
                .agents
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            agents.pending.get_mut(id).map(|pending| {
                pending.purge |= purge;
                pending.clone()
            })
        };
        if let Some(pending) = pending {
            let mut retry = PendingRemoval {
                runtime: self,
                id: id.to_owned(),
                pending,
                finished: false,
            };
            if let Some(cleanup) = &retry.pending.cleanup {
                cleanup.finish().await.map_err(AgentError::McpCleanup)?;
            }
            crate::agent::mcp_cleanup::finish_removal(&self.agents, id, &retry.pending)?;
            retry.finished = true;
            return Ok(());
        }
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
            _runtime: self,
            inner,
            purge,
            finished: false,
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
        inner.reserve_cleanup(purge);
        inner.finish_mcp_cleanup().await?;
        inner.finish_removal(purge)?;
        removal.finished = true;
        log::debug!("[embed][runtime] agent removed id={id}");
        Ok(())
    }
}
