//! Loading modules, and deciding when not to.
//!
//! [`ensure_loaded`] is the entry point every caller uses. It resolves a module
//! once per process and remembers the outcome — including failure, which is the
//! part worth explaining.
//!
//! tinybus never unloads a library. A module that was refused, faulted, or failed
//! to initialise keeps whatever it mapped, and loading it again cannot reach a
//! different outcome without a restart. Retrying would therefore mean paying a
//! download and a `dlopen` on every tool call to arrive at the same error, so a
//! failure is cached and returned directly. The user-visible consequence is that
//! fixing a module means restarting the core, which is stated in the error.
//!
//! # Where an artifact comes from, and where it stays
//!
//! Resolution order is cheapest-first: already loaded, a developer's override,
//! the module search path, then the release cache. The cache is a directory per
//! module version under [`install_dir`], filled by the first load — downloaded,
//! verified against the digest in [`registry`], extracted — and re-verified from
//! disk on every later launch. That is the difference between a launch that
//! maps a library in milliseconds and one that downloads five archives over
//! whatever network it happens to be on: the second is what every launch did
//! before the cache existed, and on a link with one unreachable CDN address it
//! cost minutes during which every memory call and every chat turn waited.
//!
//! # How callers wait
//!
//! Each module has one slot in [`super::resolution`]. The first caller runs the
//! resolution as a process-lifetime task and every other caller waits on its
//! outcome; two modules never queue behind each other. [`ensure_loaded_within`]
//! bounds the wait, so a caller with a deadline of its own can report the module
//! as still loading instead of hanging into that deadline.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::resolution::{self, Claim, Resolution, ResolutionState, ResolutionTable, Waited};
use super::types::{ModuleRecord, ModuleState, ModuleStatus};
use super::{host, platform, registry};
use crate::openhuman::config::Config;

/// Why a bounded [`ensure_loaded_within`] did not end with the module serving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// Terminal for the process. Carries a message suitable for a user or a
    /// model: no artifact for this host, downloads disabled, a refused
    /// artifact, or a previous failure in this process.
    Failed(String),
    /// The module is still being downloaded, verified, or initialised, and the
    /// caller's bound passed first. Asking again later can succeed.
    StillLoading,
}

impl LoadError {
    /// The message for a caller that cannot distinguish the two cases.
    #[must_use]
    pub fn into_message(self) -> String {
        match self {
            Self::Failed(message) => message,
            Self::StillLoading => "the module is still loading; try again shortly".to_string(),
        }
    }
}

/// Ensure `id` is loaded and serving, loading it if this is the first ask.
///
/// Waits without bound. Prefer [`ensure_loaded_within`] from any path that has
/// a deadline of its own.
///
/// # Errors
///
/// Returns a message suitable for surfacing to a user or a model when the module
/// cannot be loaded: no artifact for this host, downloads disabled, a refused
/// artifact, or a previous failure in this process.
pub async fn ensure_loaded(config: &Config, id: &str) -> Result<(), String> {
    ensure_loaded_within(config, id, None)
        .await
        .map_err(LoadError::into_message)
}

/// [`ensure_loaded`] with a bound on how long this caller waits.
///
/// The bound is on the *wait*, never on the resolution: a caller that gives up
/// leaves the download running and the slot intact, and a later call finds the
/// outcome. `None` waits without bound.
///
/// # Errors
///
/// [`LoadError::Failed`] when the module cannot be loaded in this process;
/// [`LoadError::StillLoading`] when `within` passed first.
pub async fn ensure_loaded_within(
    config: &Config,
    id: &str,
    within: Option<Duration>,
) -> Result<(), LoadError> {
    if !config.modules.enabled {
        return Err(LoadError::Failed(format!(
            "module '{id}' is unavailable: modules are disabled in configuration"
        )));
    }
    let record =
        registry::find(id).ok_or_else(|| LoadError::Failed(format!("unknown module '{id}'")))?;

    let receiver = match resolution::table().claim(id) {
        Claim::Done(Resolution::Ready) => return Ok(()),
        Claim::Done(Resolution::Failed(reason)) => return Err(LoadError::Failed(reason)),
        Claim::Wait(receiver) => receiver,
        Claim::Run { sender, receiver } => {
            start_resolution(config.clone(), record, sender).await;
            receiver
        }
    };
    match ResolutionTable::wait(receiver, within).await {
        Waited::Ready => Ok(()),
        Waited::Failed(reason) => Err(LoadError::Failed(reason)),
        Waited::StillLoading => Err(LoadError::StillLoading),
    }
}

/// The state of `id` as the resolution table reports it, without touching it.
#[must_use]
pub fn state_of(id: &str) -> ModuleState {
    match resolution::table().peek(id) {
        ResolutionState::Unresolved => ModuleState::Available,
        ResolutionState::Loading => ModuleState::Loading,
        ResolutionState::Ready => ModuleState::Ready,
        ResolutionState::Failed(_) => ModuleState::Failed,
    }
}

/// Run the resolution for `record` and record its outcome.
///
/// Spawned on the module runtime, which lives for the process: a caller
/// runtime that shuts down mid-load — a test's, an embedder's — must not
/// cancel a resolution other callers are waiting on. Without a module runtime
/// there is nothing to load into, so the outcome is recorded inline instead.
async fn start_resolution(
    config: Config,
    record: &'static ModuleRecord,
    sender: tokio::sync::watch::Sender<Option<Resolution>>,
) {
    let id = record.id;
    let work = async move {
        log::info!("[modules] resolving '{id}' {}", record.version);
        let resolution = match resolve(&config, record).await {
            Ok(()) => {
                log::info!("[modules] '{id}' is serving");
                Resolution::Ready
            }
            Err(reason) => {
                log::warn!("[modules] '{id}' did not load: {reason}");
                Resolution::Failed(reason)
            }
        };
        resolution::table().complete(id, resolution, sender);
    };
    match host::runtime().await {
        Ok(runtime) => {
            runtime.spawn(work);
        }
        Err(error) => {
            log::warn!("[modules] the module bus could not start: {error}");
            work.await;
        }
    }
}

/// Do the actual work of getting `record` serving.
async fn resolve(config: &Config, record: &'static ModuleRecord) -> Result<(), String> {
    let runtime = host::runtime().await.map_err(|_| {
        format!(
            "module '{}' is unavailable: the module bus could not start",
            record.id
        )
    })?;

    // Already serving — a module loaded from the search path at boot, or by an
    // earlier explicit `modules.load_local`.
    if runtime
        .host()
        .list()
        .iter()
        .any(|info| info.manifest.bus_name.as_str() == record.bus_name)
    {
        return Ok(());
    }

    // An override points at a developer's own build. Checked before the pinned
    // release so a module can be iterated on against a live core.
    if let Some(path) = local_override(config, record.id) {
        let module_config = module_config(config, record.id);
        return blocking(move || load_local(runtime, &path, record.id, module_config)).await;
    }

    // The search path tinybus itself honours, including OPENHUMAN_MODULE_PATH.
    // A refused search-path artifact is ordinary — most directories hold
    // nothing, and tinybus reports each refusal with a sanitised reason — so the
    // errors are dropped here and only a match on the bus name counts.
    let bus_name = record.bus_name;
    let found_on_search_path = tokio::task::spawn_blocking(move || {
        runtime
            .host()
            .load_search_paths()
            .into_iter()
            .flatten()
            .any(|info| info.manifest.bus_name.as_str() == bus_name)
    })
    .await
    .unwrap_or(false);
    if found_on_search_path {
        return Ok(());
    }

    // The release cache: a verified artifact from an earlier launch, or a
    // download into the same place. Off the runtime worker — the cold path
    // fetches over the network, hashes the archive, extracts it and `dlopen`s
    // the result, all synchronously, and left inline it would stall every
    // other task sharing this worker for the length of a download on
    // whatever link the user has.
    let Some(root) = install_dir(config) else {
        return Err(format!(
            "module '{}' is unavailable: no directory is available to install modules into",
            record.id
        ));
    };
    let allow_download = config.modules.allow_download;
    let module_config = module_config(config, record.id);
    let cache_root = root.clone();
    let outcome =
        blocking(move || load_cached(runtime, record, &cache_root, module_config, allow_download))
            .await;
    match outcome {
        Ok(()) => {
            prune_stale_versions(&root, record);
            Ok(())
        }
        Err(reason) if !allow_download => {
            log::debug!(
                "[modules] '{}' release cache miss with downloads disabled: {reason}",
                record.id
            );
            Err(format!(
                "module '{}' is unavailable: no local artifact is installed and downloads are \
                 disabled in configuration",
                record.id
            ))
        }
        Err(reason) => Err(reason),
    }
}

/// Run a blocking module operation on the blocking pool.
///
/// A panic in the loader is reported rather than propagated: it would otherwise
/// take down whichever task happened to be awaiting the load.
async fn blocking<F>(work: F) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    host::runtime()
        .await
        .map_err(|error| format!("the module bus could not start: {error}"))?
        .blocking(work)
        .await
        .map_err(|error| {
            format!(
                "{error}. This is terminal for the running process; restart the app to try again"
            )
        })
}

/// Load the pinned release for this host through the release cache.
///
/// Tries the preferred artifact first and falls through on admission failure —
/// a host newer than the newest published build runs that build, and one whose
/// toolchain the newest artifact does not match falls back. Each artifact has
/// its own directory under the version, so two builds of one release never
/// share an extraction.
fn load_cached(
    runtime: &'static host::ModuleRuntime,
    record: &'static ModuleRecord,
    install_root: &Path,
    module_config: serde_json::Value,
    allow_download: bool,
) -> Result<(), String> {
    let candidates = platform::host_candidates();
    let assets: Vec<_> = candidates
        .iter()
        .filter_map(|key| record.asset_for(key))
        .collect();
    if assets.is_empty() {
        return Err(format!(
            "module '{}' is not available for this platform, so the feature it provides is \
             unavailable in this build",
            record.id
        ));
    }

    let mut last_error = String::new();
    for asset in assets {
        let Some(cache_dir) = artifact_dir(install_root, record, asset.host_key) else {
            last_error =
                "the module's cache path could not be built from its registry entry".to_string();
            continue;
        };
        let release = tinybus::module::CachedRelease {
            release_url: record.release_url,
            asset_name: asset.archive,
            expected_sha256: Some(asset.sha256),
            cache_dir: &cache_dir,
            allow_download,
        };
        match runtime
            .host()
            .load_github_release_cached(&release, module_config.clone())
        {
            Ok(_) => {
                log::info!(
                    "[modules] loaded '{}' {} ({}) through the release cache",
                    record.id,
                    record.version,
                    asset.host_key
                );
                return Ok(());
            }
            Err(err) => {
                // Sanitised: tinybus's own errors carry only a basename and a
                // fixed reason, and nothing here adds a path or a URL.
                last_error = err.to_string();
                log::warn!(
                    "[modules] '{}' artifact for {} was not admitted: {last_error}",
                    record.id,
                    asset.host_key
                );
            }
        }
    }
    Err(format!(
        "module '{}' could not be loaded: {last_error}. This is terminal for the running \
         process; restart the app to try again",
        record.id
    ))
}

/// Whether `component` is safe to use as one directory name.
///
/// The three values that build a cache path — a module id, its version, and a
/// host key — are compiled-in `const` data today, so nothing reaches this with
/// a separator in it. It is checked anyway because of what sits at the end of
/// the path: [`prune_stale_versions`] calls `remove_dir_all` on what these
/// build. A registry edit or a future value that carried `..` or a separator
/// would turn a cache tidy-up into deleting somewhere else entirely, and a
/// rule that has to hold for a delete is worth stating rather than inferring
/// from where the data happens to come from today.
fn is_safe_path_component(component: &str) -> bool {
    !component.is_empty()
        && component != "."
        && component != ".."
        && !component.contains('/')
        && !component.contains('\\')
        && !component.contains('\0')
        // A leading dot would collide with the `.staging-*` directories a
        // concurrent download is filling.
        && !component.starts_with('.')
}

/// Where one artifact of one module version is cached, when all three
/// components are usable as directory names.
///
/// `None` rather than a sanitised path: a registry entry that cannot name a
/// directory is a build-time mistake, and quietly rewriting it would hide the
/// mistake behind a cache that silently never hits.
fn artifact_dir(install_root: &Path, record: &ModuleRecord, host_key: &str) -> Option<PathBuf> {
    for component in [record.id, record.version, host_key] {
        if !is_safe_path_component(component) {
            log::error!(
                "[modules] '{}' has a component that cannot name a directory; refusing to build a \
                 cache path from it",
                record.id
            );
            return None;
        }
    }
    Some(
        install_root
            .join(record.id)
            .join(record.version)
            .join(host_key),
    )
}

/// Remove cached versions of `record` other than the pinned one.
///
/// Best-effort and after the fact: a version that is no longer pinned will
/// never be loaded again, so keeping it only costs disk. Staging directories
/// are left alone — a concurrent process may be filling one — and every
/// removal is logged, because a cache that empties itself is worth noticing.
fn prune_stale_versions(install_root: &Path, record: &ModuleRecord) {
    // Both sides of the comparison below have to be real directory names, or
    // "everything that is not the pinned version" is not a set this function
    // should be handing to `remove_dir_all`.
    if !is_safe_path_component(record.id) || !is_safe_path_component(record.version) {
        log::error!(
            "[modules] refusing to prune '{}': its id or version cannot name a directory",
            record.id
        );
        return;
    }
    let module_root = install_root.join(record.id);
    let Ok(entries) = std::fs::read_dir(&module_root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // `read_dir` never yields `.` or `..`, and the leading-dot skip covers
        // the staging directories; the guard is here so the delete depends on
        // this function's own check rather than on that being remembered.
        if !path.is_dir() || name == record.version || !is_safe_path_component(&name) {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => log::info!(
                "[modules] removed cached '{}' {name}; {} is pinned",
                record.id,
                record.version
            ),
            Err(error) => log::warn!(
                "[modules] could not remove cached '{}' {name}: {error}",
                record.id
            ),
        }
    }
}

/// Load a platform library from `path`.
pub(super) fn load_local(
    runtime: &host::ModuleRuntime,
    path: &Path,
    id: &str,
    module_config: serde_json::Value,
) -> Result<(), String> {
    match runtime.host().load_file_with_config(path, module_config) {
        Ok(_) => {
            log::info!("[modules] loaded '{id}' from a local artifact");
            Ok(())
        }
        Err(err) => Err(format!(
            "module '{id}' could not be loaded from its local artifact: {err}. This is terminal \
             for the running process; restart the app to try again"
        )),
    }
}

/// Configuration crossing into a first-party compiled module.
///
/// Credentials are intentionally absent. TinyMemory calls back into the host
/// for embedding and chat compute; the other modules need no host config.
fn module_config(config: &Config, id: &str) -> serde_json::Value {
    if id == super::connectors::MODULE_ID {
        // The connector module takes its route and credential from here and
        // reads one from nowhere else. A configuration that cannot be built —
        // direct mode with no key, an unknown mode — loads the module with an
        // empty blob rather than failing the load: the capability members need
        // no route and must still answer, and every member that does need one
        // reports the missing route when it is called.
        return super::connectors::module_config(config).unwrap_or_else(|error| {
            tracing::info!(
                error = %error,
                "[connectors] no route configured; loading with the capability surface only"
            );
            serde_json::json!({})
        });
    }
    if id != super::memory::MODULE_ID {
        return serde_json::json!({});
    }
    serde_json::json!({
        "workspace_dir": config.workspace_dir,
        // The registry file the host writes `[[memory_sources]]` into. The
        // module used to derive `workspace_dir/config.toml`, a file that does
        // not exist, and answered `NotFound` for every host-registered source
        // on sync (openhuman#5820). Additive: an older module ignores it.
        "config_path": config.config_path,
        "memory": config.memory,
        "memory_tree": config.memory_tree,
        "scheduler_gate": config.scheduler_gate,
        "local_ai": config.local_ai,
        "embeddings_provider": config.embeddings_provider,
        "memory_provider": config.memory_provider,
        "default_model": config.default_model,
        "default_temperature": config.default_temperature,
        "output_language": config.output_language,
        "memory_sources": config.memory_sources,
        "embedding_routes": config.embedding_routes,
        "storage_provider": config.storage.provider.config,
        "ollama_base_url": crate::openhuman::inference::local::ollama_base_url_from_config(config),
        // The module's `EmbeddingHost::default_cloud_embedding_model`: what the
        // engine switches to when the opted-in local model is unreachable
        // (`store::factories`). That is the host's managed-cloud default, the
        // same constant the in-process `OpenHumanEmbeddingHost` answers with.
        // It is NOT `config.memory.embedding_model`, which is the user's
        // intended model and is usually the local one; sending that here made
        // the cloud fallback ask the managed embedder for `nomic-embed-text`
        // (openhuman#5820).
        "cloud_embedding_model":
            crate::openhuman::inference::embeddings::DEFAULT_CLOUD_EMBEDDING_MODEL,
        "cloud_embedding_dimensions":
            crate::openhuman::inference::embeddings::DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
        "models_supporting_dimensions":
            crate::openhuman::inference::embeddings::MODELS_SUPPORTING_DIMENSIONS,
        // The periodic composio and workspace-source sync loops run INSIDE the
        // module now (tinymemory#100), and these three are what let them run at
        // all. Without the cadence the module answers manual-only and skips
        // every source silently; without the mode `composio_config` never
        // selects its direct branch and every connection fails. All three are
        // `#[serde(default)]` upstream, so an older module ignores them rather
        // than failing to load.
        "memory_sync_interval_secs": config.memory_sync_interval_secs,
        "composio_mode": config.composio.mode,
        "composio_entity_id": config.composio.entity_id,
        // Proxied Composio addresses the backend with this; without it the module
        // builds its request against an empty base and fails in the HTTP client.
        "backend_api_url": crate::api::config::effective_backend_api_url(&config.api_url),
        "driver_id": "tinymemory",
    })
}

/// A configured local artifact for `id`, if one is set.
///
/// The test fixture uses the same explicit-override path as a developer build,
/// so TinyMemory is initialized with the host's real module configuration.
/// Loading it through `OPENHUMAN_MODULE_PATH` would initialize it during boot
/// before that configuration and its host callbacks are installed.
fn local_override(config: &Config, id: &str) -> Option<PathBuf> {
    let configured = config
        .modules
        .overrides
        .iter()
        .find_map(|entry| (entry.id == id).then(|| PathBuf::from(entry.path.clone())));

    configured
        .or_else(|| {
            (id == super::memory::MODULE_ID)
                .then(|| std::env::var_os("TINYMEMORY_TEST_MODULE"))
                .flatten()
                .map(PathBuf::from)
        })
        .or_else(|| {
            // TinyConnectors exposes its contract to the host, but has no
            // host-side module namespace: it is resolved by its registry ID.
            (id == "tinyconnectors")
                .then(|| std::env::var_os("TINYCONNECTORS_TEST_MODULE"))
                .flatten()
                .map(PathBuf::from)
        })
}

/// Where downloaded artifacts are kept.
///
/// The user cache directory, falling back to the workspace when there is none —
/// the same shape the Node and Python runtime installers use, for the same
/// reason: a headless container often has no `XDG_CACHE_HOME`, and failing to
/// install because of that would be worse than writing beside the workspace.
#[must_use]
pub fn install_dir(config: &Config) -> Option<PathBuf> {
    if let Some(configured) = &config.modules.install_dir {
        return Some(PathBuf::from(configured));
    }
    if let Some(cache) = dirs::cache_dir() {
        return Some(cache.join("openhuman").join("modules"));
    }
    log::warn!("[modules] no cache directory; installing modules under the workspace instead");
    Some(config.workspace_dir.join("modules"))
}

/// Status of every module this build knows about.
#[must_use]
pub fn list(config: &Config) -> Vec<ModuleStatus> {
    registry::ALL
        .iter()
        .map(|record| status_of(config, record))
        .collect()
}

/// Status of one module.
fn status_of(config: &Config, record: &ModuleRecord) -> ModuleStatus {
    // Configuration is authoritative for the current core instance. A module
    // may remain loaded in this process after a prior request, but callers
    // whose configuration disables modules must still be told it is unusable.
    let (state, detail) = match () {
        _ if !config.modules.enabled => (
            ModuleState::Unsupported,
            Some("modules are disabled in configuration".to_string()),
        ),
        _ => match resolution::table().peek(record.id) {
            ResolutionState::Ready => (ModuleState::Ready, None),
            ResolutionState::Loading => (ModuleState::Loading, None),
            ResolutionState::Failed(reason) => (ModuleState::Failed, Some(reason)),
            ResolutionState::Unresolved => {
                let supported = platform::host_candidates()
                    .iter()
                    .any(|key| record.asset_for(key).is_some());
                if supported {
                    (ModuleState::Available, None)
                } else {
                    (
                        ModuleState::Unsupported,
                        Some("no artifact is published for this platform".to_string()),
                    )
                }
            }
        },
    };
    ModuleStatus {
        id: record.id.to_string(),
        description: record.description.to_string(),
        version: record.version.to_string(),
        bus_name: record.bus_name.to_string(),
        state,
        detail,
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
