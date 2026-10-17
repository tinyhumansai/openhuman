//! Configured-client cache admission, retirement and acknowledged cleanup.
//!
//! The cache, cleanup reservations and open/close receipt machine live in
//! `tinymcp_bus::client_lifecycle`; this adapter supplies the host policy
//! (module gate, removed-agent context, shutdown) and performs the bus calls.

use super::*;
use std::sync::OnceLock;
use tinymcp_bus::client_lifecycle::{AdmitError, CleanupReservation, ClientTable};

static CLIENTS: OnceLock<ClientTable<McpConfiguredClient>> = OnceLock::new();

pub(super) fn table() -> &'static ClientTable<McpConfiguredClient> {
    CLIENTS.get_or_init(ClientTable::new)
}

impl McpConfiguredClient {
    /// The process-owned configured object for this workspace. Entries remain
    /// reachable until `close` confirms cleanup; workspace/config switching is
    /// serialized by the client itself.
    pub(crate) fn for_config(config: &Config) -> Result<Arc<Self>, ModuleCallError> {
        if SHUTTING_DOWN.load(Ordering::Acquire) {
            crate::modules::report::unavailable(MODULE);
            return Err(ModuleCallError::Unavailable);
        }
        if !config.modules.enabled {
            crate::modules::report::unavailable(MODULE);
            return Err(ModuleCallError::Unavailable);
        }
        if current_agent_context_is_removed() {
            crate::modules::report::unavailable(MODULE);
            return Err(ModuleCallError::Unavailable);
        }
        let data_dir = crate::core::runtime::agent_scope_dir(config);
        let key = client_key(&data_dir);
        // The table rechecks removal and pending cleanup under its lock, so a
        // reservation installed after the first check cannot be bypassed.
        table()
            .admit(&key, current_agent_context_is_removed, || {
                Self::new(&data_dir, config).map(Arc::new)
            })
            .map_err(|error| match error {
                AdmitError::Create(error) => error,
                _ => {
                    crate::modules::report::unavailable(MODULE);
                    ModuleCallError::Unavailable
                }
            })
    }

    /// A snapshot of configured workspaces currently retained by the host.
    /// The supervisor only drains open objects; it never creates a workspace.
    pub(crate) fn open_clients() -> Vec<(PathBuf, Arc<Self>)> {
        table().snapshot()
    }

    /// Drain observations from the module-owned supervisor for an open child.
    /// An object that is opening or closing is skipped and will be revisited
    /// on the next host poll.
    pub(crate) async fn drain_supervisor_events(
        &self,
        limit: usize,
    ) -> Result<Option<SupervisorBatch>, ModuleCallError> {
        let state = self.state.lock().await;
        if SHUTTING_DOWN.load(Ordering::Acquire) || self.retiring.load(Ordering::Acquire) {
            return Err(ModuleCallError::Unavailable);
        }
        let Some(object) = state.open_object() else {
            return Ok(None);
        };
        let args = serde_json::to_value((limit,)).map_err(|_| ModuleCallError::ModuleFault)?;
        let reply = self
            .port
            .call_object(
                &object.object_path,
                names::methods::DRAIN_SUPERVISOR_EVENTS,
                args,
                false,
            )
            .await?;
        serde_json::from_value(reply)
            .map(Some)
            .map_err(|_| ModuleCallError::ModuleFault)
    }

    /// Register this runtime's cleanup. The flag is reset by the drained hook,
    /// so a later embedded-runtime start can register a fresh one.
    pub(crate) fn register_shutdown_hook() {
        if SHUTDOWN_REGISTERED.swap(true, Ordering::AcqRel) {
            return;
        }
        SHUTTING_DOWN.store(false, Ordering::Release);
        crate::core::shutdown::register(|| async {
            SHUTTING_DOWN.store(true, Ordering::Release);
            for (data_dir, client) in table().snapshot() {
                if client.retiring.load(Ordering::Acquire) {
                    let reservation = table().reservation_for(&data_dir);
                    Self::close_retired_client(data_dir, client, reservation).await;
                } else if let Err(error) = client.close().await {
                    tracing::warn!(?error, "[mcp] workspace cleanup is still pending");
                }
            }
            table().retry_orphaned();
            SHUTDOWN_REGISTERED.store(false, Ordering::Release);
        });
    }

    pub(crate) fn new(
        data_dir: &std::path::Path,
        config: &Config,
    ) -> Result<Self, ModuleCallError> {
        let data_dir = data_dir
            .to_str()
            .ok_or(ModuleCallError::IncompatibleContract)?
            .to_owned();
        Ok(Self::with_port(
            data_dir,
            Box::new(TinyMcpPort {
                root: crate::modules::client(config.clone()),
            }),
        ))
    }

    #[cfg(test)]
    pub(crate) fn install_test_port(
        config: &Config,
        port: Box<dyn McpModulePort>,
    ) -> Result<Arc<Self>, ModuleCallError> {
        let data_dir = crate::core::runtime::agent_scope_dir(config);
        let client = Arc::new(Self::with_port(
            data_dir
                .to_str()
                .ok_or(ModuleCallError::IncompatibleContract)?
                .to_owned(),
            port,
        ));
        table().insert(client_key(&data_dir), Arc::clone(&client));
        Ok(client)
    }

    pub(super) fn with_port(data_dir: String, port: Box<dyn McpModulePort>) -> Self {
        Self {
            data_dir,
            port,
            state: tokio::sync::Mutex::new(ConfiguredLifecycle::new()),
            retiring: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_port(data_dir: String, port: Box<dyn McpModulePort>) -> Self {
        Self::with_port(data_dir, port)
    }

    /// Close an already-open agent-scoped client without creating one. A
    /// failed close remains in the cache, marked unavailable, so shutdown or
    /// a later cleanup attempt can retry the module's retained transition.
    pub(crate) async fn close_agent_data_dir(data_dir: &std::path::Path) -> bool {
        let key = client_key(data_dir);
        let reservation = table().reservation_for(&key);
        let Some(client) = table().retire(&key) else {
            return table().finish_cleanup(&key, None, &reservation);
        };
        if let Some(cleaned) = Self::finish_if_closed(&key, &client, &reservation) {
            return cleaned;
        }
        Self::close_retired_client(key, client, reservation).await
    }

    /// Marks the exact cached client unavailable before scheduling cleanup.
    /// If no Tokio runtime is active, the retired client remains cached for the
    /// registered shutdown hook to close later.
    pub(crate) fn schedule_agent_close_data_dir(data_dir: &std::path::Path) {
        let key = client_key(data_dir);
        let reservation = table().reservation_for(&key);
        let Some(client) = table().retire(&key) else {
            table().finish_cleanup(&key, None, &reservation);
            return;
        };
        if Self::finish_if_closed(&key, &client, &reservation).is_some() {
            return;
        }
        Self::spawn_close(key, client, reservation, "agent client close");
    }

    /// Retires the exact scoped client and runs `after_close` only after the
    /// module acknowledges its close. The path remains unavailable until the
    /// callback succeeds, so a replacement cannot reuse files being purged.
    pub(crate) fn schedule_agent_close_and_cleanup(
        data_dir: &std::path::Path,
        after_close: CleanupAction,
    ) {
        let key = client_key(data_dir);
        let reservation = table().reservation_with_action(&key, after_close);
        let Some(client) = table().retire(&key) else {
            if !table().finish_cleanup(&key, None, &reservation) {
                tracing::warn!("[mcp] agent cleanup remains pending for retry");
            }
            return;
        };
        if Self::finish_if_closed(&key, &client, &reservation).is_some() {
            return;
        }
        Self::spawn_close(key, client, reservation, "agent close and cleanup");
    }

    fn spawn_close(
        key: PathBuf,
        client: Arc<Self>,
        reservation: Arc<CleanupReservation>,
        what: &'static str,
    ) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            if !Self::close_retired_client(key, client, reservation).await {
                tracing::warn!("[mcp] {what} remains pending for retry");
            }
        });
    }

    pub(crate) fn agent_close_pending(data_dir: &std::path::Path) -> bool {
        table().close_pending(&client_key(data_dir))
    }

    fn finish_if_closed(
        data_dir: &std::path::Path,
        client: &Arc<Self>,
        reservation: &Arc<CleanupReservation>,
    ) -> Option<bool> {
        table().finish_if_closed(data_dir, client, reservation, || {
            client.state.try_lock().is_ok_and(|state| state.is_closed())
        })
    }

    pub(super) async fn close_retired_client(
        data_dir: PathBuf,
        client: Arc<Self>,
        reservation: Arc<CleanupReservation>,
    ) -> bool {
        match client.close().await {
            Ok(()) => table().finish_cleanup(&data_dir, Some(&client), &reservation),
            Err(error) => {
                tracing::warn!(?error, "[mcp] agent client close remains pending");
                false
            }
        }
    }
}

/// The table key for an agent data directory, stable across spellings of the
/// same folder (symlinks, `..`, case on case-insensitive volumes). The longest
/// existing ancestor is canonicalized and the rest re-appended, so a directory
/// that a purge has already removed keeps the key it was admitted under.
fn client_key(data_dir: &std::path::Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut ancestor = data_dir;
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
            _ => return data_dir.to_path_buf(),
        }
    }
}
