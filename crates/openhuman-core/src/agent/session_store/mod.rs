//! The host-installed session store.
//!
//! A host that keeps conversations in its own database (a cloud deployment
//! serving many users from one process) installs a
//! [`SessionStoreProvider`] here once, before agents run. From then on every
//! agent's transcripts, turn journal, run status, goals and todos go through
//! that agent's [`AgentStores`] instead of files under `workspace_dir`. For a
//! store that is not file-backed ([`replaces_files`]) the file-era mirrors
//! (the session dual-write and its shadow reads) stand down: the host store
//! is the only record.
//!
//! Like the memory engine's host binding
//! ([`crate::memory::engine::install_host_engine`]), this is process-wide:
//! OpenHuman runs one runtime per process. With nothing installed, the core
//! keeps today's on-disk layout.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError, RwLock};

use tinyagents_session::port::{AgentStores, SessionStoreProvider};

pub mod ledger_sync;
mod rebound;
mod workspace;

pub use rebound::{rebind_all, ReboundSessionStores, StoreRegistry};
pub use workspace::{
    artifact_blobs_for, attachment_stash_for, current_documents_for, default_turn_states_for,
    ledger_for, provider_for_workspace, release_workspace, release_workspace_defaults,
    require_ledger, status_store_for, stores_for, stores_or_ephemeral, turn_states_for,
    with_ledger, with_ledger_async, WorkspaceSessionStores,
};

static PROVIDER: LazyLock<RwLock<Option<Arc<dyn SessionStoreProvider>>>> =
    LazyLock::new(|| RwLock::new(None));

/// Routes every agent's session state through `provider`, replacing any
/// earlier one.
pub fn install(provider: Arc<dyn SessionStoreProvider>) -> Option<Arc<dyn SessionStoreProvider>> {
    tracing::info!(
        destination = ?provider.destination_key(),
        "[session_store] host session store installed"
    );
    PROVIDER
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(provider)
}

/// Removes the installed provider; the on-disk layout applies again. Returns
/// whether one was installed.
pub fn clear() -> bool {
    let had = PROVIDER
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .is_some();
    if had {
        tracing::info!("[session_store] host session store removed");
    }
    had
}

/// Clears the installed provider only when it is still the provider claimed
/// by the caller. This keeps a runtime from removing a replacement installed
/// by another owner after the runtime itself has been dropped.
pub fn clear_if(expected: &Arc<dyn SessionStoreProvider>) -> bool {
    let mut installed = PROVIDER.write().unwrap_or_else(PoisonError::into_inner);
    if installed
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, expected))
    {
        installed.take();
        tracing::info!("[session_store] host session store removed");
        true
    } else {
        false
    }
}

pub fn restore(provider: Option<Arc<dyn SessionStoreProvider>>) {
    *PROVIDER.write().unwrap_or_else(PoisonError::into_inner) = provider;
}

/// The provider in effect for the calling task's ambient context: see
/// [`installed_in`].
#[must_use]
pub fn installed() -> Option<Arc<dyn SessionStoreProvider>> {
    installed_in(&crate::core::runtime::DispatchCtx::ambient())
}

/// The provider in effect for `ctx`: the host-override one of the agent it
/// runs for, else the installed one, if any.
#[must_use]
pub fn installed_in(
    ctx: &crate::core::runtime::DispatchCtx,
) -> Option<Arc<dyn SessionStoreProvider>> {
    ctx.host_overrides()
        .and_then(|local| local.session_store.clone())
        .or_else(|| {
            PROVIDER
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        })
}

/// Whether a host session store is in effect.
#[must_use]
pub fn is_installed() -> bool {
    installed().is_some()
}

/// Whether the store in effect keeps conversations somewhere other than the
/// classic workspace files ([`SessionStoreProvider::workspace_dir`] is
/// `None`), so companions that read those files back have nothing to read.
#[must_use]
pub fn replaces_files() -> bool {
    installed().is_some_and(|provider| provider.workspace_dir().is_none())
}

/// `agent_id`'s stores from the installed provider, or `None` when the core
/// keeps the on-disk layout.
#[must_use]
pub fn for_agent(agent_id: &str) -> Option<AgentStores> {
    installed().map(|provider| provider.for_agent(agent_id))
}

/// `agent_id`'s transcripts from the store in effect, or `None` when the core
/// keeps them as workspace files.
#[must_use]
pub fn transcripts_for(
    agent_id: &str,
) -> Option<Arc<dyn tinyagents_session::transcript::TranscriptLocator>> {
    let stores = for_agent(agent_id)?;
    log::debug!("[session_store] transcripts via the host store agent={agent_id}");
    Some(stores.transcripts)
}

/// [`transcripts_for`] `agent_id`, or the workspace's own transcript store
/// ([`workspace_transcripts`]) when no host store is in effect — the session
/// host's resolution.
#[must_use]
pub fn transcripts_or_files(
    agent_id: &str,
    workspace_dir: &std::path::Path,
) -> Arc<dyn tinyagents_session::transcript::TranscriptLocator> {
    transcripts_for(agent_id).unwrap_or_else(|| workspace_transcripts(agent_id, workspace_dir))
}

/// [`transcripts_or_files`] for the store `core` selects, never the ambient
/// context's: `core`'s own host-override provider, else the installed one,
/// else the workspace's store. For work that carries its context explicitly
/// (a detached sub-agent on a bare `tokio::spawn`, which has no ambient one).
#[must_use]
pub fn transcripts_or_files_in(
    core: &crate::core::runtime::CoreContext,
    agent_id: &str,
    workspace_dir: &std::path::Path,
) -> Arc<dyn tinyagents_session::transcript::TranscriptLocator> {
    let provider = core
        .host_overrides()
        .and_then(|overrides| overrides.session_store.clone())
        .or_else(|| {
            PROVIDER
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        });
    match provider {
        Some(provider) => {
            log::debug!(
                "[session_store] transcripts via the carried context's store agent={agent_id}"
            );
            provider.for_agent(agent_id).transcripts
        }
        None => workspace_transcripts(agent_id, workspace_dir),
    }
}

/// The workspace stores this process holds open, so repeat lookups share one
/// backend ([`release_workspace_stores`] drops them for a data reset).
static OPEN_WORKSPACE_STORES: LazyLock<
    Mutex<HashMap<(std::path::PathBuf, String), tinyagents_session::port::WorkspaceStores>>,
> = LazyLock::new(Mutex::default);

/// Drops the workspace transcript stores held open by [`workspace_transcripts`].
pub fn release_workspace_stores() {
    OPEN_WORKSPACE_STORES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

/// Drops the workspace transcript stores held open for `workspace_dir` (every
/// agent's), leaving other workspaces' alone.
pub fn release_workspace_stores_for(workspace_dir: &std::path::Path) {
    OPEN_WORKSPACE_STORES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|(dir, _), _| dir != workspace_dir);
}

/// `agent_id`'s transcripts for a host that only has a workspace directory:
/// TinyAgents' `WorkspaceStores` (the driver ports over a SQLite store under
/// the workspace). It imports the workspace's classic `session_raw/*.jsonl`
/// files once, read-only, so no file API is involved.
///
/// A workspace whose store cannot be opened (read-only disk) degrades to an
/// in-memory locator for the process rather than failing the turn.
#[must_use]
pub fn workspace_transcripts(
    agent_id: &str,
    workspace_dir: &std::path::Path,
) -> Arc<dyn tinyagents_session::transcript::TranscriptLocator> {
    let key = (workspace_dir.to_path_buf(), agent_id.to_string());
    let mut open = OPEN_WORKSPACE_STORES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(stores) = open.get(&key) {
        return stores.transcripts();
    }
    match tinyagents_session::port::WorkspaceStores::open(workspace_dir, agent_id) {
        Ok(stores) => {
            log::debug!(
                "[session_store] transcripts via the workspace store agent={agent_id} import={:?}",
                stores.import_outcome()
            );
            let transcripts = stores.transcripts();
            open.insert(key, stores);
            transcripts
        }
        Err(error) => {
            log::error!(
                "[session_store] workspace store unavailable agent={agent_id} workspace={}: {error:#}; \
                 transcripts are not kept",
                workspace_dir.display()
            );
            tinyagents_session::port::InMemorySessionStores::new()
                .for_agent(agent_id)
                .transcripts
        }
    }
}

/// The stores of the tenant the current [`CoreContext`] works for, keyed by
/// [`session_key`](crate::core::runtime::session_key): the agent it was
/// derived for ([`CoreContext::session_agent`]), else [`DEFAULT_AGENT`],
/// prefixed by its SaaS profile when it serves one — or `None` when the core
/// keeps the on-disk layout.
///
/// For code that has a workspace path but no agent id of its own (goals,
/// todos, the turn journal): under an embedded agent's context it lands in
/// that agent's stores. A SaaS task with no scope gets none rather than the
/// shared default bucket.
///
/// [`CoreContext`]: crate::core::runtime::CoreContext
/// [`CoreContext::session_agent`]: crate::core::runtime::CoreContext::session_agent
#[must_use]
pub fn current(ctx: &crate::core::runtime::DispatchCtx) -> Option<AgentStores> {
    let provider = installed()?;
    let tenant = match ctx.tenant() {
        Ok(tenant) => tenant,
        Err(no_tenant) => {
            log::warn!("[session_store] current: {no_tenant}");
            return None;
        }
    };
    Some(provider.for_agent(&crate::core::runtime::session_key(&tenant)))
}

/// The agent id a session host resolves transcripts under for the current
/// tenant: the [`session_key`](crate::core::runtime::session_key) when it
/// serves a SaaS profile, else the context's agent, else `fallback` (the
/// session's own definition id), as before profiles existed.
#[must_use]
pub fn current_agent_key_or(ctx: &crate::core::runtime::DispatchCtx, fallback: &str) -> String {
    agent_key_for(
        &crate::core::runtime::tenant::isolated_unless_tenant(ctx.tenant(), "session_locator"),
        fallback,
    )
}

/// The agent id `tenant`'s transcripts are kept under, for a caller that
/// already holds the request's tenant (an RPC handler's context): the
/// [`session_key`](crate::core::runtime::session_key) when it serves a SaaS
/// profile, else its agent, else `fallback` (the session's definition id).
/// [`current_agent_key_or`] is this over a context's tenant, so a reader that
/// passes its tenant here resolves the same store the session host wrote to.
#[must_use]
pub fn agent_key_for(tenant: &crate::core::runtime::Tenant, fallback: &str) -> String {
    if tenant.profile.is_some() {
        return crate::core::runtime::session_key(tenant);
    }
    tenant.agent.clone().unwrap_or_else(|| fallback.to_string())
}

/// The workspace the current [`CoreContext`](crate::core::runtime::CoreContext)
/// is bound to, for a file-backed store that must follow it (the desktop
/// rebinds it when a different user signs in). Before the core has booted —
/// when no store is asked for anything — the default config's workspace.
#[must_use]
pub fn context_workspace_dir() -> std::path::PathBuf {
    crate::core::runtime::CoreContext::current()
        .and_then(|context| context.workspace_dir().ok())
        .unwrap_or_else(|| {
            log::warn!("[session_store] no booted context; using the default workspace");
            crate::config::Config::default().workspace_dir
        })
}

/// The agent whose stores a turn without a definition id uses. Root turns of
/// the desktop app have no per-user agent; a single shared bucket keeps them
/// together, as the shared workspace always did.
pub const DEFAULT_AGENT: &str = "default";

#[cfg(test)]
pub(crate) mod splice_fixture;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
