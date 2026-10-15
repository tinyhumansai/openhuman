//! Loadable native modules: capabilities that live outside this binary.
//!
//! A module is a compiled `cdylib` that speaks the tinybus module ABI. It is
//! downloaded from a pinned release, verified against a digest compiled into
//! [`registry`], admitted through tinybus's ABI and manifest gates, and attached
//! to a private in-process broker as an ordinary bus peer. The core then calls it
//! over that bus like any other service.
//!
//! # What this buys, and what it costs
//!
//! It buys a dependency boundary that survives compilation. A document writer, a
//! codec, an integration SDK — none of these are kernel work, and each one drags
//! a tree of parsers into a binary that mostly does something else. Moving one
//! out removes its dependencies from the build rather than merely gating them.
//!
//! It costs process isolation, and that is not a small thing. A loaded module
//! shares this address space, these privileges and this crash domain: tinybus's
//! deadlines, bounded queues and caught panics contain ordinary misbehaviour, not
//! a segfault. `dlopen` also runs code before any symbol can be inspected, so the
//! ABI, manifest and digest gates decide what is *admitted*, never what is
//! *safe*. Modules are first-party code that happens to ship separately. Anything
//! untrusted belongs in a process, not here.
//!
//! And tinybus never unloads a library. A module that fails is failed until the
//! process restarts, which is why [`ops`] caches failures rather than retrying.
//!
//! # Layout
//!
//! - [`registry`] — the compiled-in set of loadable modules and their digests.
//! - [`documents`] — the host half of the `tinydocs` module's three operations.
//! - [`browser`] and [`browser_task`] — TinyComputer browser members and tasks
//!   without linking the browser engine into the core; [`site_memory_host`] serves
//!   where the module keeps what finished tasks learned about each site, and
//!   [`profile_claim`] asks the module to claim the persistent Chrome profile
//!   for a browser about to start.
//! - [`wallet`] — the host half of the `tinywallet` module, which builds and
//!   assembles transactions while the signing key stays in this process.
//! - [`linked`] — the registry of modules linked into the executable (local development).
//! - [`runtime`] — the process-wide `tinybus::module::ModuleRuntime` and the host
//!   callbacks served on its bus.
//! - [`ops`] — the adapter over `tinybus::module::Resolver` and `ModuleClient`:
//!   builds them from [`Config`](crate::config::Config) and the registry.
//! - [`report`] — the Sentry sink for terminal module outcomes.
//! - [`client`] — a `tinybus::module::ModuleClient`, with or without the loader.
//! - [`schemas`] — the `modules` RPC surface.
//! - [`boot`] — what happens at startup.

pub mod accessibility;
#[cfg(feature = "modules")]
pub mod boot;
#[cfg(feature = "modules")]
pub mod browser;
#[cfg(feature = "modules")]
pub mod browser_task;
#[cfg(feature = "modules")]
mod browser_task_report;
#[cfg(feature = "channels")]
pub(crate) mod channel_controller;
pub(crate) mod channel_health;
#[cfg(all(feature = "channels", feature = "modules"))]
pub(crate) mod channel_runtime;
#[cfg(feature = "modules")]
pub mod computer;
#[cfg(feature = "modules")]
pub mod computer_config;
#[cfg(feature = "modules")]
pub mod connectors;
#[cfg(feature = "modules")]
pub mod desktop;
#[cfg(feature = "documents")]
#[cfg(feature = "modules")]
pub mod documents;
#[cfg(feature = "modules")]
pub mod linked;
#[cfg(feature = "modules")]
pub mod ops;
#[cfg(feature = "modules")]
pub(crate) mod profile_claim;
pub mod registry;
pub(crate) mod report;
#[cfg(feature = "modules")]
pub mod runtime;
#[cfg(feature = "modules")]
pub mod schemas;
#[cfg(feature = "modules")]
pub mod search;
#[cfg(feature = "security-module")]
pub mod security;
#[cfg(feature = "security-module")]
mod security_host;
#[cfg(feature = "security-module")]
mod security_watch;
#[cfg(feature = "modules")]
mod search_path;
#[cfg(feature = "modules")]
pub(crate) mod site_memory_host;
mod source;
#[cfg(feature = "modules")]
mod storage_grant_tinybox;
#[cfg(feature = "modules")]
mod storage_grant_tinychannels;
#[cfg(feature = "modules")]
mod storage_grant_tinyconnectors;
#[cfg(feature = "modules")]
mod storage_grant_tinyjuice;
#[cfg(feature = "modules")]
mod storage_grant_tinymcp;
#[cfg(feature = "modules")]
mod storage_grant_tinywallet;
#[cfg(feature = "modules")]
pub mod storage_grants;
#[cfg(feature = "modules")]
pub(crate) mod storage_host;
#[cfg(feature = "modules")]
mod storage_host_linked;
#[cfg(all(test, feature = "modules"))]
mod storage_host_watching;
#[cfg(feature = "modules")]
mod storage_import_support;
#[cfg(feature = "modules")]
mod storage_import_tinybox;
#[cfg(feature = "modules")]
mod storage_import_tinyconnectors;
#[cfg(feature = "modules")]
mod storage_import_tinyjuice;
#[cfg(feature = "modules")]
mod storage_import_tinymcp;
#[cfg(all(feature = "modules", feature = "web3"))]
mod storage_import_tinywallet;
#[cfg(feature = "modules")]
pub mod tinybox;
#[cfg(feature = "modules")]
mod tokenjuice_host;
pub mod types;
#[cfg(not(feature = "modules"))]
mod unloaded;
#[cfg(feature = "voice")]
#[cfg(feature = "modules")]
pub mod voice;
#[cfg(feature = "web3")]
#[cfg(feature = "modules")]
pub mod wallet;
#[cfg(feature = "web3")]
mod wallet_host;
#[cfg(feature = "web3")]
pub(crate) mod wallet_service;
#[cfg(feature = "web3")]
pub(crate) mod web3_service;
#[cfg(feature = "web3")]
pub(crate) mod x402_service;

#[cfg(feature = "modules")]
pub use ops::{ensure_loaded, ensure_loaded_within, state_of};
#[cfg(feature = "modules")]
pub use schemas::{all_controller_schemas, all_registered_controllers};
#[doc(hidden)]
pub use source::local_artifact;
/// A `tinybus::module::ModuleClient` that loads modules lazily through the
/// process-wide resolver (see [`ops`]). Constructing one starts nothing. In a
/// build without the `modules` feature there is no loader, and every call is
/// unavailable and reported once with the `loader_disabled` reason.
#[must_use]
pub fn client(config: crate::config::Config) -> tinybus::module::ModuleClient {
    #[cfg(feature = "modules")]
    {
        ops::client(config)
    }
    #[cfg(not(feature = "modules"))]
    {
        unloaded::client(config)
    }
}

pub use types::{LoadPolicy, ModuleRecord, ModuleSource, ModuleState, ModuleStatus, PlatformAsset};

/// Imports tinymcp's legacy SQLite files under `data_dir` into the module's
/// storage, once per directory. Never fails the caller: an error is logged and
/// the import is retried at the next open.
pub async fn import_legacy_mcp(data_dir: &std::path::Path) {
    #[cfg(feature = "modules")]
    match storage_import_tinymcp::import_legacy(data_dir).await {
        Ok(outcome) => log::debug!("[mcp] legacy storage import: {outcome:?}"),
        Err(error) => log::warn!("[mcp] legacy storage import failed, will retry: {error:#}"),
    }
    #[cfg(not(feature = "modules"))]
    let _ = data_dir;
}

/// What an importer's outcome says about whether it may be skipped next time.
#[cfg(feature = "modules")]
trait Settled {
    /// True once the import finished or its marker was found; false while the
    /// outcome (no backend, no source, unsupported backend) can still change.
    fn settled(&self) -> bool;
}

#[cfg(feature = "modules")]
macro_rules! settled_outcome {
    ($($module:ident),*) => {$(
        impl Settled for $module::Outcome {
            fn settled(&self) -> bool {
                matches!(self, Self::AlreadyDone | Self::Imported { .. })
            }
        }
    )*};
}

#[cfg(feature = "modules")]
settled_outcome!(
    storage_import_tinyconnectors,
    storage_import_tinybox,
    storage_import_tinyjuice
);
#[cfg(all(feature = "modules", feature = "web3"))]
settled_outcome!(storage_import_tinywallet);

/// The (importer, module scope, directory) triples this process already
/// imported, so a hot call path pays for the marker lookup once rather than
/// on every call. The async lock is held across an import, so a concurrent
/// first caller waits for the migration instead of racing it.
#[cfg(feature = "modules")]
type ImportedSet =
    tokio::sync::Mutex<std::collections::HashSet<(&'static str, String, std::path::PathBuf)>>;

#[cfg(feature = "modules")]
fn imported_once() -> &'static ImportedSet {
    static DONE: std::sync::OnceLock<ImportedSet> = std::sync::OnceLock::new();
    DONE.get_or_init(Default::default)
}

/// Runs one legacy import at most once per process, module scope and
/// directory, and again after a failure or an unsettled outcome (which is
/// logged, never returned). Returns false only when the import failed. The key is recorded only after the import
/// finishes, so a cancelled caller leaves nothing behind.
#[cfg(feature = "modules")]
async fn import_once<F, T>(
    kind: &'static str,
    module_id: &str,
    dir: &std::path::Path,
    run: F,
) -> bool
where
    F: std::future::Future<Output = anyhow::Result<T>>,
    T: std::fmt::Debug + Settled,
{
    let scope = match storage_host::current_module_scope(module_id) {
        Ok(scope) => format!("{scope:?}"),
        Err(error) => {
            log::warn!("[{kind}] legacy storage import skipped: {:?}", error.kind());
            return true;
        }
    };
    let key = (kind, scope, dir.to_path_buf());
    let mut done = imported_once().lock().await;
    if done.contains(&key) {
        return true;
    }
    match run.await {
        Ok(outcome) => {
            log::debug!("[{kind}] legacy storage import: {outcome:?}");
            if outcome.settled() {
                done.insert(key);
            }
            true
        }
        Err(error) => {
            log::warn!("[{kind}] legacy storage import failed, will retry: {error:#}");
            false
        }
    }
}

/// Imports the connector module's legacy preference and trigger-archive files
/// under `state_dir` into its storage, once per directory. Never fails the
/// caller: an error is logged and the import is retried at the next call.
pub async fn import_legacy_connectors(state_dir: &std::path::Path) {
    #[cfg(feature = "modules")]
    import_once(
        "connectors",
        storage_grant_tinyconnectors::MODULE_ID,
        state_dir,
        storage_import_tinyconnectors::import_legacy(state_dir),
    )
    .await;
    #[cfg(not(feature = "modules"))]
    let _ = state_dir;
}

/// Imports the x402 ledger's legacy `payments.jsonl` under `workspace_dir`
/// into the wallet module's storage, once per workspace. Never fails the
/// caller: an error is logged and the import is retried at the next call. Returns
/// false when the import failed, so the caller can refuse to open a ledger that
/// would ignore the legacy payments.
pub async fn import_legacy_x402(workspace_dir: &std::path::Path) -> bool {
    #[cfg(all(feature = "modules", feature = "web3"))]
    return import_once(
        "x402",
        storage_grant_tinywallet::MODULE_ID,
        workspace_dir,
        storage_import_tinywallet::import_legacy(workspace_dir),
    )
    .await;
    #[cfg(not(all(feature = "modules", feature = "web3")))]
    {
        let _ = workspace_dir;
        true
    }
}

/// Imports the legacy jail registry index under `<workspace_dir>/jails` into
/// the tinybox module's storage, once. Never fails the caller: an error is
/// logged and the import is retried at the next call.
pub async fn import_legacy_tinybox(workspace_dir: &std::path::Path) {
    #[cfg(feature = "modules")]
    import_once(
        "tinybox",
        storage_grant_tinybox::MODULE_ID,
        workspace_dir,
        storage_import_tinybox::import_legacy(workspace_dir),
    )
    .await;
    #[cfg(not(feature = "modules"))]
    let _ = workspace_dir;
}

/// Scopes, under the current tenant, that hold expiring blobs written by
/// modules (today TinyJuice's CCR originals), for the blob sweeper. A module
/// whose scope cannot be resolved here is skipped. Without the `modules`
/// feature no module writes blobs, so there is nothing to sweep.
#[cfg(feature = "modules")]
pub(crate) fn expiring_blob_module_scopes() -> Vec<crate::storage::Scope> {
    #[cfg(feature = "modules")]
    {
        [storage_grant_tinyjuice::MODULE_ID]
            .into_iter()
            .filter_map(|id| storage_host::current_module_scope(id).ok())
            .collect()
    }
    // Without the loader no module holds blobs.
    #[cfg(not(feature = "modules"))]
    {
        Vec::new()
    }
}

/// [`expiring_blob_module_scopes`] for SaaS profile `profile`, whose acting
/// scope is `base`, from outside that profile's task scope.
#[cfg(feature = "modules")]
pub(crate) fn expiring_blob_module_scopes_for(
    profile: &str,
    base: &crate::storage::Scope,
) -> Vec<crate::storage::Scope> {
    [storage_grant_tinyjuice::MODULE_ID]
        .into_iter()
        .filter_map(|id| storage_host::module_scope(Some(profile), base, id).ok())
        .collect()
}

/// See the `modules` build: with the feature off no module writes blobs.
#[cfg(not(feature = "modules"))]
pub(crate) fn expiring_blob_module_scopes_for(
    _profile: &str,
    _base: &crate::storage::Scope,
) -> Vec<crate::storage::Scope> {
    Vec::new()
}

/// See the `modules` build: with the feature off no module writes blobs.
#[cfg(not(feature = "modules"))]
pub(crate) fn expiring_blob_module_scopes() -> Vec<crate::storage::Scope> {
    Vec::new()
}

/// Imports the legacy on-disk CCR tier under `root` into the tinyjuice
/// module's blob storage, once per directory. `ttl` is the configured original
/// lifetime. Never fails the caller: an error is logged and the import is
/// retried at the next install.
pub async fn import_legacy_tinyjuice(root: &std::path::Path, ttl: Option<std::time::Duration>) {
    #[cfg(feature = "modules")]
    import_once(
        "tinyjuice",
        storage_grant_tinyjuice::MODULE_ID,
        root,
        storage_import_tinyjuice::import_legacy(root, ttl),
    )
    .await;
    #[cfg(not(feature = "modules"))]
    let _ = (root, ttl);
}

#[cfg(all(test, feature = "modules"))]
#[path = "import_once_tests.rs"]
mod import_once_tests;
