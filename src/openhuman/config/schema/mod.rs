//! Configuration schema: types and defaults for config.toml.
//!
//! Split into submodules; this module re-exports the main `Config` and all public types.

mod accessibility;
mod agent;
mod autocomplete;
mod autonomy;
mod channels;
mod context;
mod defaults;
mod dictation;
mod heartbeat_cron;
mod identity_cost;
mod learning;
mod load;
pub use load::{
    clear_active_user, default_root_openhuman_dir, pre_login_user_dir, read_active_user_id,
    user_openhuman_dir, write_active_user_id, PRE_LOGIN_USER_ID,
};
mod local_ai;
mod node;
mod observability;
mod proxy;
mod routes;
mod runtime;
mod scheduler_gate;
mod storage_memory;
mod tools;
mod update;

pub use accessibility::ScreenIntelligenceConfig;
pub use agent::{AgentConfig, DelegateAgentConfig};
pub use autocomplete::AutocompleteConfig;
pub use autonomy::AutonomyConfig;
pub use channels::{
    AuditConfig, ChannelsConfig, DingTalkConfig, DiscordConfig, IMessageConfig, IrcConfig,
    LarkConfig, LarkReceiveMode, MatrixConfig, MattermostConfig, QQConfig, ResourceLimitsConfig,
    SandboxBackend, SandboxConfig, SecurityConfig, SignalConfig, SlackConfig, StreamMode,
    TelegramConfig, WebhookConfig, WhatsAppConfig,
};
pub use context::ContextConfig;
pub use dictation::{DictationActivationMode, DictationConfig};
pub use heartbeat_cron::{CronConfig, HeartbeatConfig};
pub use identity_cost::{CostConfig, ModelPricing};
pub use learning::{LearningConfig, ReflectionSource};
pub use local_ai::LocalAiConfig;
pub use node::NodeConfig;
pub use observability::ObservabilityConfig;
pub use proxy::{
    apply_runtime_proxy_to_builder, build_runtime_proxy_client,
    build_runtime_proxy_client_with_timeouts, runtime_proxy_config, set_runtime_proxy_config,
    ProxyConfig, ProxyScope,
};
pub use routes::{EmbeddingRouteConfig, ModelRouteConfig};
pub use runtime::{DockerRuntimeConfig, ReliabilityConfig, RuntimeConfig, SchedulerConfig};
pub use scheduler_gate::{SchedulerGateConfig, SchedulerGateMode};
pub use storage_memory::{
    MemoryConfig, MemoryTreeConfig, StorageConfig, StorageProviderConfig, StorageProviderSection,
};
pub use tools::{
    BrowserComputerUseConfig, BrowserConfig, ComposioConfig, ComputerControlConfig, CurlConfig,
    GitbooksConfig, HttpRequestConfig, IntegrationToggle, IntegrationsConfig, MultimodalConfig,
    SecretsConfig, WebSearchConfig,
};
pub use update::UpdateConfig;
mod voice_server;
pub use voice_server::{VoiceActivationMode, VoiceServerConfig};
mod types;
pub use types::*;
