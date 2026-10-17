//! Following the workspace.
//!
//! The default backend is `sqlite:<user dir>/storage.db` (layout v2) or
//! `sqlite:<workspace>/storage` (v1), so it belongs to a workspace. The desktop rebinds its workspace when a different user signs in
//! (`CoreContext::rebind_default_workspace`); this module is the storage half
//! of that rebind. It re-resolves the URL for the new workspace and, when it
//! differs from the one the host installed:
//!
//! 1. opens the new backend;
//! 2. points the session store (`ReboundSessionStores`) and the process's
//!    storage slot at it, together, so no store reads one workspace's
//!    sessions beside another's documents;
//! 3. runs the read-only legacy importers over the new workspace, since a
//!    user who never ran this build has files to bring in.
//!
//! An explicit URL (`OPENHUMAN_STORAGE_URL`, `[storage] url`) names one
//! backend for every user and is never replaced. A backend someone installed
//! without a URL (a test's, an embedder's own) is left alone as well.

use std::sync::Arc;

use anyhow::{Context, Result};

use super::import;
use super::StorageBackend;
use crate::config::Config;

/// What a rebind did.
#[derive(Debug)]
pub struct Rebound {
    /// The URL that was installed before (credentials redacted).
    pub from: String,
    /// The URL installed now (credentials redacted).
    pub to: String,
    /// How the legacy importers fared over the new workspace.
    pub imports: Vec<import::Report>,
}

/// Re-resolves the storage URL for `config`'s workspace and reinstalls the
/// backend when it moved. `Ok(None)` when nothing changed or nothing is
/// managed by URL.
///
/// Synchronous: it runs on the shared storage bridge, so it may be called from
/// the credential path that rebinds every other process-global store.
///
/// # Errors
///
/// When the new backend cannot be opened or the session store refuses it. The
/// previous backend stays installed on error.
pub fn rebind(config: &Config) -> Result<Option<Rebound>> {
    rebind_with(config, super::installed_url(), |url, backend| {
        // Sessions first: no store may read one workspace's sessions beside
        // another's documents, so both move together.
        let rebound = crate::agent::session_store::rebind_all(&backend);
        super::install_for_url(url, backend);
        rebound
    })
}

/// [`rebind`] with the URL the host installed and the installation made
/// explicit, so the rule is testable without touching the process's slot.
/// `install` receives the new URL and backend and returns how many session
/// stores it moved.
pub(crate) fn rebind_with(
    config: &Config,
    current: Option<String>,
    install: impl FnOnce(&str, Arc<dyn StorageBackend>) -> usize,
) -> Result<Option<Rebound>> {
    let Some(current) = current else {
        return Ok(None);
    };
    let Some(next) = super::configured_url(config) else {
        return Ok(None);
    };
    if next == current {
        return Ok(None);
    }
    let redacted_from = crate::config::schema::storage::redact_url(&current);
    let redacted_to = crate::config::schema::storage::redact_url(&next);
    tracing::info!(
        target: "openhuman::storage",
        from = %redacted_from,
        to = %redacted_to,
        workspace = %config.workspace_dir.display(),
        "[storage] the workspace moved; reopening the storage backend"
    );
    super::config::prepare_default_dir(&config.workspace_dir).with_context(|| {
        format!(
            "create the storage directory for {}",
            config.workspace_dir.display()
        )
    })?;
    let url = next.clone();
    let backend = super::block_on_anyhow(async move { Ok(super::open(&url).await?) })
        .with_context(|| format!("open the storage backend {redacted_to}"))?;

    let session_stores = install(&next, Arc::clone(&backend));
    tracing::debug!(
        target: "openhuman::storage",
        session_stores,
        "[storage] backend reinstalled"
    );

    let imports = if crate::core::runtime::mode::is_saas() {
        Vec::new()
    } else {
        import::run_all(&backend, config)
    };
    Ok(Some(Rebound {
        from: redacted_from,
        to: redacted_to,
        imports,
    }))
}

#[cfg(test)]
#[path = "rebind_tests.rs"]
mod tests;
