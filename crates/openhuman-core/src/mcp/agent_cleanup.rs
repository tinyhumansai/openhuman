//! Captured cleanup of an agent's existing configured module object.
use super::*;

/// A retired agent's original MCP owner and its retryable close receipts.
/// Capturing this handle never creates a client or loads a module.
#[derive(Clone)]
pub struct AgentMcpCleanup {
    directory: PathBuf,
    client: Arc<McpConfiguredClient>,
}

/// Retire the existing configured client for this exact agent directory.
/// An absent entry requires no cleanup and leaves the module loader untouched.
pub fn take_agent_cleanup(workspace: &std::path::Path, agent_id: &str) -> Option<AgentMcpCleanup> {
    let directory = super::lifecycle::client_key(&workspace.join("agents").join(agent_id));
    let client = super::lifecycle::table().retire(&directory)?;
    Some(AgentMcpCleanup { directory, client })
}

impl AgentMcpCleanup {
    /// Close the original object and evict it only after confirmed cleanup.
    /// The whole attempt, including lock acquisition and bus calls, has a
    /// 30-second deadline. Timeout returns `TransportFailed`; cancellation or
    /// error retains the same client and transition receipt for explicit retry.
    pub async fn finish(&self) -> Result<(), ModuleCallError> {
        tokio::time::timeout(TRANSITION_TIMEOUT, self.client.close())
            .await
            .map_err(|_| ModuleCallError::TransportFailed)??;
        self.evict();
        Ok(())
    }

    /// Finish an unused client without starting an asynchronous executor.
    /// An accepted call holding the lifecycle lock requires asynchronous drain.
    pub fn finish_if_idle(&self) -> bool {
        let Ok(state) = self.client.state.try_lock() else {
            return false;
        };
        if !state.is_closed() {
            return false;
        }
        self.evict();
        true
    }

    fn evict(&self) {
        super::lifecycle::table().evict(&self.directory, &self.client);
    }
}
