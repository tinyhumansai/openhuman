//! The host adapter over `tinybus::module::Resolver`.
//!
//! The resolver ladder (already loaded, override, search path, release cache,
//! prune), the once-per-process resolution slots, failure caching, the module
//! runtime and the client all live in `tinybus::module`. This file is only what
//! OpenHuman contributes to them:
//!
//! - [`resolver`] turns a [`Config`] and the compiled [`registry`] into the
//!   resolver's plain inputs: the install directory, whether a cache miss may
//!   reach the network, the source each module is served from (see
//!   [`super::source`]), the `[components]` gate, per-module initialisation
//!   configuration and the shared Sentry reporter ([`super::report`]).
//! - [`client`] is a `tinybus::module::ModuleClient` over that resolver.
//! - [`list`] and [`state_of`] shape the resolver's answers into the
//!   `modules.list` RPC output.
//!
//! # Failures are cached, not retried
//!
//! tinybus never unloads a library. A module that was refused, faulted, or
//! failed to initialise keeps whatever it mapped, and loading it again cannot
//! reach a different outcome without a restart, so the resolver caches the
//! failure and returns it directly. The user-visible consequence is that fixing
//! a module means restarting the core, which the error says.
//!
//! The first caller for a module decides its resolution; later callers, with
//! whatever configuration they hold, wait on that outcome.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use super::types::{ModuleRecord, ModuleSource, ModuleState, ModuleStatus};
use super::{linked, registry, report, runtime, source};
use crate::config::Config;
use tinybus::module::{
    Availability, LoadError, ModuleClient, ModuleSource as ResolverSource, Resolver, ResolverInputs,
};

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

/// Library file stems of every module whose component is off, for skipping
/// their search-path artifacts. Both `name` and `libname` spellings.
#[must_use]
pub fn disabled_lib_stems(config: &Config) -> Vec<String> {
    registry::ALL
        .iter()
        .filter(|record| disabled_component(config, record.id).is_some())
        .flat_map(|record| {
            let stem = record.source.lib_name.to_string();
            [format!("lib{stem}"), stem]
        })
        .collect()
}

/// The disabled `[components]` switch that owns module `id`, if any.
#[must_use]
pub fn disabled_component(config: &Config, id: &str) -> Option<&'static str> {
    let component = registry::component_for(id)?;
    let set = crate::core::runtime::ComponentSet::from_config(&config.components);
    (!set.enabled(component)).then_some(component)
}

/// A resolver over the compiled registry, configured from `config`.
///
/// Cheap to build and to drop: the records are compiled constants, and the
/// resolution table, the runtime and the reporter are process-wide. Building
/// one starts nothing.
#[must_use]
pub fn resolver(config: &Config) -> Resolver {
    let shared = Arc::new(config.clone());
    let mut sources = std::collections::HashMap::new();
    let mut linked_factories = std::collections::HashMap::new();
    for record in registry::ALL {
        let plan = source::plan(config, record.id);
        log_plan_once(record, &plan);
        match plan.served {
            ModuleSource::LocalFile | ModuleSource::Source => {
                if let Some(path) = plan.artifact {
                    sources.insert(
                        record.id.to_string(),
                        ResolverSource::LocalFile {
                            path: path.to_string_lossy().into_owned(),
                        },
                    );
                }
            }
            ModuleSource::Linked => {
                sources.insert(record.id.to_string(), ResolverSource::Linked);
                if let Some(factory) = linked::factory_for(record.id) {
                    linked_factories.insert(record.id.to_string(), factory);
                }
            }
            ModuleSource::Release => {}
        }
    }
    let gate_config = Arc::clone(&shared);
    Resolver::new(ResolverInputs {
        enabled: config.modules.enabled,
        install_dir: install_dir(config),
        allow_download: config.modules.allow_download,
        bundled_dir: bundled_releases_dir(),
        sources,
        linked: linked_factories,
        search_skip: disabled_lib_stems(config),
        gate: Some(Arc::new(move |id| disabled_component(&gate_config, id))),
        on_resolved: Some(Arc::new(on_resolved)),
        records: registry::release_records(),
        terminal_marker: tinytools::status::MODULE_FAULT_MARKER.to_string(),
        runtime: Arc::new(|| Box::pin(runtime::runtime())),
        module_config: Arc::new(move |id| module_config(&shared, id)),
        reporter: report::reporter(),
    })
}

/// A `tinybus::module::ModuleClient` that loads modules lazily through
/// [`resolver`].
///
/// Constructing a client starts nothing. It needs neither a signed-in session
/// nor a core runtime. Set the bundled releases directory through the module
/// facade before the first call when artifacts ship with the host.
#[must_use]
pub fn client(config: Config) -> ModuleClient {
    ModuleClient::new(resolver(&config))
}

/// A client whose every call goes to `proxy` instead of loading a module. The
/// module must still be in the registry and modules must still be enabled.
#[cfg(test)]
#[must_use]
pub(crate) fn fixture_client(proxy: tinybus::Proxy) -> ModuleClient {
    client(Config::default()).with_fixture(proxy)
}

/// Log which source serves each module and warn about a local build whose
/// gitlink is ahead of its released pin, once per id per process.
fn log_plan_once(record: &ModuleRecord, plan: &source::SourcePlan) {
    static LOGGED: OnceLock<Mutex<std::collections::HashSet<&'static str>>> = OnceLock::new();
    let first = LOGGED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(record.id);
    if !first {
        return;
    }
    if let Some(reason) = plan.fallback {
        log::warn!(
            "[modules] id={} requested={} served=release: {reason}",
            record.id,
            plan.requested.as_str()
        );
    }
    log::debug!(
        "[modules] id={} requested={} served={:?}",
        record.id,
        plan.requested.as_str(),
        plan.served
    );
    if let Some(line) = source::pin_ahead_warning(
        record.id,
        plan.served,
        std::env::var(source::PIN_AHEAD_ENV).ok().as_deref(),
        cfg!(debug_assertions),
    ) {
        log::warn!("{line}");
    }
}

/// The connector module's curated catalogs are cached in the host; a module
/// that has just been (re)resolved may serve other tables, so drop the snapshot
/// before any waiting caller is released. The next connector call (or the boot
/// warm-up) reads it again.
fn on_resolved(id: &str) {
    if id == super::connectors::MODULE_ID {
        crate::integrations::composio::contract::curated::invalidate();
    }
}

/// The message for a caller that cannot distinguish the cases. A switched-off
/// component keeps its typed `COMPONENT_DISABLED:<name>` spelling.
#[must_use]
pub fn load_error_message(error: LoadError) -> String {
    match error {
        LoadError::Disabled(component) => format!("COMPONENT_DISABLED:{component}"),
        other => other.into_message(),
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
        .map_err(load_error_message)
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
/// [`LoadError::StillLoading`] when `within` passed first;
/// [`LoadError::Disabled`] when its `[components]` switch is off.
pub async fn ensure_loaded_within(
    config: &Config,
    id: &str,
    within: Option<Duration>,
) -> Result<(), LoadError> {
    resolver(config).ensure_loaded_within(id, within).await
}

/// The state of `id` as the resolution table reports it, without touching it.
#[must_use]
pub fn state_of(id: &str) -> ModuleState {
    use tinybus::module::resolution::{self, ResolutionState};
    match resolution::global().peek(id) {
        ResolutionState::Unresolved => ModuleState::Available,
        ResolutionState::Loading => ModuleState::Loading,
        ResolutionState::Ready => ModuleState::Ready,
        ResolutionState::Failed(_) => ModuleState::Failed,
    }
}

/// Configuration crossing into a first-party compiled module.
///
/// Credentials are intentionally absent. TinyMemory calls back into the host
/// for embedding and chat compute; the other modules need no host config.
fn module_config(config: &Config, id: &str) -> serde_json::Value {
    #[cfg(feature = "security-module")]
    if id == super::security::MODULE_ID {
        return serde_json::to_value(super::security::module_config(config))
            .expect("TinySecurity config serializes");
    }
    if id == super::search::MODULE_ID {
        return super::search::load_configuration(config);
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
            serde_json::json!({ "offered_toolkits": super::connectors::offered_toolkits(config) })
        });
    }
    serde_json::json!({})
}

/// [`module_config`] for a test in a sibling module.
#[cfg(test)]
pub(super) fn module_config_for_test(config: &Config, id: &str) -> serde_json::Value {
    module_config(config, id)
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
    let (state, detail) = match disabled_component(config, record.id) {
        Some(component) => (
            ModuleState::Disabled,
            Some(format!(
                "COMPONENT_DISABLED:{component} (set [components] {component} = true to enable)"
            )),
        ),
        None => {
            let (availability, detail) = resolver(config).status_of(&record.release());
            (
                match availability {
                    Availability::Available => ModuleState::Available,
                    Availability::Loading => ModuleState::Loading,
                    Availability::Ready => ModuleState::Ready,
                    Availability::Failed => ModuleState::Failed,
                    Availability::Unsupported => ModuleState::Unsupported,
                    Availability::Disabled => ModuleState::Disabled,
                },
                detail,
            )
        }
    };
    let plan = source::plan(config, record.id);
    let source =
        (!matches!(state, ModuleState::Unsupported | ModuleState::Disabled)).then_some(plan.served);
    let detail = detail.or_else(|| plan.fallback.map(str::to_string));
    ModuleStatus {
        id: record.id.to_string(),
        description: record.description.to_string(),
        version: record.version.to_string(),
        bus_name: record.bus_name.to_string(),
        state,
        source,
        detail,
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ops_components_tests.rs"]
mod components_tests;

#[cfg(test)]
#[path = "ops_client_tests.rs"]
mod client_tests;

#[cfg(test)]
#[path = "ops_resolver_tests.rs"]
mod resolver_tests;
