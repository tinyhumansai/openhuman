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
///
/// The main (user-facing) agent is a planner/router: its job is to read the
/// user request, decide which sub-agent to delegate to via `spawn_subagent`,
/// and synthesise the final answer from sub-agent outputs. Reasoning-tier
/// models are tuned for that decision-heavy workload, so we pin the main
/// agent to `reasoning-v1` by default. Sub-agents that actually execute tool
/// calls (e.g. `integrations_agent`) explicitly ride on the `agentic` tier via
/// their `ModelSpec::Hint("agentic")` — see `builtin_definitions.rs`.
pub const DEFAULT_MODEL: &str = MODEL_REASONING_V1;

/// Top-level configuration (config.toml root).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    #[serde(skip)]
    pub workspace_dir: PathBuf,
    #[serde(skip)]
    pub config_path: PathBuf,
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

    /// Global context management configuration — budget thresholds,
    /// summarization trigger, microcompact/autocompact toggles, and the
    /// session-memory extraction cadence. Consumed by
    /// [`crate::openhuman::context::ContextManager`].
    #[serde(default)]
    pub context: ContextConfig,

    #[serde(default)]
    pub model_routes: Vec<ModelRouteConfig>,

    #[serde(default)]
    pub embedding_routes: Vec<EmbeddingRouteConfig>,

    #[serde(default)]
    pub heartbeat: HeartbeatConfig,

    #[serde(default)]
    pub cron: CronConfig,

    #[serde(default)]
    pub channels_config: ChannelsConfig,

    #[serde(default)]
    pub memory: MemoryConfig,

    /// Phase 4 memory-tree embedding wiring (#710). Controls whether
    /// ingest/seal pass new chunks/summaries through an Ollama embedder,
    /// and whether missing endpoint config is fatal or warns and falls
    /// back to inert zero vectors.
    #[serde(default)]
    pub memory_tree: MemoryTreeConfig,

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
    pub curl: CurlConfig,

    #[serde(default)]
    pub gitbooks: GitbooksConfig,

    #[serde(default)]
    pub multimodal: MultimodalConfig,

    #[serde(default)]
    pub web_search: WebSearchConfig,

    #[serde(default)]
    pub proxy: ProxyConfig,

    #[serde(default)]
    pub cost: CostConfig,

    #[serde(default)]
    pub computer_control: ComputerControlConfig,

    #[serde(default)]
    pub agents: HashMap<String, DelegateAgentConfig>,

    #[serde(default)]
    pub local_ai: LocalAiConfig,

    /// Node.js managed runtime configuration (skills that need `node`/`npm`).
    #[serde(default)]
    pub node: NodeConfig,

    #[serde(default)]
    pub voice_server: VoiceServerConfig,

    #[serde(default)]
    pub integrations: IntegrationsConfig,

    #[serde(default)]
    pub learning: LearningConfig,

    #[serde(default)]
    pub update: UpdateConfig,

    #[serde(default)]
    pub dictation: DictationConfig,

    /// Whether the user has completed the **React UI** onboarding flow.
    ///
    /// Set by `OnboardingOverlay.tsx::handleDone` and the multi-step
    /// `Onboarding.tsx` wizard via the `config.set_onboarding_completed`
    /// JSON-RPC method. Gates whether the React layer renders the
    /// full-screen onboarding overlay on top of the chat pane: when
    /// `false`, the overlay is shown and the user cannot interact with
    /// the chat until they complete or defer the wizard.
    ///
    /// Distinct from [`Config::chat_onboarding_completed`] — this flag
    /// only tracks the UI wizard, NOT the welcome agent's chat-based
    /// greeting flow. See that field for the agent routing semantics.
    #[serde(default)]
    pub onboarding_completed: bool,

    /// Whether the **chat-based welcome agent** flow has run for this
    /// user. Distinct from [`Config::onboarding_completed`] (the
    /// React UI wizard flag) so the welcome agent can run on the very
    /// first chat turn even after the React wizard has already
    /// completed.
    ///
    /// Routing semantics:
    /// * **`false`** — incoming channel messages and Tauri in-app
    ///   chat turns route to the `welcome` agent definition (see
    ///   `channels::providers::web::build_session_agent` and
    ///   `channels::runtime::dispatch::resolve_target_agent`). The
    ///   welcome agent inspects the user's setup, delivers a
    ///   personalized greeting, and (when the essentials are in
    ///   place) calls `complete_onboarding` which
    ///   flips this flag to `true`.
    /// * **`true`** — the welcome agent has already run; future chat
    ///   turns route to the orchestrator.
    ///
    /// Why two separate flags:
    ///
    /// In the Tauri desktop app, `OnboardingOverlay` blocks the chat
    /// pane until `onboarding_completed=true`. If the welcome agent
    /// also gated on `onboarding_completed`, by the time the user
    /// could type in chat the flag would already be `true` and the
    /// welcome agent would never run on the desktop. Using a separate
    /// flag lets the React wizard manage UI gating while the chat
    /// welcome runs orthogonally — every user gets greeted by the
    /// welcome agent on their first chat turn regardless of which
    /// surface they came from (web, Telegram, Discord, etc.).
    ///
    /// Defaults to `false` for backward compatibility — existing
    /// `config.toml` files without this field will get the welcome
    /// agent on their next chat turn, which is the correct behaviour
    /// (the welcome agent is idempotent and re-running it for an
    /// already-onboarded user just produces a recognition message).
    #[serde(default)]
    pub chat_onboarding_completed: bool,
}

impl Default for Config {
    fn default() -> Self {
        let openhuman_dir =
            crate::openhuman::config::default_root_openhuman_dir().unwrap_or_else(|_| {
                let home = UserDirs::new()
                    .map_or_else(|| PathBuf::from("."), |u| u.home_dir().to_path_buf());
                let dir_name = if crate::api::config::is_staging_app_env(
                    crate::api::config::app_env_from_env().as_deref(),
                ) {
                    ".openhuman-staging"
                } else {
                    ".openhuman"
                };
                home.join(dir_name)
            });

        Self {
            workspace_dir: openhuman_dir.join("workspace"),
            config_path: openhuman_dir.join("config.toml"),
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
            context: ContextConfig::default(),
            model_routes: Vec::new(),
            embedding_routes: Vec::new(),
            heartbeat: HeartbeatConfig::default(),
            cron: CronConfig::default(),
            channels_config: ChannelsConfig::default(),
            memory: MemoryConfig::default(),
            memory_tree: MemoryTreeConfig::default(),
            storage: StorageConfig::default(),
            composio: ComposioConfig::default(),
            secrets: SecretsConfig::default(),
            browser: BrowserConfig::default(),
            http_request: HttpRequestConfig::default(),
            curl: CurlConfig::default(),
            gitbooks: GitbooksConfig::default(),
            multimodal: MultimodalConfig::default(),
            web_search: WebSearchConfig::default(),
            proxy: ProxyConfig::default(),
            cost: CostConfig::default(),
            computer_control: ComputerControlConfig::default(),
            agents: HashMap::new(),
            local_ai: LocalAiConfig::default(),
            node: NodeConfig::default(),
            voice_server: VoiceServerConfig::default(),
            integrations: IntegrationsConfig::default(),
            learning: LearningConfig::default(),
            update: UpdateConfig::default(),
            dictation: DictationConfig::default(),
            onboarding_completed: false,
            chat_onboarding_completed: false,
        }
    }
}

// Load/save and env overrides extend Config in load.rs
