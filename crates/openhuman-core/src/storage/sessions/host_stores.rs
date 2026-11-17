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

use super::tenant_router::TenantSessionStores;
use tinyagents_harness::multimodal::BlobAttachmentStash;
use tinyagents_session::port::{
    AgentStorageFactory, AgentStores, DriverSessionStores, SessionStoreLease,
    SessionStoreLeaseError, SessionStoreProvider, TurnStates,
};
use tinyagents_session::run_ledger::{LedgerError, LedgerResult, RunLedgerStore};
use tinyagents_storage_kit::ScopedStorage;
use tinystoragedrivers_api::{BlobStore, DocumentStore, StorageBackend};

/// One fallback provider and the backend it was built over.
struct Fallback {
    backend: Arc<dyn StorageBackend>,
    provider: Arc<TenantSessionStores>,
}

static FALLBACKS: LazyLock<Mutex<HashMap<PathBuf, Fallback>>> = LazyLock::new(Mutex::default);

/// Drops the fallback providers (and, with them, the default backends they
/// held), so a data reset can delete the files.
pub fn release_workspace_defaults() {
    FALLBACKS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
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
    crate::storage::local::forget(workspace_dir);
}

/// The provider for `workspace_dir`: the installed one, else a
/// `TenantSessionStores` over the workspace's default backend.
///
/// A storage refusal is retained by a refusing session provider. Session
/// state never silently moves to an in-memory backend.
pub fn provider_for_workspace(workspace_dir: &Path) -> Arc<dyn SessionStoreProvider> {
    match super::installed() {
        Some(installed) => installed,
        None => fallback_provider(workspace_dir),
    }
}

/// The provider selected by `context` for `workspace_dir`, else the workspace
/// fallback. This preserves agent-local host overrides when work carries an
/// explicit dispatch context rather than the ambient task context.
pub fn provider_for_workspace_in(
    workspace_dir: &Path,
    context: &crate::core::runtime::DispatchCtx,
) -> Arc<dyn SessionStoreProvider> {
    match super::installed_in(context) {
        Some(installed) => installed,
        None => fallback_provider(workspace_dir),
    }
}

/// The provider over `workspace_dir`'s default backend, installed or not.
pub(super) fn fallback_provider(workspace_dir: &Path) -> Arc<dyn SessionStoreProvider> {
    if crate::core::runtime::is_saas()
        && crate::storage::installed().is_none()
        && tenant_agent_key().is_none()
    {
        return refusing_provider(
            "no host storage backend is installed for this SaaS session".to_owned(),
        );
    }
    let backend = match crate::storage::local::backend_for(workspace_dir) {
        Ok(backend) => backend,
        Err(error) => {
            log::error!(
                "[session_store] no storage backend for workspace={}: {error:#}",
                workspace_dir.display()
            );
            return refusing_provider(error.to_string());
        }
    };
    let mut fallbacks = FALLBACKS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(existing) = fallbacks.get(workspace_dir) {
        if Arc::ptr_eq(&existing.backend, &backend) {
            return existing.provider.clone();
        }
    }
    match TenantSessionStores::new_workspace_local(Arc::clone(&backend), false) {
        Ok(provider) => {
            let provider = Arc::new(provider);
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
            refusing_provider(error.to_string())
        }
    }
}

fn refusing_provider(message: String) -> Arc<dyn SessionStoreProvider> {
    let factory: AgentStorageFactory = Arc::new(move |_| {
        Err(tinystoragedrivers_types::StorageError::unavailable(
            message.clone(),
        ))
    });
    Arc::new(
        DriverSessionStores::new(factory)
            .unwrap_or_else(|error| panic!("could not start refusal-only session bridge: {error}")),
    )
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

/// [`stores_for`] `workspace_dir`, or the local default agent's stores for a
/// non-SaaS call without an ambient tenant. A SaaS task without a profile gets
/// refusing stores rather than reaching another tenant's records.
#[must_use]
pub fn stores_or_ephemeral(workspace_dir: &Path) -> AgentStores {
    stores_for(workspace_dir).unwrap_or_else(|| {
        if crate::core::runtime::is_saas() {
            log::warn!("[session_store] no tenant in SaaS mode; session access refused");
            return refusing_provider(
                "no profile in SaaS mode; refusing session storage".to_owned(),
            )
            .for_agent("refused");
        }
        provider_for_workspace(workspace_dir).for_agent(super::DEFAULT_AGENT)
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

/// The artifact ports for the tenant carried by `context` on the workspace's
/// storage backend. Unlike [`artifact_blobs_for`], this does not consult the
/// ambient dispatch context, so a tool that was moved to a detached task keeps
/// using the profile that authorized the call.
#[must_use]
pub fn artifact_scoped_for_in(
    workspace_dir: &Path,
    context: &crate::core::runtime::DispatchCtx,
) -> Option<ScopedStorage> {
    let tenant = match context.tenant() {
        Ok(tenant) => tenant,
        Err(no_tenant) => {
            log::warn!("[session_store] no tenant for artifact scope: {no_tenant}");
            return None;
        }
    };
    // Match the storage domain's established scope for this tenant. Session
    // keys encode a default agent even when the storage scope is local, so do
    // not derive artifact scope from the session key.
    let scope = match crate::storage::scope_from(
        tenant.profile.as_deref(),
        tenant.agent.as_deref(),
        crate::core::runtime::mode::is_saas(),
    ) {
        Ok(scope) => scope,
        Err(error) => {
            log::warn!("[session_store] no artifact storage scope: {error}");
            return None;
        }
    };
    let backend = match context.resolved() {
        Some(core) => crate::core::runtime::CoreContext::sync_scope(core, || {
            crate::storage::local::backend_for(workspace_dir)
        }),
        // Outside SaaS, an unresolved context represents the single-user
        // default tenant validated above. The backend slot is process-global.
        None => crate::storage::local::backend_for(workspace_dir),
    };
    let backend = match backend {
        Ok(backend) => backend,
        Err(error) => {
            log::warn!("[session_store] no storage backend for artifacts: {error:#}");
            return None;
        }
    };
    match backend.for_scope(&scope) {
        Ok(scoped) => match tinyagents_storage_kit::Scope::new(scope.as_str()) {
            Ok(port_scope) => Some(ScopedStorage::new(scoped, port_scope)),
            Err(error) => {
                log::warn!("[session_store] artifact scope invalid: {error}");
                None
            }
        },
        Err(error) => {
            log::warn!(
                "[session_store] artifact scope refused scope={}: {error}",
                scope.as_str()
            );
            None
        }
    }
}

/// The calling tenant's scoped storage on `workspace_dir`'s backend. `what`
/// only labels the log line when the backend or scope is refused.
fn tenant_scope_for(workspace_dir: &Path, what: &str) -> Option<(String, ScopedStorage)> {
    let key = match tenant_agent_key() {
        Some(key) => key,
        None if !crate::core::runtime::is_saas() => super::DEFAULT_AGENT.to_owned(),
        None => return None,
    };
    let backend = match crate::storage::local::backend_for(workspace_dir) {
        Ok(backend) => backend,
        Err(error) => {
            log::warn!("[session_store] no storage backend for {what}: {error:#}");
            return None;
        }
    };
    let scope = session_scope(&key);
    if let Err(error) = crate::storage::documents::prepare_once(&backend, &scope) {
        log::warn!("[session_store] {what} scope not prepared agent={key}: {error}");
        return None;
    }
    match backend.for_scope(&scope) {
        Ok(scoped) => match tinyagents_storage_kit::Scope::new(scope.as_str()) {
            Ok(port_scope) => Some((key, ScopedStorage::new(scoped, port_scope))),
            Err(error) => {
                log::warn!("[session_store] {what} scope invalid agent={key}: {error}");
                None
            }
        },
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
    let scope = session_scope(&key);
    if let Err(error) = crate::storage::documents::prepare_once(&backend, &scope) {
        log::warn!("[session_store] documents scope not prepared agent={key}: {error}");
        return None;
    }
    match backend.for_scope(&scope) {
        Ok(scoped) => Some(Arc::clone(scoped.documents())),
        Err(error) => {
            log::warn!("[session_store] documents scope refused agent={key}: {error}");
            None
        }
    }
}

/// The calling tenant's durable run-status store for `workspace_dir`: the
/// `harness_run_status` documents of its agent scope, beside its session
/// stores. Non-SaaS calls without an ambient tenant use the local default
/// agent. SaaS calls without a tenant have no authorized store.
pub fn status_store_for(
    workspace_dir: &Path,
) -> Option<tinyagents_harness::observability::DriverStatusStore> {
    let docs = current_documents_for(workspace_dir).or_else(|| {
        if crate::core::runtime::is_saas() {
            return None;
        }
        let backend = match crate::storage::local::backend_for(workspace_dir) {
            Ok(backend) => backend,
            Err(error) => {
                log::warn!("[session_store] run-status backend unavailable: {error:#}");
                return None;
            }
        };
        let scope = session_scope(super::DEFAULT_AGENT);
        if let Err(error) = crate::storage::documents::prepare_once(&backend, &scope) {
            log::warn!("[session_store] local run-status scope not prepared: {error}");
            return None;
        }
        let scoped = match backend.for_scope(&scope) {
            Ok(scoped) => scoped,
            Err(error) => {
                log::warn!("[session_store] local run-status scope refused: {error}");
                return None;
            }
        };
        Some(Arc::clone(scoped.documents()))
    })?;
    Some(tinyagents_harness::observability::DriverStatusStore::new(
        docs,
    ))
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
/// [`TenantSessionStores`](super::TenantSessionStores) over its backend.
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

    fn lease_for_agent(&self, agent_id: &str) -> Result<SessionStoreLease, SessionStoreLeaseError> {
        Self::current().lease_for_agent(agent_id)
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

/// The storage scope a session key (`crate::core::runtime::session_key`)
/// names: `local/agent:<id>` or `u:<profile>/agent:<id>`, the scope the
/// session store itself binds that key to.
pub fn session_scope(key: &str) -> tinystoragedrivers::Scope {
    let (profile, mut agent) = crate::core::runtime::split_session_key(key);
    if agent.is_empty() {
        agent = crate::storage::scope::DEFAULT_AGENT.to_owned();
    }
    crate::storage::tenant_scope(profile.as_deref())
        .agent(&agent)
        .unwrap_or_else(|_| unreachable!("a non-empty agent id is a valid segment"))
        .to_storage_scope()
}

#[cfg(test)]
#[path = "host_stores_tests.rs"]
mod tests;
