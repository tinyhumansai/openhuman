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

use std::sync::{Arc, LazyLock, PoisonError, RwLock};

pub use agent_transcripts::{agent_transcript_root, AgentTranscriptFiles};
use tinyagents_session::port::{AgentStores, SessionStoreProvider};

mod agent_transcripts;

static PROVIDER: LazyLock<RwLock<Option<Arc<dyn SessionStoreProvider>>>> =
    LazyLock::new(|| RwLock::new(None));

tokio::task_local! {
    /// A provider for one task tree only, ahead of the process-wide one; how
    /// tests in a shared binary use a store without touching each other.
    static SCOPED: Arc<dyn SessionStoreProvider>;
}

/// Runs `future` with `provider` as the session store, for that task only.
/// Tasks it spawns do not inherit it.
pub async fn scope<F: std::future::Future>(
    provider: Arc<dyn SessionStoreProvider>,
    future: F,
) -> F::Output {
    SCOPED.scope(provider, future).await
}

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

/// The provider in effect: the task's [`scope`]d one, else the installed
/// one, if any.
#[must_use]
pub fn installed() -> Option<Arc<dyn SessionStoreProvider>> {
    SCOPED
        .try_with(Arc::clone)
        .ok()
        .or_else(|| {
            crate::core::runtime::CoreContext::current_host_overrides()
                .and_then(|local| local.session_store.clone())
        })
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

/// [`transcripts_for`] `agent_id`, or the transcript files under
/// `workspace_dir` when no store is in effect — the session host's resolution.
#[must_use]
pub fn transcripts_or_files(
    agent_id: &str,
    workspace_dir: &std::path::Path,
) -> Arc<dyn tinyagents_session::transcript::TranscriptLocator> {
    transcripts_for(agent_id).unwrap_or_else(|| match current_embedded_agent() {
        Some(embedded) => {
            log::debug!("[session_store] transcripts under the agent's directory agent={embedded}");
            Arc::new(AgentTranscriptFiles::new(workspace_dir, &embedded))
        }
        None => Arc::new(tinyagents_session::transcript::FileTranscriptLocator::new(
            workspace_dir,
        )),
    })
}

/// The workspace root transcripts are written under: the agent's own
/// directory under an embedded agent's context, else `workspace_dir` — which
/// for a SaaS profile's default agent is that profile's own workspace.
#[must_use]
pub fn transcript_root(workspace_dir: &std::path::Path) -> std::path::PathBuf {
    match current_embedded_agent() {
        Some(agent) => agent_transcript_root(workspace_dir, &agent),
        None => workspace_dir.to_path_buf(),
    }
}

/// The embedded agent the calling tenant runs as, if any. A SaaS profile's
/// default agent has none: its transcripts live at its workspace root, which
/// no other profile shares. A SaaS task with no scope has none either; the
/// workspace it was handed is the only one it touches.
#[must_use]
pub fn current_embedded_agent() -> Option<String> {
    crate::core::runtime::current_tenant()
        .ok()
        .and_then(|tenant| tenant.agent)
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
pub fn current() -> Option<AgentStores> {
    let provider = installed()?;
    let tenant = match crate::core::runtime::current_tenant() {
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
pub fn current_agent_key_or(fallback: &str) -> String {
    let tenant = crate::core::runtime::tenant::current_tenant_or_isolated("session_locator");
    if tenant.profile.is_some() {
        return crate::core::runtime::session_key(&tenant);
    }
    tenant.agent.unwrap_or_else(|| fallback.to_string())
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
#[path = "mod_tests.rs"]
mod tests;
