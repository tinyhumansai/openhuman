//! A driver-backed session store whose backend can be swapped in place.
//!
//! The default storage backend lives in the workspace
//! (`sqlite:<workspace>/storage`), and the desktop rebinds its workspace when a
//! different user signs in. The session store is installed once, process-wide
//! (a detached turn still writing across a rebind must keep landing in a
//! store, not in a no-provider fallback), so it cannot be replaced by a new
//! provider object; it forwards to a [`DriverSessionStores`] that
//! [`rebind_all`] points at the new workspace's backend.
//!
//! Handles already handed out keep the backend they were opened on, so a turn
//! in flight when the user switches finishes against the old workspace.

use std::sync::{Arc, LazyLock, Mutex, PoisonError, RwLock, Weak};

use tinyagents_session::port::{AgentStores, SessionStoreProvider};
use tinyagents_session::DriverSessionStores;
use tinystoragedrivers::{Blocking, StorageBackend, StorageError};

/// The live [`ReboundSessionStores`] of one owner, so a rebind reaches the one
/// a host installed however it was installed (process-wide, per runtime, per
/// task).
///
/// The process has one ([`rebind_all`] moves every store in it). A registry of
/// its own scopes a rebind to the stores created in it, which is how a test
/// exercises rebinding without moving the stores of tests running beside it.
#[derive(Default)]
pub struct StoreRegistry {
    live: Mutex<Vec<Weak<ReboundSessionStores>>>,
}

impl StoreRegistry {
    fn register(&self, stores: &Arc<ReboundSessionStores>) {
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        live.retain(|weak| weak.strong_count() > 0);
        live.push(Arc::downgrade(stores));
    }

    fn live(&self) -> Vec<Arc<ReboundSessionStores>> {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter_map(Weak::upgrade)
            .collect()
    }

    /// Rebinds every live store of this registry to `backend`; returns how many.
    pub fn rebind(&self, backend: &Arc<dyn StorageBackend>) -> usize {
        let live = self.live();
        for stores in &live {
            stores.rebind(Arc::clone(backend));
        }
        tracing::debug!(
            target: "openhuman::storage",
            rebound = live.len(),
            "[session_store] registry rebind finished"
        );
        live.len()
    }
}

/// The process's registry, the one [`ReboundSessionStores::new`] joins.
static PROCESS: LazyLock<StoreRegistry> = LazyLock::new(StoreRegistry::default);

/// [`DriverSessionStores`] over a backend [`rebind_all`] can replace.
pub struct ReboundSessionStores {
    inner: RwLock<Arc<DriverSessionStores>>,
    bridge: Blocking,
    recover_on_open: bool,
}

impl std::fmt::Debug for ReboundSessionStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReboundSessionStores")
            .field("destination", &self.current().destination_key())
            .finish()
    }
}

impl ReboundSessionStores {
    /// Session stores over `backend`; `recover_on_open` is
    /// [`DriverSessionStores::recover_on_open`].
    ///
    /// # Errors
    ///
    /// When the blocking bridge cannot start.
    pub fn new(
        backend: Arc<dyn StorageBackend>,
        recover_on_open: bool,
    ) -> Result<Arc<Self>, StorageError> {
        Self::new_in(&PROCESS, backend, recover_on_open)
    }

    /// [`Self::new`], joining `registry` instead of the process's, so only a
    /// rebind of that registry reaches it.
    ///
    /// # Errors
    ///
    /// When the blocking bridge cannot start.
    pub fn new_in(
        registry: &StoreRegistry,
        backend: Arc<dyn StorageBackend>,
        recover_on_open: bool,
    ) -> Result<Arc<Self>, StorageError> {
        let bridge = Blocking::new()?;
        let inner = DriverSessionStores::with_bridge(backend, bridge.clone())
            .recover_on_open(recover_on_open);
        let this = Arc::new(Self {
            inner: RwLock::new(Arc::new(inner)),
            bridge,
            recover_on_open,
        });
        registry.register(&this);
        Ok(this)
    }

    fn current(&self) -> Arc<DriverSessionStores> {
        Arc::clone(&self.inner.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Points the store at `backend`: agents opened from now on live there.
    pub fn rebind(&self, backend: Arc<dyn StorageBackend>) {
        let next = DriverSessionStores::with_bridge(backend, self.bridge.clone())
            .recover_on_open(self.recover_on_open);
        *self.inner.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(next);
        tracing::info!(
            target: "openhuman::storage",
            destination = ?self.current().destination_key(),
            "[session_store] rebound to a new storage backend"
        );
    }
}

/// Rebinds every live [`ReboundSessionStores`] of the process to `backend`;
/// returns how many.
pub fn rebind_all(backend: &Arc<dyn StorageBackend>) -> usize {
    PROCESS.rebind(backend)
}

impl SessionStoreProvider for ReboundSessionStores {
    fn for_agent(&self, agent_id: &str) -> AgentStores {
        self.current().for_agent(agent_id)
    }

    fn try_for_agent(&self, agent_id: &str) -> Result<AgentStores, StorageError> {
        SessionStoreProvider::try_for_agent(self.current().as_ref(), agent_id)
    }

    fn bind_actor(
        &self,
        backend: Arc<dyn StorageBackend>,
    ) -> Result<Option<Arc<dyn SessionStoreProvider>>, StorageError> {
        self.current().bind_actor(backend)
    }

    fn bind_actor_with_schema_admission(
        &self,
        backend: Arc<dyn StorageBackend>,
        admission: tinyagents_harness::store::SchemaAdmission,
    ) -> Result<Option<Arc<dyn SessionStoreProvider>>, StorageError> {
        self.current()
            .bind_actor_with_schema_admission(backend, admission)
    }

    fn run_ledger_for_agent(
        &self,
        agent_id: &str,
    ) -> tinyagents_session::run_ledger::LedgerResult<
        Option<Arc<dyn tinyagents_session::run_ledger::RunLedgerStore>>,
    > {
        self.current().run_ledger_for_agent(agent_id)
    }

    fn conversations_for_agent(
        &self,
        agent_id: &str,
    ) -> tinyagents_session::threads::ConversationResult<
        Option<Arc<dyn tinyagents_session::threads::ConversationPersistence>>,
    > {
        self.current().conversations_for_agent(agent_id)
    }

    fn recover(&self) -> anyhow::Result<()> {
        self.current().recover()
    }

    fn destination_key(&self) -> Option<String> {
        self.current().destination_key()
    }
}

#[cfg(test)]
#[path = "rebound_tests.rs"]
mod tests;
