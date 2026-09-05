//! Configuration schema: types and defaults for config.toml.
//!
//! Split into submodules; this module re-exports the main `Config` and all public types.

pub mod activity_level;
pub use activity_level::AgentActivityLevel;
pub mod cloud_providers;
pub use cloud_providers::{
    generate_provider_id, is_slug_reserved, migrate_legacy_fields, AuthStyle, CloudProviderCreds,
    CloudProviderType,
};
pub mod ephemeral_route;
pub use ephemeral_route::{EphemeralRoute, EPHEMERAL_ROUTE_SLUG};
pub mod subconscious;
pub use subconscious::{MedullaLocalConfig, SubconsciousConfig, SubconsciousEngine};
mod agent;
mod autonomy;
mod capability_providers;
mod channels;
mod cli_overrides;
#[doc(hidden)]
pub use cli_overrides::AppliedInferenceOverride;
mod context;
mod dashboard;
mod defaults;
mod dictation;
mod hooks;
pub use hooks::HooksConfig;
mod heartbeat_cron;
pub mod hosting;
pub use hosting::HostingConfig;
mod identity_cost;
mod learning;
mod load;
pub use load::{
    action_dir_env_override, active_user_marker_path, active_workspace_dir,
    active_workspace_dir_cached, active_workspace_snapshot, clear_active_user, default_action_dir,
    default_projects_dir, default_root_openhuman_dir, pre_login_user_dir, read_active_user_id,
    resolve_action_dir, user_openhuman_dir, write_active_user_id, PRE_LOGIN_USER_ID,
};
// Crate-internal: the invalidation half of the cached active workspace. The
// marker writers in `load_user_state` call it from outside `load`; the
// write-through half stays inside `load`, where the two resolvers live. Every
// other caller reads through `active_workspace_dir_cached`.
pub(crate) use load::invalidate_active_workspace;
// Crate-internal: the workspace→config-dir resolver, reused by the cloud
// embedder's keyless credential-scope resolution (mirrors `config::load`).
pub(crate) use load::resolve_config_dir_for_workspace;
// Contract shared with `core::observability::expected_error_kind`: the loader
// appends this marker to a config-read failure when the file's owner differs
// from the reading process, and the classifier keys on it to keep that case
// paging. Exported so the classifier tests pin the real constant.
pub(crate) use load::CONFIG_OWNER_MISMATCH_MARKER;
pub mod claude_agent_sdk;
pub use claude_agent_sdk::ClaudeAgentSdkConfig;
mod local_ai;
mod modules;
mod node;
mod observability;
mod privacy;
mod proxy;
mod routes;
mod runtime;
mod runtime_pool;
mod runtime_python;
mod scheduler_gate;
mod storage_memory;
mod subsystems;
mod task_sources;
mod tokenjuice;
mod tools;
mod update;
mod youpet;

pub use agent::{
    AgentConfig, DelegateAgentConfig, MemoryContextWindow, MemoryWindowLimits,
    OrchestratorModelConfig, RequiredOutputContract, TeamModelConfig,
};
pub use autonomy::AutonomyConfig;
pub use capability_providers::{CapabilityProviderConfig, CapabilityProviderTrustState};
pub use channels::{
    AuditConfig, ChannelsConfig, DingTalkConfig, DiscordConfig, EmailConfig, IMessageConfig,
    IrcConfig, LarkConfig, LarkReceiveMode, LinqConfig, MatrixConfig, MattermostConfig, QQConfig,
    ResourceLimitsConfig, SandboxBackend, SandboxConfig, SecurityConfig, SignalConfig, SlackConfig,
    StreamMode, TelegramConfig, WebhookConfig, WhatsAppConfig, YuanbaoConfig,
};
pub(crate) use cli_overrides::set_cli_inference_overrides;
pub use context::ContextConfig;
pub use dashboard::{DashboardConfig, DiagramViewerConfig, EventStreamConfig, ModelHealthConfig};
pub use dictation::{DictationActivationMode, DictationConfig};
pub use heartbeat_cron::{CronConfig, HeartbeatConfig, SubconsciousMode};
pub use identity_cost::{CostConfig, ModelPricing};
pub use learning::{LearningConfig, ReflectionSource};
pub use local_ai::{LocalAiConfig, LocalAiUsage};
pub use modules::{ModuleOverride, ModulesConfig};
pub use node::NodeConfig;
pub use observability::{AgentTracingBackend, AgentTracingConfig, ObservabilityConfig};
pub use privacy::{PrivacyConfig, PrivacyMode};
pub use proxy::{
    apply_runtime_proxy_to_builder, build_runtime_proxy_client,
    build_runtime_proxy_client_with_timeouts, runtime_proxy_config, set_runtime_proxy_config,
    ProxyConfig, ProxyScope,
};
pub use routes::{EmbeddingRouteConfig, ModelRouteConfig};
pub use runtime::{
    DockerRuntimeConfig, ReliabilityConfig, RuntimeConfig, SchedulerConfig, ShellConfig,
};
pub use runtime_pool::{RuntimePoolConfig, RuntimePoolLangConfig};
pub use runtime_python::RuntimePythonConfig;
pub use scheduler_gate::{SchedulerGateConfig, SchedulerGateMode};
pub use storage_memory::{
    LlmBackend, MemoryConfig, MemoryTreeConfig, StorageConfig, StorageProviderConfig,
    StorageProviderSection, DEFAULT_CLOUD_LLM_MODEL,
};
pub use subsystems::{
    MemoryDriverConfig, MemoryHooksConfig, MemorySubsystemConfig, SubsystemsConfig,
};
pub use task_sources::TaskSourcesConfig;
pub use tokenjuice::TokenjuiceConfig;
pub use tools::{
    BrowserComputerUseConfig, BrowserConfig, ComposioConfig, CurlConfig, GitbooksConfig,
    HttpHeader, HttpRequestConfig, IntegrationToggle, IntegrationsConfig, McpAuthConfig,
    McpClientConfig, McpClientIdentityConfig, McpServerConfig, MultimodalConfig,
    MultimodalFileConfig, SearchConfig, SearchEngine, SearchEngineCredentials, SearxngConfig,
    SecretsConfig, SeltzConfig, WebSearchConfig, COMPOSIO_MODE_BACKEND, COMPOSIO_MODE_DIRECT,
    SEARCH_ENGINE_BRAVE, SEARCH_ENGINE_DISABLED, SEARCH_ENGINE_EXA, SEARCH_ENGINE_MANAGED,
    SEARCH_ENGINE_PARALLEL, SEARCH_ENGINE_QUERIT,
};
pub use update::{UpdateConfig, UpdateRestartStrategy};
pub use youpet::YouPetConfig;
mod voice_server;
pub use voice_server::{SttEngine, VoiceActivationMode, VoiceServerConfig};
pub mod voice_providers;
pub use voice_providers::{
    generate_voice_provider_id, is_voice_slug_reserved, BuiltinVoiceProvider, SttApiStyle,
    TtsApiStyle, VoiceCapability, VoiceProviderCreds, BUILTIN_VOICE_PROVIDERS,
};
mod types;
pub use types::*;
