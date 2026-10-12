//! Retiring one captured agent owner before its id or home is reusable.
use super::*;
use crate::runtime::PendingCleanup;
use std::sync::{atomic::Ordering, Weak};

pub(crate) fn finish_removal(
    registry: &crate::runtime::AgentMap,
    id: &str,
    pending: &PendingCleanup,
) -> Result<(), AgentError> {
    let mut agents = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !agents
        .get(id)
        .is_some_and(|current| Weak::ptr_eq(current, &pending.instance))
    {
        return Ok(());
    }
    let purge = pending.purge || agents.pending.get(id).is_some_and(|current| current.purge);
    // Keep the id reserved throughout purge; never touch a successor's home.
    if purge && pending.home.exists() {
        std::fs::remove_dir_all(&pending.home).map_err(|source| AgentError::Workspace {
            what: "delete the removed agent's home",
            source,
        })?;
    }
    pending.directory.complete();
    agents.remove(id);
    agents.pending.remove(id);
    Ok(())
}

/// Run at most three retries on the process module executor. The future owns
/// the workspace guard through at most 90 seconds of polled MCP cleanup and
/// 750 ms of backoff, plus synchronous filesystem purge. Purge and executor
/// scheduling are outside the MCP wait bound. Retained records contain no
/// guard; the last guard keeps runtime admission reserved through completion.
pub(crate) fn defer_pending(
    registry: Arc<crate::runtime::AgentMap>,
    id: String,
    pending: PendingCleanup,
    guard: Arc<crate::runtime::CoreGuard>,
) {
    if pending
        .cleanup
        .as_ref()
        .is_none_or(|cleanup| cleanup.finish_if_idle())
    {
        if let Err(error) = finish_removal(&registry, &id, &pending) {
            guard.preserve_ephemeral_workspace();
            log::warn!("[embed][agent] removed home cleanup remains pending: {error}");
        }
        return;
    }
    if pending.retry_running.swap(true, Ordering::AcqRel) {
        return;
    }
    let running = Arc::clone(&pending.retry_running);
    let executor_failed = Arc::clone(&running);
    let pending_for_completion = pending.clone();
    // Construct the RAII owner before scheduling, so rejection or cancellation
    // of an unpolled future cannot delete the unconfirmed workspace.
    let cleanup = retry_cleanup(
        guard,
        move || {
            let cleanup = pending.cleanup.clone();
            async move {
                match cleanup {
                    Some(cleanup) => cleanup.finish().await.map_err(AgentError::McpCleanup),
                    None => Ok(()),
                }
            }
        },
        {
            let registry = Arc::clone(&registry);
            let pending = pending_for_completion;
            move || finish_removal(&registry, &id, &pending)
        },
    );
    let future = async move {
        cleanup.await;
        // Retained metadata has no runtime guard. A live Runtime can explicitly
        // retry; after it is gone, retained files require manual reclamation.
        running.store(false, Ordering::Release);
    };
    if let Err(error) = openhuman_core::mcp::host::spawn_agent_cleanup(future) {
        executor_failed.store(false, Ordering::Release);
        log::warn!("[embed][agent] cleanup executor unavailable: {error}");
    }
}

struct CleanupWorkspaceOwner {
    guard: Option<Arc<crate::runtime::CoreGuard>>,
    confirmed: bool,
}

impl Drop for CleanupWorkspaceOwner {
    fn drop(&mut self) {
        if !self.confirmed {
            if let Some(guard) = &self.guard {
                guard.preserve_ephemeral_workspace();
            }
        }
    }
}

fn retry_cleanup<F, Fut, C>(
    guard: Arc<crate::runtime::CoreGuard>,
    mut close: F,
    mut complete: C,
) -> impl std::future::Future<Output = ()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), AgentError>>,
    C: FnMut() -> Result<(), AgentError>,
{
    let mut owner = CleanupWorkspaceOwner {
        guard: Some(guard),
        confirmed: false,
    };
    async move {
        for attempt in 0..3 {
            let result = close()
                .await
                .and_then(|()| complete_with_guard(&mut owner.guard, &mut complete));
            match result {
                Ok(()) => {
                    owner.confirmed = true;
                    drop(owner);
                    return;
                }
                Err(error) => {
                    log::warn!("[embed][agent] captured cleanup remains pending: {error}")
                }
            }
            if attempt < 2 {
                tokio::time::sleep(std::time::Duration::from_millis(250 * (1 << attempt))).await;
            }
        }
    }
}

fn complete_with_guard<T>(
    guard: &mut Option<Arc<crate::runtime::CoreGuard>>,
    complete: impl FnOnce() -> T,
) -> T {
    // Releasing the last guard permits a new runtime with another agent map
    // to reopen this workspace. Keep admission reserved through the callback.
    let _guard = guard.as_ref();
    complete()
}

impl AgentInner {
    pub(crate) fn reserve_cleanup(&self, purge: bool) -> Option<PendingCleanup> {
        let cleanup = self
            .mcp_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut agents = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let instance = agents
            .get(&self.id)
            .filter(|weak| std::ptr::eq(weak.as_ptr(), self))
            .cloned()?;
        let pending = agents
            .pending
            .entry(self.id.clone())
            .or_insert_with(|| PendingCleanup {
                cleanup,
                directory: openhuman_core::mcp::host::reserve_agent_directory(
                    &self.config.workspace_dir,
                    &self.id,
                ),
                instance,
                home: self.layout.home.clone(),
                purge,
                retry_running: Default::default(),
            });
        pending.purge |= purge;
        Some(pending.clone())
    }

    pub(crate) async fn finish_mcp_cleanup(&self) -> Result<(), AgentError> {
        let cleanup = self
            .mcp_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(cleanup) = cleanup {
            cleanup.finish().await.map_err(AgentError::McpCleanup)?;
        }
        Ok(())
    }

    pub(crate) fn finish_removal(&self, purge: bool) -> Result<(), AgentError> {
        if let Some(pending) = self.reserve_cleanup(purge) {
            finish_removal(&self.registry, &self.id, &pending)?;
        }
        Ok(())
    }

    pub(crate) fn defer_cleanup(&self, purge: bool) {
        if let Some(pending) = self.reserve_cleanup(purge) {
            defer_pending(
                Arc::clone(&self.registry),
                self.id.clone(),
                pending,
                Arc::clone(&self._runtime_guard),
            );
        }
    }
}

#[cfg(test)]
#[path = "mcp_cleanup_tests.rs"]
mod tests;
