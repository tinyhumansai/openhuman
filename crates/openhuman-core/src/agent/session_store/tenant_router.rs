//! [`TenantSessionStores`]: one [`DriverSessionStores`] per tenant.
//!
//! TinyAgents' `DriverSessionStores` serves one tenant and derives
//! `tenant.agent(id)` itself. The host keeps its flat session keys
//! ([`crate::core::runtime::session_key`]: the agent id for the desktop
//! operator, `<profile>~<agent>` for a SaaS profile), so this provider decodes
//! the key into `(profile, agent)` and forwards to the provider of that
//! tenant (`Scope::local()` or `Scope::tenant(profile)`), created on first
//! use. Every scope a session writes is therefore `local/agent:<id>` or
//! `u:<profile>/agent:<id>`, inside the tenant fence.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use tinyagents_harness::store::SchemaAdmission;
use tinyagents_session::port::{AgentStores, SessionStoreProvider};
use tinyagents_session::DriverSessionStores;
use tinystoragedrivers::{Blocking, Scope, StorageBackend, StorageError};

/// Session stores over one backend, routed to a per-tenant provider.
pub struct TenantSessionStores {
    backend: Arc<dyn StorageBackend>,
    bridge: Blocking,
    recover_on_open: bool,
    admission: SchemaAdmission,
    tenants: Mutex<HashMap<Scope, Arc<DriverSessionStores>>>,
    /// Names this router in [`SessionStoreProvider::destination_key`].
    id: uuid::Uuid,
}

impl std::fmt::Debug for TenantSessionStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tenants = self.tenants.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("TenantSessionStores")
            .field("driver", &self.backend.driver())
            .field("tenants", &tenants.len())
            .finish_non_exhaustive()
    }
}

impl TenantSessionStores {
    /// Stores over `backend` on a shared `bridge`.
    pub fn with_bridge(
        backend: Arc<dyn StorageBackend>,
        bridge: Blocking,
        recover_on_open: bool,
    ) -> Self {
        Self::with_admission(backend, bridge, recover_on_open, SchemaAdmission::Declare)
    }

    fn with_admission(
        backend: Arc<dyn StorageBackend>,
        bridge: Blocking,
        recover_on_open: bool,
        admission: SchemaAdmission,
    ) -> Self {
        Self {
            backend,
            bridge,
            recover_on_open,
            admission,
            tenants: Mutex::new(HashMap::new()),
            id: uuid::Uuid::new_v4(),
        }
    }

    /// Stores over `backend` with a bridge of their own.
    ///
    /// # Errors
    ///
    /// When the blocking bridge cannot start.
    pub fn new(
        backend: Arc<dyn StorageBackend>,
        recover_on_open: bool,
    ) -> Result<Self, StorageError> {
        Ok(Self::with_bridge(
            backend,
            Blocking::new()?,
            recover_on_open,
        ))
    }

    /// The provider of the tenant `key` belongs to, and the agent id within it.
    fn route(&self, key: &str) -> (Arc<DriverSessionStores>, String) {
        let (profile, mut agent) = crate::core::runtime::split_session_key(key);
        if agent.is_empty() {
            agent = crate::storage::scope::DEFAULT_AGENT.to_owned();
        }
        let tenant = crate::storage::tenant_scope(profile.as_deref());
        let mut tenants = self.tenants.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(provider) = tenants.get(&tenant) {
            return (Arc::clone(provider), agent);
        }
        let provider = Arc::new(
            DriverSessionStores::with_bridge_and_schema_admission(
                Arc::clone(&self.backend),
                tenant.clone(),
                self.bridge.clone(),
                self.admission,
            )
            .recover_on_open(self.recover_on_open),
        );
        tracing::debug!(
            target: "openhuman::storage",
            tenant = tenant.as_str(),
            "[session_store] opened the tenant's session provider"
        );
        tenants.insert(tenant, Arc::clone(&provider));
        (provider, agent)
    }

    fn providers(&self) -> Vec<Arc<DriverSessionStores>> {
        self.tenants
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }
}

impl SessionStoreProvider for TenantSessionStores {
    fn for_agent(&self, agent_id: &str) -> AgentStores {
        let (provider, agent) = self.route(agent_id);
        provider.for_agent(&agent)
    }

    fn try_for_agent(&self, agent_id: &str) -> Result<AgentStores, StorageError> {
        let (provider, agent) = self.route(agent_id);
        provider.try_for_agent(&agent)
    }

    fn bind_actor(
        &self,
        backend: Arc<dyn StorageBackend>,
    ) -> Result<Option<Arc<dyn SessionStoreProvider>>, StorageError> {
        self.bind_actor_with_schema_admission(backend, self.admission)
    }

    fn bind_actor_with_schema_admission(
        &self,
        backend: Arc<dyn StorageBackend>,
        admission: SchemaAdmission,
    ) -> Result<Option<Arc<dyn SessionStoreProvider>>, StorageError> {
        Ok(Some(Arc::new(Self::with_admission(
            backend,
            self.bridge.clone(),
            self.recover_on_open,
            admission,
        ))))
    }

    fn run_ledger_for_agent(
        &self,
        agent_id: &str,
    ) -> tinyagents_session::run_ledger::LedgerResult<
        Option<Arc<dyn tinyagents_session::run_ledger::RunLedgerStore>>,
    > {
        let (provider, agent) = self.route(agent_id);
        provider.run_ledger_for_agent(&agent)
    }

    fn conversations_for_agent(
        &self,
        agent_id: &str,
    ) -> tinyagents_session::threads::ConversationResult<
        Option<Arc<dyn tinyagents_session::threads::ConversationPersistence>>,
    > {
        let (provider, agent) = self.route(agent_id);
        provider.conversations_for_agent(&agent)
    }

    fn recover(&self) -> anyhow::Result<()> {
        for provider in self.providers() {
            provider.recover()?;
        }
        Ok(())
    }

    fn destination_key(&self) -> Option<String> {
        Some(format!("{}://{}", self.backend.driver(), self.id))
    }
}

#[cfg(test)]
#[path = "tenant_router_tests.rs"]
mod tests;
