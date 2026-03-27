//! Configuration schema: types and defaults for config.toml.
//!
//! Split into submodules; this module re-exports the main `Config` and all public types.

mod accessibility;
mod agent;
mod autonomy;
mod channels;
mod defaults;
mod gateway;
mod hardware;
mod heartbeat_cron;
mod identity_cost;
mod load;
mod observability;
mod proxy;
mod routes;
mod runtime;
mod storage_memory;
mod tools;
mod tunnel;

pub use accessibility::AccessibilityAutomationConfig;
pub use agent::{AgentConfig, DelegateAgentConfig};
pub use autonomy::AutonomyConfig;
pub use channels::{
    AuditConfig, ChannelsConfig, DingTalkConfig, DiscordConfig, IMessageConfig, IrcConfig,
    LarkConfig, LarkReceiveMode, MatrixConfig, MattermostConfig, QQConfig, ResourceLimitsConfig,
    SandboxBackend, SandboxConfig, SecurityConfig, SignalConfig, SlackConfig, StreamMode,
    TelegramConfig, WebhookConfig, WhatsAppConfig,
};
pub use gateway::GatewayConfig;
pub use hardware::{HardwareConfig, HardwareTransport};
pub use heartbeat_cron::{CronConfig, HeartbeatConfig};
pub use identity_cost::{
    CostConfig, IdentityConfig, ModelPricing, PeripheralBoardConfig, PeripheralsConfig,
};
pub use observability::ObservabilityConfig;
pub use proxy::{
    apply_runtime_proxy_to_builder, build_runtime_proxy_client,
    build_runtime_proxy_client_with_timeouts, runtime_proxy_config, set_runtime_proxy_config,
    ProxyConfig, ProxyScope,
};
pub use routes::{
    ClassificationRule, EmbeddingRouteConfig, ModelRouteConfig, QueryClassificationConfig,
};
pub use runtime::{DockerRuntimeConfig, ReliabilityConfig, RuntimeConfig, SchedulerConfig};
pub use storage_memory::{
    MemoryConfig, StorageConfig, StorageProviderConfig, StorageProviderSection,
};
pub use tools::{
    BrowserComputerUseConfig, BrowserConfig, ComposioConfig, HttpRequestConfig, MultimodalConfig,
    SecretsConfig, WebSearchConfig,
};
pub use tunnel::{
    CloudflareTunnelConfig, CustomTunnelConfig, NgrokTunnelConfig, TailscaleTunnelConfig,
    TunnelConfig,
};

use directories::UserDirs;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Top-level configuration (config.toml root).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    #[serde(skip)]
    pub workspace_dir: PathBuf,
    #[serde(skip)]
    pub config_path: PathBuf,
    pub api_key: Option<String>,
    pub api_url: Option<String>,
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub default_temperature: f64,

    #[serde(default)]
    pub observability: ObservabilityConfig,

    #[serde(default)]
    pub autonomy: AutonomyConfig,

    #[serde(default)]
    pub runtime: RuntimeConfig,

    #[serde(default)]
    pub accessibility: AccessibilityAutomationConfig,

    #[serde(default)]
    pub reliability: ReliabilityConfig,

    #[serde(default)]
    pub scheduler: SchedulerConfig,

    #[serde(default)]
    pub agent: AgentConfig,

    #[serde(default)]
    pub model_routes: Vec<ModelRouteConfig>,

    #[serde(default)]
    pub embedding_routes: Vec<EmbeddingRouteConfig>,

    #[serde(default)]
    pub query_classification: QueryClassificationConfig,

    #[serde(default)]
    pub heartbeat: HeartbeatConfig,

    #[serde(default)]
    pub cron: CronConfig,

    #[serde(default)]
    pub channels_config: ChannelsConfig,

    #[serde(default)]
    pub memory: MemoryConfig,

    #[serde(default)]
    pub storage: StorageConfig,

    #[serde(default)]
    pub tunnel: TunnelConfig,

    #[serde(default)]
    pub gateway: GatewayConfig,

    #[serde(default)]
    pub composio: ComposioConfig,

    #[serde(default)]
    pub secrets: SecretsConfig,

    #[serde(default)]
    pub browser: BrowserConfig,

    #[serde(default)]
    pub http_request: HttpRequestConfig,

    #[serde(default)]
    pub multimodal: MultimodalConfig,

    #[serde(default)]
    pub web_search: WebSearchConfig,

    #[serde(default)]
    pub proxy: ProxyConfig,

    #[serde(default)]
    pub identity: IdentityConfig,

    #[serde(default)]
    pub cost: CostConfig,

    #[serde(default)]
    pub peripherals: PeripheralsConfig,

    #[serde(default)]
    pub agents: HashMap<String, DelegateAgentConfig>,

    #[serde(default)]
    pub hardware: HardwareConfig,
}

impl Default for Config {
    fn default() -> Self {
        let home =
            UserDirs::new().map_or_else(|| PathBuf::from("."), |u| u.home_dir().to_path_buf());
        let openhuman_dir = home.join(".openhuman");

        Self {
            workspace_dir: openhuman_dir.join("workspace"),
            config_path: openhuman_dir.join("config.toml"),
            api_key: None,
            api_url: None,
            default_provider: Some("openrouter".to_string()),
            default_model: Some("anthropic/claude-sonnet-4.6".to_string()),
            default_temperature: 0.7,
            observability: ObservabilityConfig::default(),
            autonomy: AutonomyConfig::default(),
            runtime: RuntimeConfig::default(),
            accessibility: AccessibilityAutomationConfig::default(),
            reliability: ReliabilityConfig::default(),
            scheduler: SchedulerConfig::default(),
            agent: AgentConfig::default(),
            model_routes: Vec::new(),
            embedding_routes: Vec::new(),
            heartbeat: HeartbeatConfig::default(),
            cron: CronConfig::default(),
            channels_config: ChannelsConfig::default(),
            memory: MemoryConfig::default(),
            storage: StorageConfig::default(),
            tunnel: TunnelConfig::default(),
            gateway: GatewayConfig::default(),
            composio: ComposioConfig::default(),
            secrets: SecretsConfig::default(),
            browser: BrowserConfig::default(),
            http_request: HttpRequestConfig::default(),
            multimodal: MultimodalConfig::default(),
            web_search: WebSearchConfig::default(),
            proxy: ProxyConfig::default(),
            identity: IdentityConfig::default(),
            cost: CostConfig::default(),
            peripherals: PeripheralsConfig::default(),
            agents: HashMap::new(),
            hardware: HardwareConfig::default(),
            query_classification: QueryClassificationConfig::default(),
        }
    }
}

// Load/save and env overrides extend Config in load.rs
