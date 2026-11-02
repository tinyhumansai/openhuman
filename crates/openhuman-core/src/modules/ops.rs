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
    AccessFuture, Availability, FailureReporter, LinkedRegistry, LoadError, ModuleAccess,
    ModuleClient, ModuleSource as ResolverSource, ProxyFailure, ReleaseRecord, Resolver,
    ResolverInputs, SearchPathConfig,
};
use tinybus::Proxy;

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
/// their search-path artifacts. Both `name` and `libname` spellings, for the
/// source build (`lib_name`) and the release download (`<id>_module`).
#[must_use]
pub fn disabled_lib_stems(config: &Config) -> Vec<String> {
    registry::ALL
        .iter()
        .filter(|record| disabled_component(config, record.id).is_some())
        .flat_map(|record| {
            let stem = record.source.lib_name.to_string();
            let released = format!("{}_module", record.id);
            [
                format!("lib{stem}"),
                stem,
                format!("lib{released}"),
                released,
            ]
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
    let linked_factories = Arc::new(LinkedRegistry::new());
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
                    linked_factories.register(record.id, factory);
                }
            }
            ModuleSource::Release if requires_verified_local_callbacks(record.id) => {
                // These modules require a per-module storage callback, which
                // TinyBus can bind only on its verified local-file path.
                // Route a staged release through that rung; the callback-aware
                // client below supplies the unique destination at load time.
                if let Some(path) = source::local_artifact(record.id) {
                    sources.insert(
                        record.id.to_string(),
                        ResolverSource::LocalFile {
                            path: path.to_string_lossy().into_owned(),
                        },
                    );
                }
            }
            ModuleSource::Release => {}
        }
    }
    let gate_config = Arc::clone(&shared);
    let module_config_source = Arc::clone(&shared);
    let mut search_skip = disabled_lib_stems(config);
    search_skip.extend(super::search_path::configured_module_lib_stems());
    Resolver::new(ResolverInputs {
        enabled: config.modules.enabled,
        install_dir: install_dir(config),
        allow_download: config.modules.allow_download,
        bundled_dir: bundled_releases_dir(),
        sources,
        linked: linked_factories,
        search_paths: Some(SearchPathConfig::new(
            "OPENHUMAN_MODULE_PATH",
            "openhuman/modules",
        )),
        search_skip,
        gate: Some(Arc::new(move |id| disabled_component(&gate_config, id))),
        on_resolved: Some(Arc::new(on_resolved)),
        records: registry::release_records(),
        terminal_marker: tinytools::status::MODULE_FAULT_MARKER.to_string(),
        runtime: Arc::new(|| Box::pin(runtime::runtime())),
        module_config: Arc::new(|_| serde_json::Value::Null),
        async_module_config: Some(Arc::new(move |id| {
            let config = Arc::clone(&module_config_source);
            Box::pin(async move {
                let value = runtime_module_config(&config, id).await;
                if id == super::connectors::MODULE_ID {
                    super::connectors::note_module_config_for_load(&config, &value);
                }
                value
            })
        })),
        reporter: report::reporter(),
    })
}

fn requires_verified_local_callbacks(id: &str) -> bool {
    matches!(id, "tinysecurity" | "tinymcp" | "tinyeventqueues")
}

/// The staged artifact the callback-bound module fixture loads in isolation.
#[cfg(test)]
pub(crate) fn local_artifact_for_test(id: &str) -> Option<std::path::PathBuf> {
    source::local_artifact(id)
}

/// A `tinybus::module::ModuleClient` that loads modules lazily through
/// [`resolver`].
///
/// Constructing a client starts nothing. It needs neither a signed-in session
/// nor a core runtime. Set the bundled releases directory through the module
/// facade before the first call when artifacts ship with the host.
#[must_use]
pub fn client(config: Config) -> ModuleClient {
    ModuleClient::new(CallbackAccess {
        resolver: resolver(&config),
        config,
    })
}

/// The modules that connect to storage through a capability scoped to
/// their exact verified module instance.
fn needs_storage_callback(id: &str) -> bool {
    matches!(id, "tinymcp" | "tinyeventqueues" | "tinychannels")
}

/// Serializes creation and retention of module-scoped storage callback bindings.
type StorageCallbackLoads = tokio::sync::Mutex<
    std::collections::HashMap<
        String,
        (
            std::sync::Arc<tinybus::module::HostCallbackBinding>,
            tinystoragedrivers_busproxy::ServedStorage,
        ),
    >,
>;

static STORAGE_CALLBACK_LOADS: std::sync::LazyLock<StorageCallbackLoads> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));

async fn ensure_loaded_with_storage_callback(
    resolver: &Resolver,
    config: &Config,
    id: &str,
) -> Result<(), String> {
    let mut loads = STORAGE_CALLBACK_LOADS.lock().await;
    if let Some((binding, _served)) = loads.get(id) {
        if matches!(state_of(id), ModuleState::Ready) {
            // Already serving with this binding: a binding is spent once the
            // module takes it, so re-presenting it would be refused. The module
            // is process-wide, so every later workspace's client finds it
            // already serving.
            return Ok(());
        }
        let mut module_config = module_config(config, id).await;
        module_config["storage_callback"] = serde_json::json!(binding.unique_name().as_str());
        return resolver
            .ensure_loaded_with_host_callbacks(id, std::sync::Arc::clone(binding), module_config)
            .await;
    }

    let runtime = resolver
        .runtime()
        .await
        .map_err(|error| error.to_string())?;
    let (binding, served) = super::storage_host::prepare_module_callbacks(runtime)
        .await
        .map_err(|error| error.to_string())?;
    let mut module_config = module_config(config, id).await;
    module_config["storage_callback"] = serde_json::json!(binding.unique_name().as_str());
    resolver
        .ensure_loaded_with_host_callbacks(id, std::sync::Arc::clone(&binding), module_config)
        .await?;
    loads.insert(id.to_owned(), (binding, served));
    Ok(())
}

#[cfg(feature = "security-module")]
type ComputerCallbackLoads = tokio::sync::Mutex<
    std::collections::HashMap<String, std::sync::Arc<tinybus::module::HostCallbackBinding>>,
>;

#[cfg(feature = "security-module")]
static COMPUTER_CALLBACK_LOADS: std::sync::LazyLock<ComputerCallbackLoads> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));

#[cfg(feature = "security-module")]
pub(super) async fn security_callback_binding(
) -> Option<std::sync::Arc<tinybus::module::HostCallbackBinding>> {
    super::security::host_callbacks::binding().await
}

#[cfg(feature = "security-module")]
pub(crate) async fn security_cipher_consumer(
) -> Option<std::sync::Arc<super::security::cipher_consumer::Consumer>> {
    super::security::host_callbacks::cipher_consumer().await
}

#[cfg(feature = "security-module")]
pub(crate) async fn security_secret_backend(
) -> Option<std::sync::Arc<super::security::secret_backend::SecretBackend>> {
    super::security::host_callbacks::secret_backend().await
}

#[cfg(feature = "security-module")]
pub(crate) async fn security_secret_scope() -> Option<crate::security::approval::native::SecretScope>
{
    super::security::host_callbacks::secret_scope().await
}

#[cfg(feature = "security-module")]
pub(crate) async fn security_secret_registration_id() -> Option<String> {
    super::security::host_callbacks::secret_registration_id().await
}

#[cfg(feature = "security-module")]
pub(crate) async fn security_callback_identity() -> Option<String> {
    super::security::host_callbacks::identity().await
}

#[cfg(feature = "security-module")]
pub(super) async fn security_module_config(config: &Config) -> Result<serde_json::Value, String> {
    super::security::host_callbacks::module_config(config).await
}

#[cfg(feature = "security-module")]
async fn ensure_loaded_with_security_callback(
    resolver: &Resolver,
    config: &Config,
    id: &str,
) -> Result<(), String> {
    super::security::host_callbacks::ensure_loaded(resolver, config, id).await
}

#[cfg(feature = "security-module")]
async fn ensure_loaded_with_computer_callback(
    resolver: &Resolver,
    config: &Config,
    id: &str,
) -> Result<(), String> {
    let mut loads = COMPUTER_CALLBACK_LOADS.lock().await;
    if let Some(binding) = loads.get(id) {
        let module_config = module_config_with_consequence_callback(config, id, binding).await;
        return resolver
            .ensure_loaded_with_host_callbacks(id, std::sync::Arc::clone(binding), module_config)
            .await;
    }

    let runtime = resolver
        .runtime()
        .await
        .map_err(|error| error.to_string())?;
    let binding = super::browser_egress_host::prepare_consequence_callback(runtime)
        .await
        .map_err(|error| error.to_string())?;
    let module_config = module_config_with_consequence_callback(config, id, &binding).await;
    resolver
        .ensure_loaded_with_host_callbacks(id, std::sync::Arc::clone(&binding), module_config)
        .await?;
    loads.insert(id.to_owned(), binding);
    Ok(())
}

#[cfg(feature = "security-module")]
async fn module_config_with_consequence_callback(
    config: &Config,
    id: &str,
    binding: &tinybus::module::HostCallbackBinding,
) -> serde_json::Value {
    let mut module_config = module_config(config, id).await;
    module_config["consequence_host"] =
        serde_json::json!({ "destination": binding.unique_name().as_str() });
    module_config
}

#[cfg(feature = "security-module")]
pub(super) async fn consequence_callback_destination(id: &str) -> Option<String> {
    COMPUTER_CALLBACK_LOADS
        .lock()
        .await
        .get(id)
        .map(|binding| binding.unique_name().to_string())
}

/// Module access that grants storage callbacks only while loading the two
/// modules whose contracts require them.
struct CallbackAccess {
    resolver: Resolver,
    config: Config,
}

impl ModuleAccess for CallbackAccess {
    fn find(&self, id: &str) -> Option<&'static ReleaseRecord> {
        self.resolver.find(id)
    }

    fn enabled(&self) -> bool {
        self.resolver.enabled()
    }

    fn reporter(&self) -> &FailureReporter {
        self.resolver.reporter()
    }

    fn ensure_loaded<'a>(&'a self, id: &'a str) -> AccessFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let loaded = if needs_storage_callback(id) {
                ensure_loaded(&self.config, id).await
            } else {
                self.resolver.ensure_loaded(id).await
            };
            // The client collapses this message into a bare "module
            // unavailable"; keep the reason where an operator can grep it.
            if let Err(error) = &loaded {
                tracing::warn!(module = id, %error, "[modules] ensure_loaded refused the module");
            }
            loaded
        })
    }

    fn proxy<'a>(
        &'a self,
        bus_name: &'a str,
        object_path: &'a str,
    ) -> AccessFuture<'a, Result<Proxy, ProxyFailure>> {
        self.resolver.proxy(bus_name, object_path)
    }
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
pub fn ensure_loaded<'a>(
    config: &'a Config,
    id: &'a str,
) -> futures::future::BoxFuture<'a, Result<(), String>> {
    Box::pin(async move {
        ensure_loaded_within(config, id, None)
            .await
            .map_err(load_error_message)
    })
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
pub fn ensure_loaded_within<'a>(
    config: &'a Config,
    id: &'a str,
    within: Option<Duration>,
) -> futures::future::BoxFuture<'a, Result<(), LoadError>> {
    Box::pin(async move {
        let resolver = resolver(config);
        let result = if needs_storage_callback(id) {
            if let Some(component) = disabled_component(config, id) {
                Err(LoadError::Disabled(component))
            } else {
                let load = ensure_loaded_with_storage_callback(&resolver, config, id);
                match within {
                    Some(within) => tokio::time::timeout(within, load)
                        .await
                        .map_err(|_| LoadError::StillLoading)?
                        .map_err(LoadError::Failed),
                    None => load.await.map_err(LoadError::Failed),
                }
            }
        } else {
            #[cfg(feature = "security-module")]
            {
                if id == super::security::MODULE_ID {
                    if let Some(component) = disabled_component(config, id) {
                        Err(LoadError::Disabled(component))
                    } else {
                        let load = ensure_loaded_with_security_callback(&resolver, config, id);
                        match within {
                            Some(within) => tokio::time::timeout(within, load)
                                .await
                                .map_err(|_| LoadError::StillLoading)?
                                .map_err(LoadError::Failed),
                            None => load.await.map_err(LoadError::Failed),
                        }
                    }
                } else if id == super::desktop::MODULE_ID {
                    if let Some(component) = disabled_component(config, id) {
                        Err(LoadError::Disabled(component))
                    } else {
                        let load = ensure_loaded_with_computer_callback(&resolver, config, id);
                        match within {
                            Some(within) => tokio::time::timeout(within, load)
                                .await
                                .map_err(|_| LoadError::StillLoading)?
                                .map_err(LoadError::Failed),
                            None => load.await.map_err(LoadError::Failed),
                        }
                    }
                } else {
                    resolver.ensure_loaded_within(id, within).await
                }
            }
            #[cfg(not(feature = "security-module"))]
            {
                resolver.ensure_loaded_within(id, within).await
            }
        };
        if id == super::connectors::MODULE_ID && result.is_ok() {
            super::connectors::note_load_resolution_ready();
        }
        #[cfg(feature = "security-module")]
        if id == super::security::MODULE_ID && result.is_ok() {
            super::security::text::arm(config);
        }
        result
    })
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
/// Credentials and callback destinations are intentionally absent. TinyMemory
/// receives only its compute configuration; TinyMCP and TinyEventQueues get a
/// unique storage callback destination from the verified loader.
pub(super) fn module_config<'a>(
    config: &'a Config,
    id: &'a str,
) -> futures::future::BoxFuture<'a, serde_json::Value> {
    Box::pin(async move {
        #[cfg(feature = "security-module")]
        if id == super::security::MODULE_ID {
            return serde_json::to_value(super::security::module_config(config))
                .expect("TinySecurity module config is serializable");
        }
        if id == super::search::MODULE_ID {
            return super::search::load_configuration(config).await;
        }
        if id == super::desktop::MODULE_ID {
            return super::desktop::module_config(config).await;
        }
        // The queue manifest and pipeline the inbound event queues run; the
        // storage host declares the same manifest on the module's scope.
        #[cfg(feature = "inbound-queues")]
        if id == super::event_queues::MODULE_ID {
            return crate::inbound::module_config(config);
        }
        if id == super::connectors::MODULE_ID {
            // The connector module takes its route and credential from here and
            // reads one from nowhere else. A configuration that cannot be built —
            // direct mode with no key, an unknown mode — loads the module with an
            // empty blob rather than failing the load: the capability members need
            // no route and must still answer, and every member that does need one
            // reports the missing route when it is called.
            return super::connectors::module_config(config).await.unwrap_or_else(|error| {
            tracing::info!(
                error = %error,
                "[connectors] no route configured; loading with the capability surface only"
            );
            serde_json::json!({ "offered_toolkits": super::connectors::offered_toolkits(config) })
        });
        }
        serde_json::json!({})
    })
}

/// Configuration passed to the loader after it has started the module bus.
/// TinySecurity binds its privileged API to this process-wide host connection;
/// a missing identity must fail at initialization rather than leave a loaded
/// module whose every call is rejected.
async fn runtime_module_config(config: &Config, id: &str) -> serde_json::Value {
    #[cfg(feature = "security-module")]
    if id == super::security::MODULE_ID {
        let trusted_host = super::runtime::existing_runtime()
            .and_then(|runtime| runtime.connection().unique_name())
            .map(|name| name.to_string());
        let config = super::security::module_config_for_host(config, trusted_host)
            .expect("module runtime must expose its host identity before loading modules");
        return serde_json::to_value(config).expect("TinySecurity module config is serializable");
    }
    module_config(config, id).await
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
