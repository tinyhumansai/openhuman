//! The session store the app, the CLI and the TUI install.
//!
//! One storage URL decides where every conversation lives
//! (`OPENHUMAN_STORAGE_URL`, else `[storage] url`, else the default
//! `sqlite:<workspace>/storage`). The backend is opened and made the process's
//! storage backend, and TinyAgents' `DriverSessionStores` goes in over it
//! ([`ReboundSessionStores`], so a different user signing in can move it to
//! their workspace's backend without reinstalling the provider): every agent's
//! transcripts, turn states, records, run ledger and journal in that backend,
//! one storage scope per agent. There is no file layout beside it; what older
//! installs wrote is imported once, read-only, by `storage::import` at boot.
//!
//! # Why there are three install paths
//!
//! The store is one provider; what differs is how long it must stay installed:
//!
//! - **Servers (`server::shims::build_and_serve`, used by `host::cli` and
//!   `host::desktop`)** install it for the life of the process
//!   ([`install_for_host`]), before the runtime builds so the recovery sweep
//!   sees it. They deliberately avoid `RuntimeBuilder::session_store`, which
//!   restores the previous provider when the runtime drops: the desktop
//!   restarts its in-process server in place, and a detached turn still
//!   writing across that gap must keep landing in the store rather than in
//!   the core's no-provider fallback. A builder that carries its own store
//!   skips this step.
//! - **The TUI (`host::tui`)** hands the provider to the builder
//!   ([`provider_for_host`]). Its runtime lives for the whole process and is
//!   dropped once at exit, so restore-on-drop is correct and leaves nothing
//!   installed behind it.
//! - **SaaS (`server::run_server_saas`)** calls [`install_for_saas`]. The core
//!   boots from `core::runtime::saas::build` rather than a `RuntimeBuilder`.
//!   With a storage URL it is the shared backend; with none, every profile
//!   keeps its sessions in the default database of its own workspace
//!   (`WorkspaceSessionStores`).
//!
//! The default URL follows the workspace. The desktop rebinds its workspace
//! when a different user signs in, and `storage::rebind` re-resolves the URL
//! and reinstalls the backend for the new one; an explicit URL names one
//! backend for every user and is never replaced.

use std::sync::Arc;

use anyhow::Context as _;
use tinyagents_session::port::SessionStoreProvider;

use crate::core_host::agent::session_store::{ReboundSessionStores, WorkspaceSessionStores};

/// Installs the session store the host's configuration asks for, before the
/// core boots.
///
/// The backend is opened and made the process's storage backend
/// ([`crate::core_host::storage::install_for_url`]), and TinyAgents'
/// `DriverSessionStores` is installed over it.
///
/// A single-process backend (SQLite, memory, files) also interrupts an
/// agent's in-flight turns the first time it is opened, since only an earlier
/// process can have left them. MongoDB does not: several processes may share
/// that database, and another one may own those turns.
///
/// # Errors
///
/// When the URL cannot be parsed or opened, or the config that may name one
/// cannot be loaded. A deployment that asked for a backend must not quietly
/// fall back to something else.
pub async fn install_for_host() -> anyhow::Result<()> {
    install_for_url(configured_storage_url().await?).await
}

/// The session store the host's configuration asks for, as a provider a
/// runtime builder's `session_store` option takes (what [`crate::host::tui`]
/// wires). [`install_for_host`] installs the same provider process-wide
/// instead.
///
/// Resolves the storage URL exactly as [`install_for_host`] does and has the
/// same side effect on the process's storage backend: it is opened and
/// installed.
///
/// # Errors
///
/// When the URL cannot be parsed or opened.
pub async fn provider_for_host() -> anyhow::Result<Arc<dyn SessionStoreProvider>> {
    provider_for_url(configured_storage_url().await?).await
}

/// The storage URL the host asks for: `OPENHUMAN_STORAGE_URL`, else
/// `[storage] url`, else the default for the active workspace. `None` only in
/// SaaS mode with nothing configured.
///
/// # Errors
///
/// When no environment URL pins the backend and the config cannot be loaded:
/// the config may name a `[storage] url`, and a deployment that asked for a
/// backend must not quietly use another.
async fn configured_storage_url() -> anyhow::Result<Option<String>> {
    let env = std::env::var(crate::core_host::storage::STORAGE_URL_VAR).ok();
    // A URL in the environment wins and never reads the config.
    if env.as_deref().is_some_and(|url| !url.trim().is_empty()) {
        return Ok(storage_url_from(env, &Default::default()));
    }
    // Boot-time resolution: the host's own context, no turn's.
    let ctx = crate::core_host::core::runtime::DispatchCtx::host_default();
    let load = || async {
        crate::core_host::config::ops::load_config_in(&ctx)
            .await
            .map_err(|error| {
                anyhow::anyhow!("loading the config to resolve the storage url: {error}")
            })
    };
    let mut config = load().await?;
    // `[storage] auto_migrate_layout`: move a layout-v1 profile to v2 before
    // the backend opens, then re-read the config, whose workspace is now the
    // profile's `files/` directory.
    if crate::core_host::storage::layout::auto_migrate(&config).await {
        config = load().await?;
    }
    Ok(storage_url_from(env, &config))
}

/// The core's URL rule ([`crate::core_host::storage::url_from`]) over `config`.
fn storage_url_from(
    env: Option<String>,
    config: &crate::core_host::config::Config,
) -> Option<String> {
    crate::core_host::storage::url_from(env, config)
}

/// [`install_for_host`] with the URL already resolved.
///
/// # Errors
///
/// When `url` cannot be parsed or opened.
pub async fn install_for_url(url: Option<String>) -> anyhow::Result<()> {
    let provider = provider_for_url(url).await?;
    log::debug!("[rpc:session_store] installing process-wide (host configuration)");
    crate::core_host::agent::session_store::install(provider);
    Ok(())
}

/// [`provider_for_host`] with the URL already resolved: the backend is
/// opened, made the process's storage backend, and the session store goes in
/// over it. `None` (SaaS mode with nothing configured) keeps each profile's
/// sessions in the default database of its own workspace.
///
/// # Errors
///
/// When `url` cannot be parsed or opened.
pub async fn provider_for_url(
    url: Option<String>,
) -> anyhow::Result<Arc<dyn SessionStoreProvider>> {
    provider_for_url_with(url, None).await
}

/// [`install_for_url`] for a SaaS core: a backend's session store never
/// sweeps in-flight turns on open, whatever its driver. In SaaS mode recovery
/// is lease-driven — a profile's turns are interrupted only when its lease is
/// taken over from a holder that never released it — because another node
/// may be running them. With no URL every profile keeps its own database.
///
/// # Errors
///
/// When `url` cannot be parsed or opened.
pub async fn install_for_saas(url: Option<String>) -> anyhow::Result<()> {
    let provider = provider_for_url_with(url, Some(false)).await?;
    log::debug!("[rpc:session_store] installing process-wide (SaaS, lease-driven recovery)");
    crate::core_host::agent::session_store::install(provider);
    Ok(())
}

/// [`provider_for_url`] with the recovery sweep made explicit: `None` recovers
/// on open exactly when the driver is single-process.
async fn provider_for_url_with(
    url: Option<String>,
    recover_on_open: Option<bool>,
) -> anyhow::Result<Arc<dyn SessionStoreProvider>> {
    let Some(url) = url else {
        // Drop a backend an earlier call installed, so storage operations do
        // not keep writing to it while each workspace uses its own.
        crate::core_host::storage::clear();
        log::debug!("[rpc:session_store] no storage url; one default database per workspace");
        return Ok(Arc::new(WorkspaceSessionStores));
    };
    let backend = crate::core_host::storage::open(&url)
        .await
        .context("opening the configured storage backend")?;
    let recover = recover_on_open
        .unwrap_or_else(|| !crate::core_host::storage::driver_is_shared(backend.driver()));
    let provider = ReboundSessionStores::new(Arc::clone(&backend), recover)
        .context("starting the session store bridge")?;
    // Only a fully working bridge makes the backend the process's storage.
    crate::core_host::storage::install_for_url(&url, backend);
    log::info!(
        "[rpc:session_store] opened the storage-backed session store \
         recover_on_open={recover}"
    );
    Ok(provider)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
