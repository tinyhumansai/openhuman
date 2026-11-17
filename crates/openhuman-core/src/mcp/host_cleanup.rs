//! Process-local host cleanup fencing, also used when MCP is compiled out.
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use tinybus::module::ModuleCallError;
use tinymcp_bus::client_lifecycle::{CleanupReservation, ClientTable};

#[cfg(feature = "mcp")]
type CleanupClient = super::module_client::McpConfiguredClient;
#[cfg(not(feature = "mcp"))]
pub(super) struct CleanupClient;
#[cfg(not(feature = "mcp"))]
impl tinymcp_bus::client_lifecycle::Retire for CleanupClient {
    fn mark_retiring(&self) {}
    fn is_retiring(&self) -> bool {
        false
    }
}
static CLIENTS: OnceLock<ClientTable<CleanupClient>> = OnceLock::new();
pub(super) fn table() -> &'static ClientTable<CleanupClient> {
    CLIENTS.get_or_init(ClientTable::new)
}

pub(super) fn directory_key(directory: &std::path::Path) -> PathBuf {
    #[cfg(feature = "mcp")]
    {
        super::module_client::lifecycle::client_key(directory)
    }
    #[cfg(not(feature = "mcp"))]
    {
        let mut missing = Vec::new();
        let mut ancestor = directory;
        loop {
            if let Ok(real) = ancestor.canonicalize() {
                return missing
                    .iter()
                    .rev()
                    .fold(real, |path, part| path.join(part));
            }
            match (ancestor.parent(), ancestor.file_name()) {
                (Some(parent), Some(name)) => {
                    missing.push(name.to_owned());
                    ancestor = parent;
                }
                _ => return directory.to_path_buf(),
            }
        }
    }
}

pub(super) fn directory_pending(workspace: &std::path::Path, agent_id: &str) -> bool {
    table().close_pending(&directory_key(&workspace.join("agents").join(agent_id)))
}

/// Run captured cleanup on the process module executor when modules are enabled.
/// Without modules, the caller must keep its current Tokio runtime alive.
/// This creates no configured client and loads no MCP artifact.
pub fn spawn_agent_cleanup<F>(
    future: F,
) -> Result<tokio::task::JoinHandle<F::Output>, ModuleCallError>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    #[cfg(feature = "modules")]
    {
        crate::modules::runtime::spawn_cleanup(future).map_err(|_| ModuleCallError::TransportFailed)
    }
    #[cfg(not(feature = "modules"))]
    {
        tokio::runtime::Handle::try_current()
            .map(|handle| handle.spawn(future))
            .map_err(|_| ModuleCallError::Unavailable)
    }
}

/// A process-local directory reservation with no runtime guard or callback.
/// Dropping the token retains the fence; only confirmed host completion clears it.
#[derive(Clone)]
pub struct AgentDirectoryCleanup {
    directory: PathBuf,
    reservation: Arc<CleanupReservation>,
}

/// Reserve an agent's directory until host-owned cleanup succeeds.
/// This never creates a client or loads a module.
pub fn reserve_agent_directory(
    workspace: &std::path::Path,
    agent_id: &str,
) -> AgentDirectoryCleanup {
    let directory = directory_key(&workspace.join("agents").join(agent_id));
    let reservation = table().reserve_host_completion(&directory);
    AgentDirectoryCleanup {
        directory,
        reservation,
    }
}

impl AgentDirectoryCleanup {
    /// Release this exact reservation after module close and host purge succeed.
    /// An independently registered canonical callback keeps its own reservation.
    pub fn complete(&self) {
        table().complete_host_cleanup(&self.directory, &self.reservation);
    }
}

/// An opaque cleanup handle for a build with no MCP client capability.
/// Such a build cannot create the handle or hold MCP resources.
#[cfg(not(feature = "mcp"))]
#[derive(Clone)]
pub struct AgentMcpCleanup {
    _unconstructible: (),
}

#[cfg(not(feature = "mcp"))]
impl AgentMcpCleanup {
    /// There are no MCP resources in this compiled-out capability.
    pub async fn finish(&self) -> Result<(), ModuleCallError> {
        Ok(())
    }

    /// There are no MCP resources to drain or retire.
    pub fn finish_if_idle(&self) -> bool {
        true
    }
}

/// A build without MCP cannot contain a configured MCP client.
#[cfg(not(feature = "mcp"))]
pub fn take_agent_cleanup(_workspace: &std::path::Path, _id: &str) -> Option<AgentMcpCleanup> {
    None
}
