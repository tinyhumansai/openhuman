//! Configuration schema: types and defaults for config.toml.
//!
//! Split into submodules; this module re-exports the main `Config` and all public types.

pub mod cloud_providers;
pub use cloud_providers::{
    generate_provider_id, is_slug_reserved, migrate_legacy_fields, AuthStyle, CloudProviderCreds,
    CloudProviderType,
};
mod accessibility;
mod agent;
mod autocomplete;
mod autonomy;
mod capability_providers;
mod channels;
mod context;
mod dashboard;
mod defaults;
mod dictation;
mod heartbeat_cron;
mod identity_cost;
mod learning;
mod load;
pub use load::{
    clear_active_user, default_projects_dir, default_root_openhuman_dir, pre_login_user_dir,
    read_active_user_id, user_openhuman_dir, write_active_user_id, PRE_LOGIN_USER_ID,
};
pub mod claude_agent_sdk;
pub use claude_agent_sdk::ClaudeAgentSdkConfig;
mod local_ai;
mod meet;
mod node;
mod observability;
mod proxy;
mod routes;
mod runtime;
mod runtime_python;
mod scheduler_gate;
mod storage_memory;
mod task_sources;
mod tools;
mod update;

pub use accessibility::ScreenIntelligenceConfig;
pub use agent::{
    AgentConfig, DelegateAgentConfig, MemoryContextWindow, MemoryWindowLimits,
    OrchestratorModelConfig, TeamModelConfig,
};
pub use autocomplete::AutocompleteConfig;
pub use autonomy::AutonomyConfig;
pub use capability_providers::{CapabilityProviderConfig, CapabilityProviderTrustState};
pub use channels::{
    AuditConfig, ChannelsConfig, DingTalkConfig, DiscordConfig, IMessageConfig, IrcConfig,
    LarkConfig, LarkReceiveMode, MatrixConfig, MattermostConfig, QQConfig, ResourceLimitsConfig,
    SandboxBackend, SandboxConfig, SecurityConfig, SignalConfig, SlackConfig, StreamMode,
    TelegramConfig, WebhookConfig, WhatsAppConfig,
};
pub use context::ContextConfig;
pub use dashboard::{DashboardConfig, DiagramViewerConfig, EventStreamConfig, ModelHealthConfig};
pub use dictation::{DictationActivationMode, DictationConfig};
pub use heartbeat_cron::{CronConfig, HeartbeatConfig};
pub use identity_cost::{CostConfig, ModelPricing};
pub use learning::{LearningConfig, ReflectionSource};
pub use local_ai::{LocalAiConfig, LocalAiUsage};
pub use meet::MeetConfig;
pub use node::NodeConfig;
pub use observability::ObservabilityConfig;
pub use proxy::{
    apply_runtime_proxy_to_builder, build_runtime_proxy_client,
    build_runtime_proxy_client_with_timeouts, runtime_proxy_config, set_runtime_proxy_config,
    ProxyConfig, ProxyScope,
};
pub use routes::{EmbeddingRouteConfig, ModelRouteConfig};
pub use runtime::{DockerRuntimeConfig, ReliabilityConfig, RuntimeConfig, SchedulerConfig};
pub use runtime_python::RuntimePythonConfig;
pub use scheduler_gate::{SchedulerGateConfig, SchedulerGateMode};
pub use storage_memory::{
    LlmBackend, MemoryConfig, MemoryTreeConfig, StorageConfig, StorageProviderConfig,
    StorageProviderSection, DEFAULT_CLOUD_LLM_MODEL,
};
pub use task_sources::TaskSourcesConfig;
pub use tools::{
    BrowserComputerUseConfig, BrowserConfig, ComposioConfig, ComputerControlConfig, CurlConfig,
    GitbooksConfig, HttpRequestConfig, IntegrationToggle, IntegrationsConfig, McpAuthConfig,
    McpClientConfig, McpClientIdentityConfig, McpServerConfig, MultimodalConfig,
    PolymarketClobCredentials, PolymarketConfig, SearchConfig, SearchEngine,
    SearchEngineCredentials, SearxngConfig, SecretsConfig, SeltzConfig, WebSearchConfig,
    COMPOSIO_MODE_BACKEND, COMPOSIO_MODE_DIRECT, SEARCH_ENGINE_BRAVE, SEARCH_ENGINE_DISABLED,
    SEARCH_ENGINE_MANAGED, SEARCH_ENGINE_PARALLEL, SEARCH_ENGINE_QUERIT,
};
pub use update::{UpdateConfig, UpdateRestartStrategy};
mod voice_server;
pub use voice_server::{VoiceActivationMode, VoiceServerConfig};
pub mod voice_providers;
pub use voice_providers::{
    generate_voice_provider_id, is_voice_slug_reserved, BuiltinVoiceProvider, SttApiStyle,
    TtsApiStyle, VoiceCapability, VoiceProviderCreds, BUILTIN_VOICE_PROVIDERS,
};
mod types;
pub use types::*;
