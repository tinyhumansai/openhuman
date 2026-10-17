//! [`RuntimeBuilder::build`]: the boot sequence.

use std::path::Path;
use std::sync::Arc;

use openhuman_core::config::Config;
use openhuman_core::core::runtime::CoreBuilder;
use openhuman_core::core::types::HostKind;
use tinyagents_session::port::SessionStoreProvider;

use super::builder::{starts_services_on_build, ConfigSource};
use super::presets::{default_domains, default_services};
use super::{ApiKey, Runtime, RuntimeBuilder, RuntimeError, RUNTIME_LIVE};
use crate::harness::workspace::ResolvedWorkspace;
use crate::harness::{Provider, Workspace};
use crate::Core;

struct SessionStoreCleanup {
    installed: Option<Arc<dyn SessionStoreProvider>>,
    previous: Option<Arc<dyn SessionStoreProvider>>,
}

impl Drop for SessionStoreCleanup {
    fn drop(&mut self) {
        let Some(installed) = self.installed.take() else {
            return;
        };
        if openhuman_core::agent::session_store::clear_if(&installed) {
            openhuman_core::agent::session_store::restore(self.previous.take());
        }
    }
}

/// Releases [`RUNTIME_LIVE`] unless disarmed: held across
/// [`RuntimeBuilder::build`]'s boot so a failed or cancelled build never
/// leaves the process slot claimed.
struct SlotClaim {
    armed: bool,
}

impl Drop for SlotClaim {
    fn drop(&mut self) {
        if self.armed {
            RUNTIME_LIVE.store(false, std::sync::atomic::Ordering::Release);
            log::debug!("[embed][runtime] build did not complete; process slot released");
        }
    }
}

impl RuntimeBuilder {
    /// Build the core and return a runtime ready to host agents.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::AlreadyRunning`] if this process already has one;
    /// [`RuntimeError::BlankApiKey`] for an empty key;
    /// [`RuntimeError::Invalid`] for a knob the [`ConfigSource`] cannot
    /// honour or a refused controller extension.
    pub async fn build(self) -> Result<Runtime, RuntimeError> {
        // Claim the process slot before doing any work, so a losing racer
        // neither creates a temp dir nor half-initializes global state.
        if RUNTIME_LIVE.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Err(RuntimeError::AlreadyRunning);
        }
        // From here on every early return must release the slot, or a failed
        // build would permanently poison the process against retrying. The
        // claim is a drop guard rather than a match on the result so that a
        // *cancelled* build (the future dropped mid-boot, as the desktop
        // shell's startup timeout does to its server task) releases it too.
        let mut claim = SlotClaim { armed: true };
        let result = self.build_inner().await;
        if result.is_ok() {
            // The runtime's `CoreGuard` owns the slot from here.
            claim.armed = false;
        }
        result
    }

    /// Refuse combinations the chosen [`ConfigSource`] cannot honour.
    pub(super) fn validate(&self) -> Result<(), RuntimeError> {
        self.validate_modules()?;
        if self.agent_defaults.skills.root.is_some() && !cfg!(feature = "skills") {
            return Err(RuntimeError::Invalid(
                "copying runtime skill bundles requires Cargo feature `openhuman-embed/skills`"
                    .into(),
            ));
        }

        validate_storage_feature(self.seams.storage.as_ref())?;
        if self.seams.storage.is_none() {
            if let Some(config) = &self.config {
                if let Some(url) = openhuman_core::storage::explicit_url(config) {
                    validate_storage_feature(Some(&super::StorageSource::Url(url)))?;
                }
            }
        }
        self.agent_defaults
            .model
            .validate()
            .map_err(RuntimeError::Invalid)?;
        if self.api_key.as_ref().is_some_and(ApiKey::is_blank) {
            return Err(RuntimeError::BlankApiKey);
        }
        if matches!(self.workspace, Workspace::Stateless)
            && self.session_store.is_none()
            && self.seams.storage.is_none()
            && self
                .config
                .as_ref()
                .and_then(openhuman_core::storage::explicit_url)
                .is_none()
        {
            return Err(RuntimeError::NoSessionStore);
        }
        if self.config_source == ConfigSource::Discovered {
            if !matches!(self.workspace, Workspace::Inherit) {
                return Err(RuntimeError::Invalid(
                    "ConfigSource::Discovered needs Workspace::Inherit: the core discovers \
                     the operator's install"
                        .into(),
                ));
            }
            let edits = [
                ("config", self.config.is_some()),
                ("backend_url", self.backend_url.is_some()),
                ("workspace_dir", self.workspace_dir.is_some()),
                ("action_dir", self.action_dir.is_some()),
                ("api_key", self.api_key.is_some()),
                ("typed configuration", self.config_knobs.is_set()),
            ];
            if let Some((knob, _)) = edits.iter().find(|(_, set)| *set) {
                return Err(RuntimeError::Invalid(format!(
                    "`{knob}` edits the boot config, which ConfigSource::Discovered leaves \
                     to the core; use ConfigSource::Resolved"
                )));
            }
        }
        Ok(())
    }

    async fn build_inner(mut self) -> Result<Runtime, RuntimeError> {
        self.validate()?;
        let discovered = self.config_source == ConfigSource::Discovered;
        let inherit = self.workspace.is_operator_owned();
        let resolved = ResolvedWorkspace::resolve(&self.workspace, self.action_dir.as_deref())
            .map_err(map_ws)?;

        // Discovered: the core loads its own config during boot; nothing to
        // assemble here. Resolved: `Inherit` starts from the operator's own
        // config — loaded here rather than left to the core to discover,
        // because the other knobs are applied *on top* of it.
        let mut config = if discovered {
            None
        } else {
            Some(self.resolve_config(&resolved).await?)
        };

        // The seams are the last fallible step before side effects a failed
        // build cannot undo (the stored API key, the host memory engine), so
        // a refused controller extension leaves neither behind. Restorable
        // seams are undone by this guard if the boot below fails.
        let mut storage_info = super::StorageInfo::describe(
            self.seams.storage.as_ref(),
            config.as_ref().unwrap_or(&Config::default()),
        );
        let mut host_seams = std::mem::take(&mut self.seams);
        if host_seams.storage.is_none() {
            if let Some(config) = &config {
                if let Some(url) = openhuman_core::storage::explicit_url(config) {
                    host_seams.storage = Some(super::StorageSource::Url(url));
                }
            }
        }
        // A stateless workspace keeps nothing durable on disk: with no host
        // backend, the documents the core keeps beside the session store (run
        // status, cron, flows) go to memory instead of the workspace-default
        // SQLite under `<workspace>/storage`. Installed process-wide like any
        // storage seam, and removed with the runtime.
        if host_seams.storage.is_none() && matches!(self.workspace, Workspace::Stateless) {
            host_seams.storage = Some(super::StorageSource::Url("memory".into()));
        }
        validate_storage_feature(host_seams.storage.as_ref())?;
        host_seams
            .open_storage()
            .await
            .map_err(RuntimeError::Invalid)?;
        let storage_backend = match &host_seams.storage {
            Some(super::StorageSource::Backend(backend)) => Some(backend.clone()),
            _ => None,
        };
        if self.session_store.is_none() {
            if let Some(backend) = &storage_backend {
                let stores = openhuman_core::agent::session_store::TenantSessionStores::new(
                    backend.clone(),
                    !openhuman_core::storage::driver_is_shared(backend.driver()),
                )
                .map_err(|e| RuntimeError::Build(e.into()))?;
                self.session_store = Some(Arc::new(stores));
            }
        }
        let mut seams = host_seams.install().map_err(RuntimeError::Invalid)?;

        // Before `CoreBuilder::build()`: the scheduler gate reads the credential
        // store exactly once, at boot, to decide whether it is signed in.
        let has_api_key = match (self.api_key.as_ref(), config.as_ref()) {
            (Some(key), Some(config)) => {
                store_api_key(config, key)?;
                true
            }
            _ => false,
        };

        // An endpoint without a model is deliberately ignored by the route
        // applicator, so host policy follows the effective behaviour: only a
        // *usable* runtime-default route exempts an inherited install from
        // its session gate. An API key is a credential in its own right.
        let routed_provider_effective = routed_provider_effective(&self.provider, config.as_ref());
        let host_kind = effective_host_kind(
            self.host_kind,
            inherit,
            routed_provider_effective || has_api_key,
        );

        let domains = self.domains.unwrap_or_else(default_domains);
        let tool_groups = self.tool_groups.clone().unwrap_or_default();
        let services = self.services.unwrap_or_else(default_services);

        if let Some(engine) = self.memory_engine.clone() {
            openhuman_core::memory::engine::install_host_engine(engine);
        }
        if let Some(delivery) = self.cron_delivery.clone() {
            log::debug!("[embed][runtime] host cron delivery installed");
            openhuman_core::cron::host_delivery::install(delivery);
        }

        // Install only after all fallible workspace/config resolution has
        // completed; core boot performs recovery against this provider.
        let installed_session_store = self.session_store.clone();
        let previous_session_store = installed_session_store
            .clone()
            .and_then(openhuman_core::agent::session_store::install);
        let mut session_store_cleanup = SessionStoreCleanup {
            installed: installed_session_store,
            previous: previous_session_store,
        };

        log::debug!(
            "[embed][runtime] building host_kind={host_kind:?} config_source={:?} \
             inherit_workspace={inherit} routed_provider={} api_key={has_api_key} \
             domains={domains:?} services={services:?} tool_groups={tool_groups:?} \
             listen_host={:?} listen_port={:?}",
            self.config_source,
            self.provider.is_routed(),
            self.listen_host,
            self.listen_port,
        );

        let mut builder = CoreBuilder::new(host_kind)
            .domains(domains)
            .tool_groups(tool_groups.clone())
            .services(services)
            .token(std::mem::replace(
                &mut self.token,
                openhuman_core::core::runtime::TokenSource::EnvOrFile,
            ));
        if let Some(config) = config.clone() {
            builder = builder.config(config);
        }
        if let Some(host) = self.listen_host.take() {
            builder = builder.host(host);
        }
        if let Some(port) = self.listen_port {
            builder = builder.port(port);
        }
        if let Some(transport) = self.backend_transport.take() {
            builder = builder.backend_transport(transport);
        }
        let runtime = builder.build().await.map_err(|error| {
            log::warn!("[embed][runtime] core build failed: {error:#}");
            RuntimeError::Build(error)
        })?;
        let core = Core::from_runtime(Arc::new(runtime));
        let storage_backend = storage_backend.or_else(openhuman_core::storage::installed);
        if !storage_info.configured {
            if let Some(backend) = &storage_backend {
                storage_info = super::StorageInfo {
                    source: "backend".into(),
                    driver: Some(backend.driver().into()),
                    configured: true,
                };
            }
        }

        // Discovered: agents start from the config the core just loaded.
        let (mut base_config, config_unavailable) = match config.take() {
            Some(config) => (config, None),
            None => discovered_base_config().await,
        };
        if discovered {
            // Agent defaults only — the core's own config is not edited.
            self.config_knobs.apply(&mut base_config);
            if let Some(value) = self.agent_defaults.model.temperature {
                base_config.default_temperature = value;
            }
            if let Some(value) = self.agent_defaults.model.max_iterations {
                base_config.agent.max_tool_iterations_override = Some(value);
            }
            self.access.apply(&mut base_config);
            apply_provider(&mut base_config, &self.provider);
        }

        if seams.has_pending_live_policy() {
            if config_unavailable.is_some() {
                // The base config is a placeholder: its directories are the
                // default root, not the operator's install, so a policy
                // scoped to them would guard the wrong tree. Keep the policy
                // the core's bootstrap installed.
                log::warn!(
                    "[embed][runtime] live policy not installed: discovered config unavailable"
                );
            } else {
                seams.install_live_policy(&base_config.workspace_dir, &base_config.action_dir);
            }
        }

        if let Some(session) = self.session.take() {
            core.auth().store(session).await?;
        }

        let installed_session_store = session_store_cleanup.installed.take();
        let previous_session_store = session_store_cleanup.previous.take();
        log::debug!("[embed][runtime] built host_kind={host_kind:?}");

        let defaults = self.resolved_defaults(&base_config, domains, tool_groups.clone());
        let runtime = Runtime::new(
            core,
            resolved,
            installed_session_store,
            previous_session_store,
            Some(seams),
            base_config,
            config_unavailable,
            inherit,
            domains,
            tool_groups,
            self.max_agents,
            defaults,
            host_kind,
            self.selection,
            storage_info,
            storage_backend,
        );
        for (kind, handler) in self.cron_job_handlers {
            runtime.register_job_handler(kind, handler);
        }
        if starts_services_on_build(services, self.defer_services) {
            log::debug!("[embed][runtime] starting background services {services:?}");
            runtime.start_services().await;
        }
        Ok(runtime)
    }

    pub(super) fn resolved_defaults(
        &self,
        config: &Config,
        domains: crate::DomainSet,
        tool_groups: crate::ToolGroups,
    ) -> super::AgentDefaults {
        let mut defaults = self.agent_defaults.clone();
        defaults.provider = self.provider.clone();
        if defaults.provider.model_id().is_none() {
            if let Some(model) = &config.default_model {
                defaults.provider = defaults.provider.model(model);
            }
        }

        defaults.access = self.access.clone();
        defaults.domains = domains;
        defaults.tool_groups = tool_groups;
        let definition_model = defaults.definition.explicit_model_defaults();
        defaults.model.temperature = defaults
            .model
            .temperature
            .or(definition_model.temperature)
            .or(Some(config.default_temperature));
        defaults.model.max_iterations = defaults
            .model
            .max_iterations
            .or(definition_model.max_iterations)
            .or(config.agent.max_tool_iterations_override);
        if let Ok(definition) = defaults.definition.clone().into_core("runtime-defaults") {
            defaults.model.max_iterations = defaults
                .model
                .max_iterations
                .or(Some(definition.max_iterations));
            if defaults.sandbox == crate::SandboxModeSpec::None {
                defaults.sandbox = match definition.sandbox_mode {
                    tinyagents_definition::SandboxMode::None => crate::SandboxModeSpec::None,
                    tinyagents_definition::SandboxMode::ReadOnly => {
                        crate::SandboxModeSpec::ReadOnly
                    }
                    tinyagents_definition::SandboxMode::Sandboxed => {
                        crate::SandboxModeSpec::Sandboxed
                    }
                };
            }
        }
        defaults
    }

    /// The config a [`ConfigSource::Resolved`] runtime hands the core.
    async fn resolve_config(
        &mut self,
        resolved: &ResolvedWorkspace,
    ) -> Result<Config, RuntimeError> {
        let mut config = match (&self.workspace, self.config.take()) {
            (Workspace::Inherit, Some(config)) => config,
            (Workspace::Inherit, None) => {
                Config::load_or_init().await.map_err(RuntimeError::Build)?
            }
            (_, supplied) => {
                let mut config = supplied.unwrap_or_default();
                config.workspace_dir = resolved.workspace_dir.clone();
                config.action_dir = resolved.action_dir.clone();
                // Credential state, auth profiles and the keyring file backend
                // all resolve against this path's parent, not `workspace_dir`.
                config.config_path = resolved.config_path.clone();
                config
            }
        };
        // Raw paths refine the resolved workspace field by field; the
        // credential root (`config_path`) is left to the `Workspace`.
        if let Some(dir) = self.workspace_dir.clone() {
            create_dir(&dir, "create the workspace_dir")?;
            log::debug!("[embed][runtime] workspace_dir override applied");
            config.workspace_dir = dir;
        }
        if let Some(dir) = self.action_dir.clone() {
            create_dir(&dir, "create the action_dir")?;
            log::debug!("[embed][runtime] action_dir override applied");
            config.action_dir = dir;
        }
        if let Some(url) = self.backend_url.clone() {
            config.api_url = Some(url);
        }
        self.config_knobs.apply(&mut config);
        if let Some(temperature) = self.agent_defaults.model.temperature {
            config.default_temperature = temperature;
        }
        if let Some(iterations) = self.agent_defaults.model.max_iterations {
            config.agent.max_tool_iterations_override = Some(iterations);
        }
        self.access.apply(&mut config);
        apply_provider(&mut config, &self.provider);
        Ok(config)
    }
}

/// The base config for agents of a runtime whose core discovered its own:
/// the same `load_or_init` the core just ran, so the two agree.
///
/// When it fails the core has already booted without its workspace-bound
/// stores. The runtime still comes up (a host that never creates an agent,
/// like the desktop shell, keeps working), but the failure is returned with
/// the placeholder config so [`Runtime::agent`] refuses instead of laying an
/// agent out under the default root, a different workspace and credential
/// store from the one the operator selected.
async fn discovered_base_config() -> (Config, Option<String>) {
    match Config::load_or_init().await {
        Ok(config) => (config, None),
        Err(error) => {
            log::warn!(
                "[embed][runtime] discovered config failed to load; agents will be refused: {error:#}"
            );
            (Config::default(), Some(format!("{error:#}")))
        }
    }
}

/// Whether the runtime-default provider route would actually take effect. An
/// endpoint without a model is deliberately ignored by the route applicator,
/// so host policy follows the effective behaviour: only a usable route with a
/// model exempts an inherited install from its session gate. A
/// [`ConfigSource::Discovered`] build has no config yet (`None`), but the
/// provider is applied to the agents' base config after boot, so it is judged
/// by the model it carries itself.
pub(crate) fn routed_provider_effective(provider: &Provider, config: Option<&Config>) -> bool {
    provider.custom_model().is_some()
        || (provider.has_usable_route()
            && match config {
                Some(config) => config.default_model.as_deref(),
                None => provider.model_id(),
            }
            .is_some_and(|model| !model.trim().is_empty()))
}

fn store_api_key(config: &Config, key: &ApiKey) -> Result<(), RuntimeError> {
    let state_dir = config
        .config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| config.config_path.clone());
    openhuman_core::security::credentials::api_key::store_api_key_in(
        &state_dir,
        config.secrets.encrypt,
        key.expose(),
    )
    .map(|_| ())
    .map_err(RuntimeError::Build)
}

fn create_dir(dir: &Path, what: &'static str) -> Result<(), RuntimeError> {
    std::fs::create_dir_all(dir).map_err(|source| RuntimeError::Workspace { what, source })
}

fn map_ws(err: crate::HarnessError) -> RuntimeError {
    match err {
        crate::HarnessError::Workspace { what, source } => RuntimeError::Workspace { what, source },
        crate::HarnessError::Invalid(msg) => RuntimeError::Invalid(msg),
        crate::HarnessError::Build(e) => RuntimeError::Build(e),
        crate::HarnessError::Call(e) => RuntimeError::Call(e),
        crate::HarnessError::AlreadyRunning => RuntimeError::AlreadyRunning,
    }
}

/// Preserve the installed application's authentication policy when the
/// runtime borrows both its workspace and provider. `Library` means the host
/// supplied inference (a route or an API key); it must not become a blanket
/// way to bypass the session gate around an operator-installed provider.
pub(crate) fn effective_host_kind(
    requested: HostKind,
    inherit_workspace: bool,
    host_supplied_credential: bool,
) -> HostKind {
    if requested == HostKind::Library && inherit_workspace && !host_supplied_credential {
        HostKind::Cli
    } else {
        requested
    }
}

/// Apply a [`Provider`]'s model to the config.
///
/// Only the model: the *route* is a per-turn parameter, never a config write,
/// because config routes persist. See the [`provider`](crate::harness::provider)
/// module docs.
pub(crate) fn apply_provider(config: &mut Config, provider: &Provider) {
    if let Some(model) = provider.model_id() {
        config.default_model = Some(model.to_string());
    }
}

/// Fail before opening a URL whose driver is absent; credential text never enters the error.
fn validate_storage_feature(source: Option<&super::StorageSource>) -> Result<(), RuntimeError> {
    let Some(super::StorageSource::Url(url)) = source else {
        return Ok(());
    };
    let driver = openhuman_core::storage::StorageUrl::parse(url)
        .map_err(|_| RuntimeError::Invalid("invalid storage URL".into()))?
        .driver()
        .to_string();
    let feature = match driver.as_str() {
        "sqlite" => Some("storage-sqlite"),
        "mongodb" => Some("storage-mongodb"),
        "file" => Some("storage-file"),
        _ => None,
    };
    if let Some(feature) = feature {
        if !openhuman_core::core::runtime::compiled_features()[feature] {
            return Err(RuntimeError::MissingStorageFeature { driver, feature });
        }
    }
    Ok(())
}
