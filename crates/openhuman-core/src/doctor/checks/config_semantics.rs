//! Static checks for configuration values that disagree with one another.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::doctor::contract::{DoctorCheck, DoctorCtx, DoctorTier, Finding, FindingSeverity};
use crate::storage::{StorageUrl, STORAGE_URL_VAR};
use tinystoragedrivers::StorageConfig;

const COMPONENTS_ID: &str = "components/referenced-but-disabled";
const MODULE_FALLBACK_ID: &str = "modules/source-fallback";
const MODULE_OVERRIDE_ID: &str = "modules/override-invalid";
const STORAGE_URL_ID: &str = "storage/url";
const ACTION_DIR_ID: &str = "workspace/action-dir";
const MASTER_KEY_ENV_ID: &str = "secrets/master-key-env";
const PROXY_ID: &str = "network/proxy";
const MEMORY_ROOT_ID: &str = "memory/root";

#[derive(Clone, Copy)]
enum CheckKind {
    Components,
    ModuleFallback,
    InvalidModuleOverride,
    StorageUrl,
    ActionDir,
    MasterKeyEnv,
    Proxy,
    MemoryRoot,
}

impl CheckKind {
    const fn id(self) -> &'static str {
        match self {
            Self::Components => COMPONENTS_ID,
            Self::ModuleFallback => MODULE_FALLBACK_ID,
            Self::InvalidModuleOverride => MODULE_OVERRIDE_ID,
            Self::StorageUrl => STORAGE_URL_ID,
            Self::ActionDir => ACTION_DIR_ID,
            Self::MasterKeyEnv => MASTER_KEY_ENV_ID,
            Self::Proxy => PROXY_ID,
            Self::MemoryRoot => MEMORY_ROOT_ID,
        }
    }

    const fn category(self) -> &'static str {
        match self {
            Self::Components => "components",
            Self::ModuleFallback | Self::InvalidModuleOverride => "modules",
            Self::StorageUrl => "storage",
            Self::ActionDir => "workspace",
            Self::MasterKeyEnv => "secrets",
            Self::Proxy => "network",
            Self::MemoryRoot => "memory",
        }
    }
}

struct ConfigSemanticsCheck(CheckKind);

pub(super) fn checks() -> Vec<Arc<dyn DoctorCheck>> {
    [
        CheckKind::Components,
        CheckKind::ModuleFallback,
        CheckKind::InvalidModuleOverride,
        CheckKind::StorageUrl,
        CheckKind::ActionDir,
        CheckKind::MasterKeyEnv,
        CheckKind::Proxy,
        CheckKind::MemoryRoot,
    ]
    .into_iter()
    .map(|kind| Arc::new(ConfigSemanticsCheck(kind)) as Arc<dyn DoctorCheck>)
    .collect()
}

#[async_trait]
impl DoctorCheck for ConfigSemanticsCheck {
    fn id(&self) -> &str {
        self.0.id()
    }

    fn category(&self) -> &str {
        self.0.category()
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    fn is_blocking(&self) -> bool {
        true
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        self.detect_blocking(ctx).unwrap_or_default()
    }

    fn detect_blocking(&self, ctx: &DoctorCtx) -> Option<Vec<Finding>> {
        let findings = match self.0 {
            CheckKind::Components => referenced_but_disabled(ctx.config()),
            CheckKind::ModuleFallback => module_source_fallback(ctx.config()),
            CheckKind::InvalidModuleOverride => invalid_module_overrides(ctx.config()),
            CheckKind::StorageUrl => storage_url(ctx.config()),
            CheckKind::ActionDir => action_dir(ctx.config()),
            CheckKind::MasterKeyEnv => master_key_env(),
            CheckKind::Proxy => proxy(ctx.config()),
            CheckKind::MemoryRoot => memory_root(ctx.config()),
        };
        Some(findings)
    }
}

fn finding(
    check_id: &str,
    severity: FindingSeverity,
    message: impl Into<String>,
    fix_hint: impl Into<String>,
    config_path: Option<String>,
) -> Finding {
    let mut finding = Finding::new(check_id, severity, message, fix_hint);
    finding.config_path = config_path;
    finding
}

fn explicitly_configured(config: &Config, path: &str) -> bool {
    config.load_provenance().has_explicit_under(path)
}

fn referenced_but_disabled(config: &Config) -> Vec<Finding> {
    let mut references = Vec::new();
    let memory = &config.memory;
    let memory_engine = memory.engine.trim();
    let default_memory_engine = crate::config::schema::memory::DEFAULT_MEMORY_ENGINE;
    if !memory_engine.is_empty()
        && !memory_engine.eq_ignore_ascii_case("none")
        && (!memory_engine.eq_ignore_ascii_case(default_memory_engine)
            || explicitly_configured(config, "memory.engine"))
    {
        references.push(("memory", "memory.engine"));
    }
    if !memory.engines.is_empty() {
        references.push(("memory", "memory.engines"));
    }
    if memory.agent_id.as_deref().is_some_and(non_empty) {
        references.push(("memory", "memory.agent_id"));
    }
    if memory.root.as_deref().is_some_and(non_empty) {
        references.push(("memory", "memory.root"));
    }
    if !memory.sources.is_empty() {
        references.push(("memory", "memory.sources"));
    }
    if !memory.agents.is_empty() {
        references.push(("memory", "memory.agents"));
    }
    if memory.embedding_provider != crate::config::schema::memory::DEFAULT_EMBEDDING_PROVIDER
        || explicitly_configured(config, "memory.embedding_provider")
    {
        references.push(("memory", "memory.embedding_provider"));
    }
    if memory.embedding_model != crate::config::schema::memory::DEFAULT_EMBEDDING_MODEL
        || explicitly_configured(config, "memory.embedding_model")
    {
        references.push(("memory", "memory.embedding_model"));
    }
    if memory.embedding_dimensions != crate::config::schema::memory::DEFAULT_EMBEDDING_DIMENSIONS
        || explicitly_configured(config, "memory.embedding_dimensions")
    {
        references.push(("memory", "memory.embedding_dimensions"));
    }

    if !config.mcp_client.servers.is_empty() {
        references.push(("mcp", "mcp_client.servers"));
    }

    let channels = &config.channels_config;
    if channels.telegram.is_some()
        || channels.discord.is_some()
        || channels.slack.is_some()
        || channels.mattermost.is_some()
        || channels.imessage.is_some()
        || channels.signal.is_some()
        || channels.whatsapp.is_some()
        || channels.linq.is_some()
        || channels.email.is_some()
        || channels.irc.is_some()
        || channels.lark.is_some()
        || channels.dingtalk.is_some()
        || channels.qq.is_some()
        || channels.yuanbao.is_some()
        || channels.relay.is_some()
    {
        references.push(("channels", "channels_config"));
    }

    if !config.voice_providers.is_empty() {
        references.push(("voice", "voice_providers"));
    }
    if config.stt_provider.as_deref().is_some_and(non_empty) {
        references.push(("voice", "stt_provider"));
    }
    if config.tts_provider.as_deref().is_some_and(non_empty) {
        references.push(("voice", "tts_provider"));
    }
    if config.voice_server.stt_engine != crate::config::schema::SttEngine::default()
        || explicitly_configured(config, "voice_server.stt_engine")
    {
        references.push(("voice", "voice_server.stt_engine"));
    }

    if memory.sources.iter().any(|source| {
        matches!(
            source.kind,
            crate::config::schema::MemorySourceKind::File
                | crate::config::schema::MemorySourceKind::Folder
        )
    }) {
        references.push(("documents", "memory.sources"));
    }

    if config.search != crate::config::schema::SearchConfig::default()
        || explicitly_configured(config, "search")
        || config.seltz.enabled
        || config.searxng.enabled
    {
        references.push(("search", "search"));
    }

    if config.computer != crate::config::ComputerConfig::default()
        || explicitly_configured(config, "computer")
        || config.browser.enabled
        || !config.browser.allowed_domains.is_empty()
        || config
            .browser
            .session_name
            .as_deref()
            .is_some_and(non_empty)
        || config.browser.chrome_path.as_deref().is_some_and(non_empty)
        || config
            .browser
            .profile_path
            .as_deref()
            .is_some_and(non_empty)
    {
        references.push(("computer", "computer"));
    }

    if config.sandbox.enabled == Some(true)
        || !matches!(&config.sandbox.backend, crate::config::SandboxBackend::Auto)
        || explicitly_configured(config, "sandbox.backend")
        || !config.sandbox.firejail_args.is_empty()
    {
        references.push(("sandbox", "sandbox"));
    }

    if config.hosting.has_api_key()
        || config.hosting.team().is_some()
        || config.hosting.provider != crate::config::schema::HostingConfig::default().provider
        || explicitly_configured(config, "hosting.provider")
    {
        references.push(("hosting", "hosting"));
    }

    if tokenjuice_is_custom(config) {
        references.push(("juice", "tokenjuice"));
    }

    if !config.capability_providers.is_empty() {
        references.push(("connectors", "capability_providers"));
    }
    if config.composio.mode != crate::config::schema::COMPOSIO_MODE_BACKEND
        || config.composio.api_key.is_some()
        || !config.composio.offered_toolkits_allow.is_empty()
        || !config.composio.offered_toolkits_deny.is_empty()
        || explicitly_configured(config, "composio")
    {
        references.push(("connectors", "composio"));
    }

    if config.inbound.queues_path.is_some()
        || config.inbound.pipeline_path.is_some()
        || config.inbound.queue_url.as_deref().is_some_and(non_empty)
        || config.inbound.triage_timeout_ms
            != crate::config::schema::InboundConfig::default().triage_timeout_ms
        || explicitly_configured(config, "inbound.triage_timeout_ms")
    {
        references.push(("eventqueues", "inbound"));
    }

    references
        .into_iter()
        .filter(|(component, _)| config.components.get(component) == Some(false))
        .map(|(component, path)| {
            finding(
                COMPONENTS_ID,
                FindingSeverity::Warn,
                format!("Configuration references the disabled {component} component."),
                format!("Enable the {component} component or remove the dependent configuration."),
                Some(path.to_owned()),
            )
        })
        .collect()
}

fn tokenjuice_is_custom(config: &Config) -> bool {
    let current = &config.tokenjuice;
    let default = crate::config::schema::TokenjuiceConfig::default();
    explicitly_configured(config, "tokenjuice")
        || current.router_enabled != default.router_enabled
        || current.ccr_enabled != default.ccr_enabled
        || current.repl_handle_enabled != default.repl_handle_enabled
        || current.ccr_disk_enabled != default.ccr_disk_enabled
        || current.max_cache_entries != default.max_cache_entries
        || current.max_cache_bytes != default.max_cache_bytes
        || current.ccr_ttl_secs != default.ccr_ttl_secs
        || current.min_bytes_to_compress != default.min_bytes_to_compress
        || current.ccr_min_tokens != default.ccr_min_tokens
        || current.search_enabled != default.search_enabled
        || current.code_enabled != default.code_enabled
        || current.html_enabled != default.html_enabled
}

fn non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn module_source_fallback(config: &Config) -> Vec<Finding> {
    module_statuses(config)
        .into_iter()
        .filter_map(|status| {
            if status.source != Some(crate::modules::ModuleSource::Release) {
                return None;
            }
            let requested = requested_module_source(config, &status.id)?;
            let detail = match requested {
                crate::config::schema::ModuleSourceMode::Source => {
                    "no source-built artifact is available; the release will be served"
                }
                crate::config::schema::ModuleSourceMode::Linked => {
                    "no linked factory is registered in this binary; the release will be served"
                }
                crate::config::schema::ModuleSourceMode::Release => return None,
            };
            Some(finding(
                MODULE_FALLBACK_ID,
                FindingSeverity::Warn,
                format!(
                    "Module `{}` requested {} mode, but {detail}.",
                    status.id,
                    requested.as_str()
                ),
                "Build or install the requested module source, or set modules.source to release.",
                Some("modules.source".into()),
            ))
        })
        .collect()
}

fn module_statuses(config: &Config) -> Vec<crate::modules::ModuleStatus> {
    #[cfg(feature = "modules")]
    {
        crate::modules::ops::list(config)
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = config;
        Vec::new()
    }
}

fn requested_module_source(
    config: &Config,
    id: &str,
) -> Option<crate::config::schema::ModuleSourceMode> {
    if let Some(override_) = config
        .modules
        .overrides
        .iter()
        .find(|override_| override_.id == id && override_.validate().is_ok())
    {
        if override_.path.is_some() {
            return None;
        }
        if let Some(source) = override_.source {
            return Some(source);
        }
    }
    Some(config.modules.source)
}

fn invalid_module_overrides(config: &Config) -> Vec<Finding> {
    config
        .modules
        .overrides
        .iter()
        .enumerate()
        .filter(|(_, override_)| override_.validate().is_err())
        .map(|(index, _)| {
            finding(
                MODULE_OVERRIDE_ID,
                FindingSeverity::Warn,
                "A module override is invalid and will be ignored.",
                "Set exactly one of `path` or `source` for each module override.",
                Some(format!("modules.overrides[{index}]")),
            )
        })
        .collect()
}

fn storage_url(config: &Config) -> Vec<Finding> {
    let url = crate::storage::url_from(std::env::var(STORAGE_URL_VAR).ok(), config);
    let Some(url) = url else {
        return Vec::new();
    };

    let parsed = match StorageUrl::parse(&url) {
        Ok(parsed) => parsed,
        Err(error) if error.message().contains("s3 blob URL needs the s3 feature") => {
            return vec![storage_driver_error(STORAGE_URL_ID, "storage-s3")];
        }
        Err(_) => {
            return vec![finding(
                STORAGE_URL_ID,
                FindingSeverity::Error,
                "The configured storage URL is invalid.",
                "Use a supported storage URL scheme and a valid location.",
                Some("storage.url".into()),
            )];
        }
    };

    missing_storage_feature(&parsed).map_or_else(Vec::new, |feature| {
        vec![storage_driver_error(STORAGE_URL_ID, feature)]
    })
}

fn storage_driver_error(check_id: &str, feature: &'static str) -> Finding {
    finding(
        check_id,
        FindingSeverity::Error,
        format!(
            "The configured storage driver is unavailable because this build omits `{feature}`."
        ),
        format!("Use another storage scheme or enable the `{feature}` build feature."),
        Some("storage.url".into()),
    )
}

fn missing_storage_feature(config: &StorageConfig) -> Option<&'static str> {
    match config {
        StorageConfig::Memory | StorageConfig::File { .. } => None,
        // The dependency enables SQLite unconditionally, so this core marker
        // does not represent whether the normal opener can use the driver.
        StorageConfig::Sqlite { .. } => None,
        StorageConfig::MongoDb { .. } => {
            (!cfg!(feature = "storage-mongodb")).then_some("storage-mongodb")
        }
        _ => None,
    }
}

fn action_dir(config: &Config) -> Vec<Finding> {
    let writable = crate::storage::probe_directory_writable(&config.action_dir).is_ok();
    if writable {
        return Vec::new();
    }
    vec![finding(
        ACTION_DIR_ID,
        FindingSeverity::Error,
        "The configured action directory is missing or not writable.",
        "Create the directory or grant the process write access, then rerun doctor.",
        Some("action_dir".into()),
    )]
}

fn master_key_env() -> Vec<Finding> {
    use crate::security::secret_storage::encrypted_file_backend::{
        MASTER_KEY_ENV, MASTER_KEY_FILE_ENV,
    };

    let inline_is_configured = std::env::var_os(MASTER_KEY_ENV)
        .as_deref()
        .is_some_and(
            crate::security::secret_storage::encrypted_file_backend::key_source::env_source_is_configured,
        );
    let file_is_configured = std::env::var_os(MASTER_KEY_FILE_ENV)
        .as_deref()
        .is_some_and(
            crate::security::secret_storage::encrypted_file_backend::key_source::env_source_is_configured,
        );
    if inline_is_configured && file_is_configured {
        return vec![finding(
            MASTER_KEY_ENV_ID,
            FindingSeverity::Error,
            "Both master-key environment sources are configured.",
            "Keep only the intended master-key source configured, then rerun doctor.",
            None,
        )];
    }
    Vec::new()
}

fn proxy(config: &Config) -> Vec<Finding> {
    if config.proxy.validate().is_ok() {
        return Vec::new();
    }
    vec![finding(
        PROXY_ID,
        FindingSeverity::Error,
        "The proxy configuration is invalid.",
        "Correct the proxy URL or service selectors in the proxy configuration.",
        Some("proxy".into()),
    )]
}

fn memory_root(config: &Config) -> Vec<Finding> {
    let Some(root) = config.memory.root.as_deref() else {
        return Vec::new();
    };
    if tinymemory_tools::layout::identity::validate_root(root).is_ok() {
        return Vec::new();
    }
    vec![finding(
        MEMORY_ROOT_ID,
        FindingSeverity::Error,
        "The configured memory root is invalid.",
        "Set memory.root to a supported namespace root and rerun doctor.",
        Some("memory.root".into()),
    )]
}
