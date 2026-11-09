//! Versioned, credential-safe runtime capability metadata.
use super::{LearningSettings, ModelDefaults, Runtime, RuntimeBuilder};
use openhuman_core::config::{
    schema::{
        AutonomyConfig, CronConfig, PrivacyConfig, PromptScreenConfig, SecretsConfig,
        SecurityJudgeRoute, SecurityModeConfig,
    },
    Config,
};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct ConfigKnobs {
    pub autonomy: Option<AutonomyConfig>,
    /// Data egress policy name.
    pub privacy: Option<PrivacyConfig>,
    /// Whether untrusted input is screened for prompt injection.
    pub prompt_screen: Option<PromptScreenConfig>,
    /// Whether TinySecurity-delegated checks run.
    pub security: Option<SecurityModeConfig>,
    /// The host's own route to the Jev judge.
    pub security_judge: Option<SecurityJudgeRoute>,
    pub secrets: Option<SecretsConfig>,
    pub cron: Option<CronConfig>,
    /// Background memory learning policy.
    pub learning: Option<LearningSettings>,
    pub tool_rules: Option<tinytools::ToolRules>,
}
impl ConfigKnobs {
    pub fn is_set(&self) -> bool {
        self.autonomy.is_some()
            || self.privacy.is_some()
            || self.prompt_screen.is_some()
            || self.security.is_some()
            || self.security_judge.is_some()
            || self.secrets.is_some()
            || self.cron.is_some()
            || self.learning.is_some()
            || self.tool_rules.is_some()
    }
    pub fn apply(&self, config: &mut Config) {
        if let Some(v) = &self.autonomy {
            config.autonomy = v.clone();
        }
        if let Some(v) = &self.privacy {
            config.privacy = *v;
        }
        if let Some(v) = &self.prompt_screen {
            config.prompt_screen = *v;
        }
        if let Some(v) = &self.security {
            config.security = *v;
        }
        if let Some(v) = &self.security_judge {
            config.security_judge = Some(v.clone());
        }
        if let Some(v) = &self.secrets {
            config.secrets = v.clone();
        }
        if let Some(v) = &self.cron {
            config.cron = v.clone();
        }
        if let Some(v) = &self.tool_rules {
            config.tool_rules = v.clone();
        }
        if let Some(v) = &self.learning {
            config.memory.recall.build_beliefs_every = v.build_beliefs_every;
            config.memory.recall.learnings_limit = v.learnings_limit;
        }
    }
}
/// Stable JSON shape. Secrets, endpoint strings, custom prompts and environment values are excluded.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuntimeInfo {
    /// Capability JSON schema version, currently one.
    pub schema_version: u32,
    /// Whether the metadata uses the loaded configuration rather than a pre-build placeholder.
    pub configuration_resolved: bool,
    /// Actual Cargo gates compiled into the core artifact.
    pub compiled_features: BTreeMap<String, bool>,
    /// Domain families available to new agents.
    pub domains: Vec<String>,
    /// Selected background and bootstrap service names.
    pub services: Vec<String>,
    /// Default tool group disclosure ceiling.
    pub tool_groups: BTreeMap<String, String>,
    /// Effective host identity used by core authentication policy.
    pub host_kind: String,
    /// Maximum simultaneously registered agents.
    pub max_agents: usize,
    /// Credential-safe summary of agent defaults.
    pub defaults: DefaultsInfo,
    /// Selected runtime configuration policies.
    pub configuration: ConfigurationInfo,
    /// Credential-safe storage selection.
    pub storage: StorageInfo,
    /// Explicitly selected module families.
    pub modules: Vec<super::RuntimeModule>,
    /// Approximate runtime activity class.
    pub weight: super::WeightClass,
}
/// Storage source and driver only; the URL and its credentials are never exposed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StorageInfo {
    /// Selection source: workspace, configuration, explicit URL or backend.
    pub source: String,
    /// Storage driver identifier, without endpoint or credentials.
    pub driver: Option<String>,
    /// Whether a non-classic storage source was selected.
    pub configured: bool,
}
impl StorageInfo {
    pub(super) fn describe(
        source: Option<&super::StorageSource>,
        config: &Config,
        retained_driver: Option<&str>,
    ) -> Self {
        match source {
            Some(super::StorageSource::Backend(_)) => Self::backend_driver(retained_driver),
            Some(super::StorageSource::Url(url)) => Self::url("explicit", url),
            None => match openhuman_core::storage::explicit_url(config) {
                Some(url) => Self::url("configuration", &url),
                None => Self {
                    source: "workspace".into(),
                    driver: None,
                    configured: false,
                },
            },
        }
    }
    pub(super) fn url(source: &str, url: &str) -> Self {
        let driver = openhuman_core::storage::StorageUrl::parse(url)
            .ok()
            .map(|u| u.driver().to_string());
        Self {
            source: source.into(),
            driver,
            configured: true,
        }
    }

    pub(super) fn backend(installed_url: Option<&str>) -> Self {
        match installed_url {
            Some(url) => Self::url("backend", url),
            None => Self::backend_driver(None),
        }
    }

    fn backend_driver(driver: Option<&str>) -> Self {
        Self {
            source: "backend".into(),
            driver: driver.map(str::to_owned),
            configured: true,
        }
    }

    pub(super) fn recover_sessions_on_open(&self) -> bool {
        self.driver
            .as_deref()
            .is_some_and(|driver| !openhuman_core::storage::driver_is_shared(driver))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
/// Public agent defaults with credentials, prompt text and filesystem paths omitted.
pub struct DefaultsInfo {
    /// Resolved sampling and iteration defaults.
    pub model: ModelDefaults,
    /// Configured default model identifier.
    pub provider_model: Option<String>,
    /// Whether the host supplied a provider route.
    pub routed_provider: bool,
    /// Default shell and file confinement.
    pub sandbox: String,
    /// Whether interactive approval is enabled by default.
    pub approval_gate: bool,
    /// Whether operator skill roots are discovered.
    pub include_user_skills: bool,
    /// Whether a runtime skill bundle directory was supplied.
    pub has_skills_root: bool,
    /// Number of inherited MCP servers.
    pub mcp_servers: usize,
    /// Reusable named definition templates.
    pub templates: Vec<String>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
/// Policy and learning settings safe to display in capability JSON.
pub struct ConfigurationInfo {
    /// Whether the in-process autonomy policy is enabled.
    pub autonomy_enabled: bool,
    /// Data egress policy name.
    pub privacy: String,
    /// Whether untrusted input is screened for prompt injection; false when
    /// the host turned the screen off, or security is off (see
    /// `security_enforced`). Absent in older metadata → true.
    #[serde(default = "prompt_screen_enforced_by_default")]
    pub prompt_screen_enforced: bool,
    /// Whether the checks OpenHuman delegates to TinySecurity run and fail
    /// closed; false when the host turned them off (`[security] mode = "off"`)
    /// or the build has no `security-module` feature, and they then allow.
    /// Absent in older metadata → true.
    #[serde(default = "security_enforced_by_default")]
    pub security_enforced: bool,
    /// Whether stored credentials are encrypted.
    pub secrets_encrypted: bool,
    /// Whether configured cron scheduling is enabled.
    pub cron_enabled: bool,
    /// Background memory learning policy.
    pub learning: LearningSettings,
    /// Whether global tool rules constrain tools through rules or a deny default.
    pub has_tool_rules: bool,
}
fn prompt_screen_enforced_by_default() -> bool {
    true
}
fn security_enforced_by_default() -> bool {
    true
}
impl RuntimeBuilder {
    /// Describe this builder without booting a core or discovering operator files.
    /// Discovered config values are unknown until `Runtime::capabilities()`.
    pub fn describe(&self) -> RuntimeInfo {
        let mut config = self.config.clone().unwrap_or_default();
        self.config_knobs.apply(&mut config);
        self.access.apply(&mut config);
        super::build::apply_provider(&mut config, &self.provider);
        let defaults = self.resolved_defaults(
            &config,
            self.domains.unwrap_or_else(super::presets::default_domains),
            self.tool_groups.clone().unwrap_or_default(),
        );
        let host = super::build::effective_host_kind(
            self.host_kind,
            self.workspace.is_operator_owned(),
            super::build::routed_provider_effective(&self.provider, Some(&config))
                || self.api_key.is_some(),
        );
        let mut info = make_info(
            &defaults,
            &config,
            self.services
                .unwrap_or_else(super::presets::default_services),
            host,
            self.max_agents,
            self.selection.as_ref(),
        );
        info.configuration_resolved = self.config_source == super::ConfigSource::Resolved;
        // This builder's own mode, not the mode a runtime already running in
        // this process booted with.
        let security_enforced =
            openhuman_core::security::switch::disabled_by_config(&config).is_none();
        info.configuration.security_enforced = security_enforced;
        info.configuration.prompt_screen_enforced =
            config.prompt_screen.enforces() && security_enforced;
        info.storage = StorageInfo::describe(
            self.seams.storage.as_ref(),
            &config,
            self.seams.storage_driver.as_deref(),
        );
        // `build` gives a stateless workspace with no storage source an
        // in-memory backend; report what it will install.
        if !info.storage.configured && matches!(self.workspace, crate::Workspace::Stateless) {
            info.storage = StorageInfo {
                source: "backend".into(),
                driver: Some("memory".into()),
                configured: true,
            };
        }
        info
    }
}
impl Runtime {
    /// Snapshot of the effective runtime, including the host kind chosen during boot.
    pub fn capabilities(&self) -> RuntimeInfo {
        let mut info = make_info(
            &self.defaults(),
            &self.base_config,
            self.services(),
            self.effective_host_kind,
            self.max_agents,
            self.selection.as_ref(),
        );
        info.configuration_resolved = self.config_unavailable.is_none();
        info.storage = self.storage_info.clone();
        info
    }
}
fn make_info(
    defaults: &super::AgentDefaults,
    config: &Config,
    services: crate::ServiceSet,
    host: crate::HostKind,
    max_agents: usize,
    selection: Option<&super::ModuleSelection>,
) -> RuntimeInfo {
    use openhuman_core::core::all::DomainGroup;
    let domains = DomainGroup::ALL
        .iter()
        .filter(|g| defaults.domains.allows(**g))
        .map(|g| format!("{g:?}").to_lowercase())
        .collect();
    let service_flags = [
        ("rpc_http", services.rpc_http),
        ("socketio", services.socketio),
        ("cron", services.cron),
        ("channels", services.channels),
        ("login_gated", services.login_gated),
        ("update_scheduler", services.update_scheduler),
        ("memory_queue", services.memory_queue),
        ("skill_catalog_refresh", services.skill_catalog_refresh),
        ("mcp_boot", services.mcp_boot),
        ("integrations", services.integrations),
        ("memory_sync", services.memory_sync),
    ];
    let security_enforced = openhuman_core::security::switch::disabled_for(Some(config)).is_none();
    RuntimeInfo {
        schema_version: 1,
        configuration_resolved: true,
        compiled_features: openhuman_core::core::runtime::compiled_features(),
        domains,
        services: service_flags
            .into_iter()
            .filter(|(_, v)| *v)
            .map(|(k, _)| k.into())
            .collect(),
        tool_groups: crate::ToolGroups::ids()
            .map(|id| {
                (
                    id.into(),
                    format!("{:?}", defaults.tool_groups.mode(id)).to_lowercase(),
                )
            })
            .collect(),
        host_kind: format!("{host:?}"),
        max_agents,
        defaults: DefaultsInfo {
            model: defaults.model.clone(),
            provider_model: config.default_model.clone(),
            routed_provider: defaults.provider.is_routed(),
            sandbox: format!("{:?}", defaults.sandbox),
            approval_gate: defaults.access.approval_gate_enabled(),
            include_user_skills: defaults.skills.include_user_skills,
            has_skills_root: defaults.skills.root.is_some(),
            mcp_servers: {
                #[cfg(feature = "mcp")]
                {
                    config.mcp_client.servers.len() + defaults.mcp_baseline.len()
                }
                #[cfg(not(feature = "mcp"))]
                {
                    config.mcp_client.servers.len()
                }
            },
            templates: defaults.templates.keys().cloned().collect(),
        },
        configuration: ConfigurationInfo {
            autonomy_enabled: config.autonomy.enabled,
            privacy: format!("{:?}", config.privacy.mode),
            prompt_screen_enforced: config.prompt_screen.enforces() && security_enforced,
            security_enforced,
            secrets_encrypted: config.secrets.encrypt,
            cron_enabled: config.cron.enabled,
            learning: LearningSettings {
                build_beliefs_every: config.memory.recall.build_beliefs_every,
                learnings_limit: config.memory.recall.learnings_limit,
            },
            has_tool_rules: config.tool_rules.default != tinytools::DefaultEffect::Allow
                || !config.tool_rules.rules.is_empty(),
        },
        storage: StorageInfo::describe(None, config, None),
        modules: selection.map(|s| s.modules.clone()).unwrap_or_default(),
        weight: selection
            .map(|s| s.weight)
            .unwrap_or(super::WeightClass::Custom),
    }
}

#[cfg(test)]
#[path = "info_tests.rs"]
mod tests;
