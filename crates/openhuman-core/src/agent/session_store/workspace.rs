//! The session store for code that has a workspace but no store of its own.
//!
//! Hosts install a [`SessionStoreProvider`] over the storage backend
//! ([`super::install`]). Code that was written against the workspace layout
//! (the run ledger, goals, todos, the turn journal, reapers) asks here: the
//! installed provider when there is one, else a provider over the same default
//! a host would have installed for that workspace
//! ([`crate::storage::local::backend_for`]). The answer is always the storage
//! ports; there is no file layout to fall back to any more.
//!
//! The tenant a call acts for picks the agent's stores
//! ([`crate::core::runtime::session_key`]): the desktop operator's is
//! [`DEFAULT_AGENT`](super::DEFAULT_AGENT), an embedded agent's its own id, a
//! SaaS profile's `<profile>~<agent>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use tinyagents_harness::multimodal::BlobAttachmentStash;
use tinyagents_session::port::{AgentStores, SessionStoreProvider, TurnStates};
use tinyagents_session::run_ledger::{LedgerError, LedgerResult, RunLedgerStore};
use tinyagents_session::DriverSessionStores;
use tinystoragedrivers::{BlobStore, DocumentStore, MemoryStorage, StorageBackend};

/// One fallback provider and the backend it was built over.
struct Fallback {
    backend: Arc<dyn StorageBackend>,
    provider: Arc<DriverSessionStores>,
}

static FALLBACKS: LazyLock<Mutex<HashMap<PathBuf, Fallback>>> = LazyLock::new(Mutex::default);

/// Drops the fallback providers (and, with them, the default backends they
/// held), so a data reset can delete the files.
pub fn release_workspace_defaults() {
    FALLBACKS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    super::release_workspace_stores();
    crate::storage::local::release_all();
}

/// Drops what this process holds open for `workspace_dir` alone (its fallback
/// provider, its workspace transcript stores and its default backend), so that
/// workspace's files can be deleted or reopened afresh while every other
/// workspace keeps its live handles. A restart of one workspace, not of the
/// process; [`release_workspace_defaults`] is the process-wide reset.
pub fn release_workspace(workspace_dir: &Path) {
    FALLBACKS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(workspace_dir);
    super::release_workspace_stores_for(workspace_dir);
    crate::storage::local::forget(workspace_dir);
}

/// The provider for `workspace_dir`: the installed one, else a
/// `DriverSessionStores` over the workspace's default backend.
///
/// Never fails: a default that cannot be opened (a read-only workspace) is
/// logged and replaced by an in-memory backend for that workspace, so a turn
/// degrades to a session that is not kept rather than to no session at all.
pub fn provider_for_workspace(workspace_dir: &Path) -> Arc<dyn SessionStoreProvider> {
    match super::installed() {
        Some(installed) => installed,
        None => fallback_provider(workspace_dir),
    }
}

/// The provider over `workspace_dir`'s default backend, installed or not.
pub(super) fn fallback_provider(workspace_dir: &Path) -> Arc<dyn SessionStoreProvider> {
    let backend = match crate::storage::local::backend_for(workspace_dir) {
        Ok(backend) => backend,
        Err(error) => {
            log::error!(
                "[session_store] no storage backend for workspace={}: {error:#}",
                workspace_dir.display()
            );
            return memory_fallback(workspace_dir);
        }
    };
    let mut fallbacks = FALLBACKS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(existing) = fallbacks.get(workspace_dir) {
        if Arc::ptr_eq(&existing.backend, &backend) {
            return existing.provider.clone();
        }
    }
    match DriverSessionStores::new(Arc::clone(&backend)) {
        Ok(provider) => {
            let provider = Arc::new(provider.recover_on_open(false));
            fallbacks.insert(
                workspace_dir.to_path_buf(),
                Fallback {
                    backend,
                    provider: provider.clone(),
                },
            );
            provider
        }
        Err(error) => {
            log::error!("[session_store] could not start the session store bridge: {error}");
            memory_fallback(workspace_dir)
        }
    }
}

fn memory_fallback(workspace_dir: &Path) -> Arc<dyn SessionStoreProvider> {
    static MEMORY: LazyLock<Mutex<HashMap<PathBuf, Arc<dyn SessionStoreProvider>>>> =
        LazyLock::new(Mutex::default);
    let mut memory = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
    memory
        .entry(workspace_dir.to_path_buf())
        .or_insert_with(|| {
            let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
            match DriverSessionStores::new(backend) {
                Ok(provider) => Arc::new(provider.recover_on_open(false)),
                Err(_) => Arc::new(tinyagents_session::port::InMemorySessionStores::new()),
            }
        })
        .clone()
}

/// The key the calling tenant's stores are under, or `None` for a SaaS task
/// with no scope (which must not act).
fn tenant_agent_key() -> Option<String> {
    match crate::core::runtime::DispatchCtx::ambient().tenant() {
        Ok(tenant) => Some(crate::core::runtime::session_key(&tenant)),
        Err(no_tenant) => {
            log::warn!("[session_store] {no_tenant}");
            None
        }
    }
}

/// The calling tenant's stores for `workspace_dir` ([`provider_for_workspace`]).
#[must_use]
pub fn stores_for(workspace_dir: &Path) -> Option<AgentStores> {
    let key = tenant_agent_key()?;
    Some(provider_for_workspace(workspace_dir).for_agent(&key))
}

/// [`stores_for`] `workspace_dir`, or throwaway in-memory stores for a call
/// that has no tenant to act for (a SaaS task with no scope), so it neither
/// fails nor reaches another tenant's records.
#[must_use]
pub fn stores_or_ephemeral(workspace_dir: &Path) -> AgentStores {
    stores_for(workspace_dir).unwrap_or_else(|| {
        log::warn!("[session_store] no tenant; using throwaway in-memory stores");
        tinyagents_session::port::InMemorySessionStores::new().for_agent("ephemeral")
    })
}

/// The calling tenant's turn-state snapshots for `workspace_dir`: the
/// `TurnStates` port of its session stores. A call with no tenant gets a
/// throwaway in-memory store.
#[must_use]
pub fn turn_states_for(workspace_dir: &Path) -> Arc<dyn TurnStates> {
    stores_or_ephemeral(workspace_dir).turn_states
}

/// The operator's ([`DEFAULT_AGENT`](super::DEFAULT_AGENT)) turn-state
/// snapshots for `workspace_dir`, independent of the calling task's tenant —
/// for boot-time recovery sweeps that run before any tenant is in scope.
#[must_use]
pub fn default_turn_states_for(workspace_dir: &Path) -> Arc<dyn TurnStates> {
    provider_for_workspace(workspace_dir)
        .for_agent(super::DEFAULT_AGENT)
        .turn_states
}

/// The calling tenant's attachment stash for `workspace_dir`: content-addressed
/// image bytes in the blob store of its agent scope, with the id index beside
/// them in its documents. `None` for a SaaS task with no scope or a backend that
/// refused the scope.
///
/// The host policy is no cap and no expiry: a conversation's attachment
/// references stay resolvable for as long as the conversation does.
#[must_use]
pub fn attachment_stash_for(workspace_dir: &Path) -> Option<BlobAttachmentStash> {
    let (key, scoped) = tenant_scope_for(workspace_dir, "attachments")?;
    log::debug!("[session_store] attachment stash bound agent={key}");
    Some(BlobAttachmentStash::new(
        Arc::clone(scoped.blobs()),
        Arc::clone(scoped.documents()),
        u64::MAX,
        std::time::Duration::MAX,
    ))
}

/// The calling tenant's blob store for worker artifacts (`outputs/`,
/// `workspace/` keys): the blob half of its agent scope on `workspace_dir`'s
/// storage backend. `None` for a SaaS task with no scope or a backend that
/// refused the scope; the caller then keeps the result inline.
#[must_use]
pub fn artifact_blobs_for(workspace_dir: &Path) -> Option<Arc<dyn BlobStore>> {
    let (key, scoped) = tenant_scope_for(workspace_dir, "artifacts")?;
    log::debug!("[session_store] artifact blobs bound agent={key}");
    Some(Arc::clone(scoped.blobs()))
}

/// The calling tenant's scoped storage on `workspace_dir`'s backend. `what`
/// only labels the log line when the backend or scope is refused.
fn tenant_scope_for(
    workspace_dir: &Path,
    what: &str,
) -> Option<(String, tinystoragedrivers::ScopedStorage)> {
    let key = tenant_agent_key()?;
    let backend = match crate::storage::local::backend_for(workspace_dir) {
        Ok(backend) => backend,
        Err(error) => {
            log::warn!("[session_store] no storage backend for {what}: {error:#}");
            return None;
        }
    };
    match backend.for_scope(&crate::storage::scope_for_agent(&key)) {
        Ok(scoped) => Some((key, scoped)),
        Err(error) => {
            log::warn!("[session_store] {what} scope refused agent={key}: {error}");
            None
        }
    }
}

/// The calling tenant's run ledger for `workspace_dir`.
///
/// The installed provider's when it keeps one, else the workspace default's.
/// `None` for a SaaS task with no scope; a ledger the backend refuses is
/// logged and also `None`, so best-effort recording never takes a turn down.
#[must_use]
pub fn ledger_for(workspace_dir: &Path) -> Option<Arc<dyn RunLedgerStore>> {
    let key = tenant_agent_key()?;
    if let Some(installed) = super::installed() {
        match installed.run_ledger_for_agent(&key) {
            Ok(Some(ledger)) => return Some(ledger),
            Ok(None) => {}
            Err(error) => {
                log::warn!("[session_store] run ledger unavailable agent={key}: {error}");
                return None;
            }
        }
    }
    let fallback = fallback_provider(workspace_dir);
    match fallback.run_ledger_for_agent(&key) {
        Ok(ledger) => ledger,
        Err(error) => {
            log::warn!("[session_store] run ledger unavailable agent={key}: {error}");
            None
        }
    }
}

/// The calling tenant's document store for `workspace_dir`, in the same scope
/// as its session stores — for the stores (run status, task ledgers) that are
/// plain collections beside them.
#[must_use]
pub fn current_documents_for(workspace_dir: &Path) -> Option<Arc<dyn DocumentStore>> {
    let key = tenant_agent_key()?;
    let backend = match crate::storage::local::backend_for(workspace_dir) {
        Ok(backend) => backend,
        Err(error) => {
            log::warn!("[session_store] no storage backend for documents: {error:#}");
            return None;
        }
    };
    match backend.for_scope(&crate::storage::scope_for_agent(&key)) {
        Ok(scoped) => Some(Arc::clone(scoped.documents())),
        Err(error) => {
            log::warn!("[session_store] documents scope refused agent={key}: {error}");
            None
        }
    }
}

/// The calling tenant's durable run-status store for `workspace_dir`: the
/// `harness_run_status` documents of its agent scope, beside its session
/// stores. A call with no tenant gets a throwaway in-memory store.
#[must_use]
pub fn status_store_for(
    workspace_dir: &Path,
) -> tinyagents_harness::observability::DriverStatusStore {
    let docs = current_documents_for(workspace_dir).unwrap_or_else(|| {
        log::warn!("[session_store] no tenant; using a throwaway run-status store");
        let scoped = MemoryStorage::new()
            .for_scope(&tinystoragedrivers::Scope::local())
            .expect("the memory backend binds the local scope");
        Arc::clone(scoped.documents())
    });
    tinyagents_harness::observability::DriverStatusStore::new(docs)
}

/// The calling tenant's run ledger for `workspace_dir` ([`ledger_for`]), or an
/// `unavailable` driver error when it has none.
///
/// # Errors
///
/// A SaaS task with no scope, or a backend that refused the scope.
pub fn require_ledger(workspace_dir: &Path) -> LedgerResult<Arc<dyn RunLedgerStore>> {
    ledger_for(workspace_dir).ok_or_else(|| {
        LedgerError::from(tinystoragedrivers::StorageError::unavailable(
            "no run ledger for the calling tenant",
        ))
    })
}

/// Runs one run-ledger operation on the calling tenant's ledger for
/// `workspace_dir`, from synchronous code (the shared storage bridge).
///
/// `op` receives the ledger and returns the future to run; it is invoked on
/// the calling thread and the future runs on the bridge.
///
/// # Errors
///
/// [`LedgerError`] from the operation, or an `unavailable` driver error when
/// the tenant has no ledger (a SaaS task with no scope, or a backend that
/// refused the scope).
pub fn with_ledger<T, F, Fut>(workspace_dir: &Path, op: F) -> LedgerResult<T>
where
    F: FnOnce(Arc<dyn RunLedgerStore>) -> Fut,
    Fut: std::future::Future<Output = LedgerResult<T>> + Send + 'static,
    T: Send + 'static,
{
    let ledger = require_ledger(workspace_dir)?;
    let future = op(ledger);
    match crate::storage::block_on(async move { Ok(future.await) }) {
        Ok(result) => result,
        Err(error) => Err(LedgerError::from(error)),
    }
}

/// [`with_ledger`] for async callers: the same ledger, awaited in place.
///
/// # Errors
///
/// As [`with_ledger`].
pub async fn with_ledger_async<T, F, Fut>(workspace_dir: &Path, op: F) -> LedgerResult<T>
where
    F: FnOnce(Arc<dyn RunLedgerStore>) -> Fut,
    Fut: std::future::Future<Output = LedgerResult<T>>,
{
    let ledger = require_ledger(workspace_dir)?;
    op(ledger).await
}

/// A session store that resolves the workspace of the calling context on every
/// use and keeps that workspace's sessions in its own default backend
/// ([`crate::storage::local::backend_for`]).
///
/// For the one host with no single backend to install: a SaaS core with no
/// storage URL, where every profile has a workspace of its own and so a
/// database of its own. Every other host installs
/// [`ReboundSessionStores`](super::ReboundSessionStores) over its backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct WorkspaceSessionStores;

impl WorkspaceSessionStores {
    fn current() -> Arc<dyn SessionStoreProvider> {
        fallback_provider(&super::context_workspace_dir())
    }
}

impl SessionStoreProvider for WorkspaceSessionStores {
    fn for_agent(&self, agent_id: &str) -> AgentStores {
        Self::current().for_agent(agent_id)
    }

    fn try_for_agent(
        &self,
        agent_id: &str,
    ) -> Result<AgentStores, tinystoragedrivers::StorageError> {
        Self::current().try_for_agent(agent_id)
    }

    fn run_ledger_for_agent(
        &self,
        agent_id: &str,
    ) -> LedgerResult<Option<Arc<dyn RunLedgerStore>>> {
        Self::current().run_ledger_for_agent(agent_id)
    }

    fn conversations_for_agent(
        &self,
        agent_id: &str,
    ) -> tinyagents_session::threads::ConversationResult<
        Option<Arc<dyn tinyagents_session::threads::ConversationPersistence>>,
    > {
        Self::current().conversations_for_agent(agent_id)
    }

    /// Nothing to sweep at install time: the workspaces are the profiles', and
    /// a profile's turns are recovered when its lease is taken over
    /// (`profiles::recovery`), not when its stores open. Resolving "the
    /// current workspace" here would also open the default workspace's
    /// database before any profile exists.
    fn recover(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn destination_key(&self) -> Option<String> {
        Some("workspace-default".to_string())
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
