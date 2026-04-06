use super::*;

use directories::UserDirs;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Standard model identifiers matching the backend model registry.
pub const MODEL_AGENTIC_V1: &str = "agentic-v1";
pub const MODEL_REASONING_V1: &str = "reasoning-v1";
pub const MODEL_CODING_V1: &str = "coding-v1";
/// Default model used when no explicit model is configured.
pub const DEFAULT_MODEL: &str = MODEL_AGENTIC_V1;

/// Top-level configuration (config.toml root).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    #[serde(skip)]
    pub workspace_dir: PathBuf,
    #[serde(skip)]
    pub config_path: PathBuf,
    pub api_key: Option<String>,
    pub api_url: Option<String>,
    pub default_model: Option<String>,
    pub default_temperature: f64,

    #[serde(default)]
    pub observability: ObservabilityConfig,

    #[serde(default)]
    pub autonomy: AutonomyConfig,

    #[serde(default)]
    pub runtime: RuntimeConfig,

    #[serde(default)]
    pub screen_intelligence: ScreenIntelligenceConfig,

    #[serde(default)]
    pub autocomplete: AutocompleteConfig,

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

    #[serde(default)]
    pub local_ai: LocalAiConfig,

    #[serde(default)]
    pub voice_server: VoiceServerConfig,

    #[serde(default)]
    pub integrations: IntegrationsConfig,

    #[serde(default)]
    pub learning: LearningConfig,

    #[serde(default)]
    pub orchestrator: OrchestratorConfig,

    #[serde(default)]
    pub update: UpdateConfig,

    #[serde(default)]
    pub dictation: DictationConfig,

    /// Whether to launch the overlay Tauri app (floating debug/voice panel)
    /// when the core RPC server starts. Defaults to `true`.
    #[serde(default = "default_true")]
    pub overlay_enabled: bool,

    /// Whether the user has completed the onboarding flow.
    #[serde(default)]
    pub onboarding_completed: bool,
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
            default_model: Some(DEFAULT_MODEL.to_string()),
            default_temperature: 0.7,
            observability: ObservabilityConfig::default(),
            autonomy: AutonomyConfig::default(),
            runtime: RuntimeConfig::default(),
            screen_intelligence: ScreenIntelligenceConfig::default(),
            autocomplete: AutocompleteConfig::default(),
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
            local_ai: LocalAiConfig::default(),
            voice_server: VoiceServerConfig::default(),
            query_classification: QueryClassificationConfig::default(),
            integrations: IntegrationsConfig::default(),
            learning: LearningConfig::default(),
            orchestrator: OrchestratorConfig::default(),
            update: UpdateConfig::default(),
            dictation: DictationConfig::default(),
            overlay_enabled: true,
            onboarding_completed: false,
        }
    }
}

// Load/save and env overrides extend Config in load.rs
