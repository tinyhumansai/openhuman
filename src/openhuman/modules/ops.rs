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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::types::{ModuleRecord, ModuleState, ModuleStatus};
use super::{host, platform, registry};
use crate::openhuman::config::Config;

/// Outcome of resolving one module, remembered for the process lifetime.
#[derive(Debug, Clone)]
enum Resolution {
    Ready,
    /// Terminal. Carries the sanitised reason, never a path or URL.
    Failed(String),
}

/// Per-module resolution results.
fn resolutions() -> &'static Mutex<HashMap<String, Resolution>> {
    static RESOLUTIONS: OnceLock<Mutex<HashMap<String, Resolution>>> = OnceLock::new();
    RESOLUTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Serialises resolution so two concurrent tool calls do not both download.
///
/// Coarse on purpose — one lock for all modules rather than one per module.
/// Resolution happens at most once per module per process, so contention is
/// bounded by the number of modules, and a per-module lock table would be more
/// machinery than the problem justifies.
fn resolve_gate() -> &'static tokio::sync::Mutex<()> {
    static GATE: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    GATE.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Ensure `id` is loaded and serving, loading it if this is the first ask.
///
/// Resolution order is cheapest-first: already loaded, then an artifact already
/// on disk, then the module search path, then a verified download.
///
/// # Errors
///
/// Returns a message suitable for surfacing to a user or a model when the module
/// cannot be loaded: no artifact for this host, downloads disabled, a refused
/// artifact, or a previous failure in this process.
pub async fn ensure_loaded(config: &Config, id: &str) -> Result<(), String> {
    if !config.modules.enabled {
        return Err(format!(
            "module '{id}' is unavailable: modules are disabled in configuration"
        ));
    }
    let record = registry::find(id).ok_or_else(|| format!("unknown module '{id}'"))?;

    if let Some(cached) = cached_resolution(id) {
        return match cached {
            Resolution::Ready => Ok(()),
            Resolution::Failed(reason) => Err(reason),
        };
    }

    // One resolver at a time. Two chat turns asking for a document at once must
    // not both download the same archive.
    let _guard = resolve_gate().lock().await;
    // Re-check: the winner of the race may have resolved it while we waited.
    if let Some(cached) = cached_resolution(id) {
        return match cached {
            Resolution::Ready => Ok(()),
            Resolution::Failed(reason) => Err(reason),
        };
    }

    let outcome = resolve(config, record).await;
    let resolution = match &outcome {
        Ok(()) => Resolution::Ready,
        Err(reason) => Resolution::Failed(reason.clone()),
    };
    if let Ok(mut cache) = resolutions().lock() {
        cache.insert(id.to_string(), resolution);
    }
    outcome
}

/// The remembered outcome for `id`, if it has been resolved.
fn cached_resolution(id: &str) -> Option<Resolution> {
    resolutions().lock().ok()?.get(id).cloned()
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

    // An artifact already extracted into the install directory by an earlier run.
    if let Some(path) = installed_artifact(config, record) {
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

    if !config.modules.allow_download {
        return Err(format!(
            "module '{}' is unavailable: no local artifact is installed and downloads are \
             disabled in configuration",
            record.id
        ));
    }

    // Off the runtime worker. `load_github_release` fetches over the network,
    // hashes the archive, extracts it and `dlopen`s the result — all
    // synchronously. Left inline it would stall every other task sharing this
    // worker for the length of a download on whatever link the user has.
    let module_config = module_config(config, record.id);
    blocking(move || download(runtime, record, module_config)).await
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

/// Download, verify, and load the pinned release artifact for this host.
fn download(
    runtime: &'static host::ModuleRuntime,
    record: &'static ModuleRecord,
    module_config: serde_json::Value,
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

    // Try the preferred artifact first and fall through on admission failure —
    // a host newer than the newest published build runs that build, and one
    // whose toolchain the newest artifact does not match falls back.
    let mut last_error = String::new();
    for asset in assets {
        match runtime.host().load_github_release(
            record.release_url,
            asset.archive,
            Some(asset.sha256),
            module_config.clone(),
        ) {
            Ok(_) => {
                log::info!(
                    "[modules] loaded '{}' {} from the pinned release ({})",
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

/// The artifact an earlier run extracted, if it is still there.
fn installed_artifact(config: &Config, record: &ModuleRecord) -> Option<PathBuf> {
    let dir = install_dir(config)?.join(record.id).join(record.version);
    let candidate = dir.join(platform_library_name(record.id));
    candidate.is_file().then_some(candidate)
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

/// The platform library file name for a module id.
fn platform_library_name(id: &str) -> String {
    let stem = id.replace('-', "_");
    if cfg!(target_os = "windows") {
        format!("{stem}_module.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{stem}_module.dylib")
    } else {
        format!("lib{stem}_module.so")
    }
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
        _ => match cached_resolution(record.id) {
            Some(Resolution::Ready) => (ModuleState::Ready, None),
            Some(Resolution::Failed(reason)) => (ModuleState::Failed, Some(reason)),
            None => {
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
