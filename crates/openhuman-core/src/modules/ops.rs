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
use std::sync::OnceLock;
use std::time::Duration;

use super::types::{ModuleRecord, ModuleState, ModuleStatus};
use super::{host, registry};
use crate::config::Config;
use tinybus::module::platform::host_candidates;
use tinybus::module::resolution::{self, Claim, Resolution, ResolutionState, Waited};
use tinybus::module::{load_first_admitted, prune_stale_versions, ReleaseAsset, ReleasePlan};

/// Environment variable naming a directory of bundled release archives, for
/// headless hosts (the Docker image, the CLI tarball, bench bundles).
pub const BUNDLED_MODULES_ENV: &str = "OPENHUMAN_BUNDLED_MODULES";

/// Directory name searched beside the executable when nothing else is set.
const BUNDLED_MODULES_DIR: &str = "bundled-modules";

/// Installer-owned, read-only release cache. The desktop host sets this before
/// starting the embedded core; headless hosts name it with
/// [`BUNDLED_MODULES_ENV`] or ship it beside the binary.
static BUNDLED_RELEASES: OnceLock<PathBuf> = OnceLock::new();

/// Register the directory of release archives shipped with the desktop app.
/// Its contents still pass the compiled digest and TinyBus admission gates.
pub fn set_bundled_releases_dir(path: PathBuf) -> Result<(), PathBuf> {
    BUNDLED_RELEASES.set(path)
}

/// The bundled release directory: the one the host registered, else
/// [`BUNDLED_MODULES_ENV`], else `bundled-modules/` beside the executable.
/// Only an existing directory counts; nothing here creates one.
fn bundled_releases_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    resolve_bundled_dir(
        BUNDLED_RELEASES.get().cloned(),
        std::env::var_os(BUNDLED_MODULES_ENV).map(PathBuf::from),
        exe_dir,
    )
}

pub(crate) fn resolve_bundled_dir(
    registered: Option<PathBuf>,
    from_env: Option<PathBuf>,
    exe_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    // Each candidate must be a directory to win: a stale registered path or a
    // mistyped env var must not hide a valid directory further down the list.
    let found = [
        registered,
        from_env,
        exe_dir.map(|dir| dir.join(BUNDLED_MODULES_DIR)),
    ]
    .into_iter()
    .flatten()
    .find(|dir| dir.is_dir());
    if let Some(dir) = &found {
        log::debug!("[modules] bundled release directory: {}", dir.display());
    }
    found
}

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

    let receiver = match resolution::global().claim(id) {
        Claim::Done(Resolution::Ready) => return Ok(()),
        Claim::Done(Resolution::Failed(reason)) => return Err(LoadError::Failed(reason)),
        Claim::Wait(receiver) => receiver,
        Claim::Run { sender, receiver } => {
            start_resolution(config.clone(), record, sender).await;
            receiver
        }
    };
    match resolution::global().wait(id, receiver, within).await {
        Waited::Ready => Ok(()),
        Waited::Failed(reason) => Err(LoadError::Failed(reason)),
        Waited::StillLoading => Err(LoadError::StillLoading),
    }
}

/// The state of `id` as the resolution table reports it, without touching it.
#[must_use]
pub fn state_of(id: &str) -> ModuleState {
    match resolution::global().peek(id) {
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
                // Every resolution failure is terminal for the process (the
                // outcome is cached), whichever path produced it; marking here
                // makes later re-reports classify as `ModuleUnavailable`.
                let reason = mark_terminal(reason);
                report_resolution_failure(id, &reason);
                Resolution::Failed(reason)
            }
        };
        resolution::global().complete(id, resolution, sender);
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

/// Report a module that did not load — once per process.
///
/// A resolution runs once and its failure is cached for every later caller
/// (see the module docs), so this is the single Sentry event a broken install
/// produces. Those callers re-raise the cached reason, and those re-reports
/// classify as `ExpectedErrorKind::ModuleUnavailable` and are demoted; this
/// one goes through [`report_error`] directly so the classifier cannot swallow
/// it too.
///
/// [`report_error`]: crate::core::observability::report_error
pub(super) fn report_resolution_failure(id: &str, reason: &str) {
    crate::core::observability::report_error(reason, "modules", "resolve", &[("module", id)]);
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
    let bundled = bundled_releases_dir();
    let outcome = blocking(move || {
        load_cached(
            runtime,
            record,
            &cache_root,
            module_config,
            allow_download,
            bundled.as_deref(),
        )
    })
    .await;
    match outcome {
        Ok(()) => {
            prune_stale_versions(&root, record.id, record.version);
            Ok(())
        }
        Err(reason) => Err(reason),
    }
}

/// Run a blocking module operation on the blocking pool.
///
/// A panic in the loader is reported rather than propagated: it would otherwise
/// take down whichever task happened to be awaiting the load.
///
/// Every loader error is terminal for the process and says so once: tinybus'
/// release-cache path and [`load_local`] already carry the marker, the rest
/// get it appended here.
async fn blocking<F>(work: F) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    host::runtime()
        .await
        .map_err(|error| mark_terminal(format!("the module bus could not start: {error}")))?
        .blocking(work)
        .await
        .map_err(mark_terminal)
}

/// Append the terminal-fault sentence to `error` unless it already has it.
pub(super) fn mark_terminal(error: String) -> String {
    let marker = crate::tools::status::MODULE_FAULT_MARKER;
    if error.contains(marker) {
        return error;
    }
    format!("{error}. {marker}; restart the app to try again")
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
    bundled_root: Option<&Path>,
) -> Result<(), String> {
    let assets: Vec<ReleaseAsset<'static>> = host_candidates()
        .iter()
        .filter_map(|key| record.asset_for(key))
        .map(|asset| ReleaseAsset {
            host_key: asset.host_key,
            archive: asset.archive,
            sha256: asset.sha256,
        })
        .collect();
    let plan = ReleasePlan {
        id: record.id,
        version: record.version,
        release_url: record.release_url,
        assets: &assets,
        install_root,
        bundled_root,
        allow_download,
    };
    load_first_admitted(runtime.host(), &plan, &module_config).map(|_| ())
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
    if id == super::search::MODULE_ID {
        return serde_json::to_value(super::search::module_config(config))
            .expect("TinySearch config serializes");
    }
    if id == super::desktop::MODULE_ID {
        return super::desktop::module_config(config);
    }
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
    serde_json::json!({})
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
            (id == super::search::MODULE_ID)
                .then(|| std::env::var_os("TINYSEARCH_TEST_MODULE"))
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
/// the same shape used by other module asset installers, for the same
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
        _ => match resolution::global().peek(record.id) {
            ResolutionState::Ready => (ModuleState::Ready, None),
            ResolutionState::Loading => (ModuleState::Loading, None),
            ResolutionState::Failed(reason) => (ModuleState::Failed, Some(reason)),
            ResolutionState::Unresolved => {
                let supported = host_candidates()
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
