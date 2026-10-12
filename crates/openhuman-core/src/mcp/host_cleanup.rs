//! Process-local host cleanup fencing, also used when MCP is compiled out.
use crate::modules::client::ModuleCallError;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};

pub(super) type CleanupAction = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Default)]
pub(super) struct CleanupReservation {
    pub(super) after_close: Mutex<Option<CleanupAction>>,
    pub(super) host_completion_pending: AtomicBool,
}

pub(super) static CLEANUP_RESERVATIONS: OnceLock<Mutex<HashMap<PathBuf, Arc<CleanupReservation>>>> =
    OnceLock::new();

#[cfg(not(feature = "mcp"))]
pub(super) fn directory_pending(workspace: &std::path::Path, agent_id: &str) -> bool {
    CLEANUP_RESERVATIONS.get().is_some_and(|reservations| {
        reservations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&workspace.join("agents").join(agent_id))
    })
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
        crate::modules::host::spawn_cleanup(future).map_err(|_| ModuleCallError::TransportFailed)
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
    let directory = workspace.join("agents").join(agent_id);
    let mut reservations = CLEANUP_RESERVATIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let reservation = Arc::clone(
        reservations
            .entry(directory.clone())
            .or_insert_with(|| Arc::new(CleanupReservation::default())),
    );
    reservation
        .host_completion_pending
        .store(true, Ordering::Release);
    AgentDirectoryCleanup {
        directory,
        reservation,
    }
}

impl AgentDirectoryCleanup {
    /// Release this exact reservation after module close and host purge succeed.
    /// An independently registered canonical callback keeps its own reservation.
    pub fn complete(&self) {
        let after_close = self
            .reservation
            .after_close
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut reservations = CLEANUP_RESERVATIONS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if reservations
            .get(&self.directory)
            .is_some_and(|current| Arc::ptr_eq(current, &self.reservation))
        {
            self.reservation
                .host_completion_pending
                .store(false, Ordering::Release);
            if after_close.is_none() {
                reservations.remove(&self.directory);
            }
        }
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
