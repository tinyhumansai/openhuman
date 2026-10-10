//! Lazy ownership of a module-side trigger archive; no files are opened here.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tinyconnectors_bus::{
    ArchiveHandle, OpenArchiveRequest, ReadArchiveRequest, RecordTriggerRequest,
};
use tokio::sync::Mutex;

use super::module_client::methods;
use super::types::{
    ComposioTriggerEvent, ComposioTriggerHistoryEntry, ComposioTriggerHistoryResult,
};
use crate::config::Config;
use crate::modules::client::ModuleClient;

static GLOBAL_TRIGGER_HISTORY: OnceLock<Arc<TriggerHistory>> = OnceLock::new();

struct Lease {
    workspace: PathBuf,
    client: ModuleClient,
    handle: ArchiveHandle,
}

/// Host lifecycle owner of one opaque archive lease. The module owns its files.
pub struct TriggerHistory {
    initial_workspace: PathBuf,
    lease: Arc<Mutex<Option<Lease>>>,
    #[cfg(all(test, feature = "modules"))]
    fixture: Option<ModuleClient>,
}

impl TriggerHistory {
    fn client(&self, config: &Config) -> ModuleClient {
        #[cfg(all(test, feature = "modules"))]
        if let Some(client) = self.fixture.as_ref() {
            return client.clone();
        }
        ModuleClient::new(config.clone())
    }

    async fn ensure_lease(
        config: &Config,
        lease: &mut Option<Lease>,
        client: ModuleClient,
    ) -> Result<ArchiveHandle, String> {
        if lease
            .as_ref()
            .is_some_and(|current| current.workspace != config.workspace_dir)
        {
            Self::release(lease).await?;
        }
        if let Some(current) = lease.as_ref() {
            return Ok(current.handle.clone());
        }
        let state_dir = config.workspace_dir.join("state");
        let state_dir = state_dir
            .to_str()
            .ok_or_else(|| "trigger archive requires a UTF-8 state directory".to_owned())?;
        let handle = client
            .call(
                "tinyconnectors",
                methods::OPEN_ARCHIVE,
                (OpenArchiveRequest {
                    state_dir: state_dir.to_owned(),
                },),
            )
            .await
            .map_err(|error| error.to_string())?;
        *lease = Some(Lease {
            workspace: config.workspace_dir.clone(),
            client,
            handle,
        });
        Ok(lease.as_ref().expect("lease installed").handle.clone())
    }

    async fn acquire(
        &self,
        config: &Config,
    ) -> Result<tokio::sync::OwnedMutexGuard<Option<Lease>>, String> {
        let lease = Arc::clone(&self.lease);
        let config = config.clone();
        let client = self.client(&config);
        // Keep ownership installation running if the caller is cancelled while
        // OpenArchive completes. A later close can still release the handle.
        tokio::spawn(async move {
            let mut guard = lease.lock_owned().await;
            Self::ensure_lease(&config, &mut guard, client).await?;
            Ok(guard)
        })
        .await
        .map_err(|_| "trigger archive ownership task failed".to_owned())?
    }

    async fn release(lease: &mut Option<Lease>) -> Result<(), String> {
        if let Some(current) = lease.as_ref() {
            current
                .client
                .call::<()>(
                    "tinyconnectors",
                    methods::CLOSE_ARCHIVE,
                    (current.handle.clone(),),
                )
                .await
                .map_err(|error| error.to_string())?;
            *lease = None;
        }
        Ok(())
    }

    /// Archive one delivered event, serialized with reads and scope changes.
    pub async fn record(
        &self,
        config: &Config,
        event: ComposioTriggerEvent,
    ) -> Result<ComposioTriggerHistoryEntry, String> {
        let lease = self.acquire(config).await?;
        let handle = lease.as_ref().expect("lease installed").handle.clone();
        lease
            .as_ref()
            .expect("lease installed")
            .client
            .call(
                "tinyconnectors",
                methods::RECORD_TRIGGER,
                (RecordTriggerRequest { handle, event },),
            )
            .await
            .map_err(|error| error.to_string())
    }

    /// Read persisted history without changing the user's connector route.
    pub async fn read(
        &self,
        config: &Config,
        limit: Option<usize>,
    ) -> Result<ComposioTriggerHistoryResult, String> {
        let lease = self.acquire(config).await?;
        let handle = lease.as_ref().expect("lease installed").handle.clone();
        lease
            .as_ref()
            .expect("lease installed")
            .client
            .call(
                "tinyconnectors",
                methods::READ_ARCHIVE,
                (ReadArchiveRequest { handle, limit },),
            )
            .await
            .map_err(|error| error.to_string())
    }

    /// Release the module resource while retaining all persisted documents.
    pub async fn close(&self) -> Result<(), String> {
        let lease = Arc::clone(&self.lease);
        tokio::spawn(async move { Self::release(&mut *lease.lock_owned().await).await })
            .await
            .map_err(|_| "trigger archive release task failed".to_owned())?
    }
}

/// Install the lifecycle owner without downloading or loading a module.
pub fn init_global(workspace_dir: PathBuf) -> Result<(), String> {
    let installed = GLOBAL_TRIGGER_HISTORY.get_or_init(|| {
        Arc::new(TriggerHistory {
            initial_workspace: workspace_dir.clone(),
            lease: Arc::new(Mutex::new(None)),
            #[cfg(all(test, feature = "modules"))]
            fixture: None,
        })
    });
    if installed.initial_workspace == workspace_dir {
        Ok(())
    } else {
        Err("trigger archive owner already initialized for another workspace".to_owned())
    }
}

/// The process-wide lifecycle owner, if initialized at startup.
pub fn global() -> Option<Arc<TriggerHistory>> {
    GLOBAL_TRIGGER_HISTORY.get().cloned()
}

/// Close an existing lease on sign-out or runtime shutdown; never load at exit.
pub(crate) async fn close_global() -> Result<(), String> {
    match global() {
        Some(store) => store.close().await,
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "trigger_history_tests.rs"]
mod tests;
