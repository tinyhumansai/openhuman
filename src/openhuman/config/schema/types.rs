use super::*;

use directories::UserDirs;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Standard model identifiers matching the backend model registry.
pub const MODEL_AGENTIC_V1: &str = "agentic-v1";
pub const MODEL_REASONING_V1: &str = "reasoning-v1";
/// Conversational tier — the orchestrator (user-facing chat agent) rides on
/// this by default. Backend maps it to Kimi K2.6 Turbo on Fireworks (128k
/// context, `supportsThinking: false`) — tuned for time-to-first-token so
/// chat responses feel snappy.
pub const MODEL_CHAT_V1: &str = "chat-v1";
/// Low-latency chat tier. Backend maps this to Kimi K2.6 Turbo on
/// Fireworks (128k context, `supportsThinking: false`) — tuned for
/// time-to-first-token on conversational turns. See backend PR #760.
/// The orchestrator (user-facing front-line agent) rides on this tier
/// by default (via `hint:chat`) so chat responses feel snappy; reach
/// for the slower `reasoning-v1` (DeepSeek V4 Pro) only when deep
/// reasoning is needed.
pub const MODEL_REASONING_QUICK_V1: &str = "reasoning-quick-v1";
pub const MODEL_CODING_V1: &str = "coding-v1";
/// Default model used when no explicit model is configured.
///
/// The orchestrator (user-facing chat agent) reads the user's message and
/// either replies directly or delegates to a sub-agent via `spawn_subagent`.
/// We route it through the `chat` workload (`hint:chat`) so the user-facing
/// `chat_provider` setting in Settings → LLM → Routing actually drives the
/// main chat turn — and so the orchestrator gets the low-latency `chat` tier
/// by default (backend maps `hint:chat` to Kimi K2.6 Turbo, tuned for
/// time-to-first-token; see backend PR #760). Sub-agents that actually
/// execute tool calls explicitly ride on `hint:agentic`/`hint:coding` via
/// their `ModelSpec::Hint(...)` declarations — see `builtin_definitions.rs`.
pub const DEFAULT_MODEL: &str = MODEL_CHAT_V1;

/// Top-level configuration (config.toml root).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    #[serde(skip)]
    pub workspace_dir: PathBuf,
    #[serde(skip)]
    pub config_path: PathBuf,
    /// Workspace data-schema version. Bumped each time a one-shot data
    /// migration under [`crate::openhuman::migrations`] runs successfully.
    /// `#[serde(default)]` so existing `config.toml` files (which predate
    /// the field) load as version `0` and pick up pending migrations on
    /// the first launch of the new build.
    #[serde(default)]
    pub schema_version: u32,
    pub api_url: Option<String>,
    pub api_key: Option<String>,
    /// Custom LLM inference endpoint (OpenAI-compatible). When set together
    /// with `api_key`, the inference provider talks directly to this URL
    /// instead of routing through the OpenHuman backend. Account/auth/billing
    /// calls always continue to use `api_url` — keeping inference and
    /// product-backend concerns cleanly separated.
    #[serde(default)]
    pub inference_url: Option<String>,
    pub default_model: Option<String>,
    #[serde(default = "default_temperature_value")]
    pub default_temperature: f64,

    /// Models (by exact ID match OR shell-style glob like `gpt-5*`, `o1-*`) that
    /// MUST NOT receive a `temperature` parameter. Used for reasoning models
    /// that error out when temperature is set (OpenAI o-series, GPT-5).
    #[serde(default = "default_temperature_unsupported_models")]
    pub temperature_unsupported_models: Vec<String>,

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

    /// Background-AI scheduler gate — throttles memory-tree digests,
    /// embeddings, and other LLM-bound background work based on power
    /// state, CPU pressure, and deployment mode. See
    /// [`crate::openhuman::scheduler_gate`].
    #[serde(default)]
    pub scheduler_gate: SchedulerGateConfig,

    #[serde(default)]
    pub agent: AgentConfig,

    /// Optional model pin for the front-line orchestrator. Provider
    /// selection still follows the normal reasoning workload; this only
    /// replaces the resolved model id when set.
    #[serde(default)]
    pub orchestrator: OrchestratorModelConfig,

    /// Optional per-team model pins for delegated swarms.
    ///
    /// Example:
    /// `[teams.research] lead_model = "minimax/m2" agent_model = "deepseek/v3.2"`.
    #[serde(default)]
    pub teams: HashMap<String, TeamModelConfig>,

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
    pub mcp_client: McpClientConfig,

    #[serde(default)]
    pub multimodal: MultimodalConfig,

    #[serde(default)]
    pub seltz: SeltzConfig,

    #[serde(default)]
    pub searxng: SearxngConfig,

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

    // ── Unified AI provider routing ──────────────────────────────────────────
    //
    // Provider-string grammar (consumed by `providers::factory`):
    //
    //   "cloud"                → resolves to `primary_cloud`; if primary is
    //                            openhuman, behaves identically to "openhuman"
    //   "openhuman"            → OpenHuman backend (api_url + api_key session JWT)
    //   "openai:<model>"       → look up cloud_providers entry of type=openai;
    //                            build OpenAiCompatibleProvider with Bearer auth
    //   "anthropic:<model>"    → type=anthropic; Bearer auth on the compat endpoint
    //   "openrouter:<model>"   → type=openrouter; Bearer auth
    //   "orcarouter:<model>"   → type=orcarouter; Bearer auth (e.g. "orcarouter:orcarouter/auto")
    //   "custom:<model>"       → type=custom; Bearer auth
    //   "ollama:<model>"       → local Ollama at config.local_ai.base_url
    //
    // Per-workload fields default to None, which the factory treats as "cloud".
    // Changing `primary_cloud` instantly re-routes every "cloud" workload.
    /// Registered cloud providers. Index 0 is always the built-in OpenHuman
    /// entry; additional entries are user-added third-party backends.
    #[serde(default)]
    pub cloud_providers: Vec<crate::openhuman::config::schema::cloud_providers::CloudProviderCreds>,

    /// Id of the `cloud_providers` entry that "cloud" and "primary" resolve to.
    /// When `None`, the factory falls back to the OpenHuman entry.
    #[serde(default)]
    pub primary_cloud: Option<String>,

    /// Provider string for direct conversational chat (simple back-and-forth).
    #[serde(default)]
    pub chat_provider: Option<String>,

    /// Provider string for the main reasoning / chat workload.
    #[serde(default)]
    pub reasoning_provider: Option<String>,

    /// Provider string for sub-agent execution and tool-loop workloads.
    #[serde(default)]
    pub agentic_provider: Option<String>,

    /// Provider string for code generation and refactor workloads.
    #[serde(default)]
    pub coding_provider: Option<String>,

    /// Provider string for memory-tree extract + summarise workloads.
    #[serde(default)]
    pub memory_provider: Option<String>,

    /// Provider string for embedding generation.
    #[serde(default)]
    pub embeddings_provider: Option<String>,

    /// Provider string for the heartbeat background-reasoning loop.
    #[serde(default)]
    pub heartbeat_provider: Option<String>,

    /// Provider string for learning / reflection passes.
    #[serde(default)]
    pub learning_provider: Option<String>,

    /// Provider string for subconscious evaluation and drift checks.
    #[serde(default)]
    pub subconscious_provider: Option<String>,

    /// Node.js managed runtime configuration (skills that need `node`/`npm`).
    #[serde(default)]
    pub node: NodeConfig,

    /// Python managed runtime configuration (Python-backed MCP servers and
    /// other Python subprocess integrations).
    #[serde(default)]
    pub runtime_python: RuntimePythonConfig,

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

    /// Google Meet integration settings — currently the
    /// `auto_orchestrator_handoff` privacy gate (see
    /// [`crate::openhuman::config::schema::MeetConfig`]).
    #[serde(default)]
    pub meet: MeetConfig,

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

/// Shared default so `#[serde(default)]` and `Config::default()` stay in sync.
pub(crate) const DEFAULT_TEMPERATURE: f64 = 0.7;

/// Returns the default temperature used by `#[serde(default = "default_temperature_value")]`.
/// A bare `#[serde(default)]` would give `0.0`; this ensures the field
/// round-trips correctly even when `default_temperature` is omitted from
/// an existing `config.toml`.
fn default_temperature_value() -> f64 {
    DEFAULT_TEMPERATURE
}

/// Returns the default list of model glob patterns that do not support the
/// `temperature` parameter. These cover OpenAI o-series and GPT-5 reasoning
/// models that return an error when `temperature` is included in the request,
/// as well as Moonshot's Kimi K2 family which only accepts `temperature: 1`
/// (see #2076 — 146 Sentry events from users in China hitting *"invalid
/// temperature: only 1 is allowed for this model"* on `kimi-k2.6`).
fn default_temperature_unsupported_models() -> Vec<String> {
    vec![
        "o1*".to_string(),
        "o3*".to_string(),
        "o4*".to_string(),
        "gpt-5*".to_string(),
        // Moonshot Kimi K2 family — temperature must be omitted (the
        // upstream defaults to 1.0). Covers `kimi-k2.6`, `kimi-k2-instruct`,
        // and any future K2 variants. See #2076.
        "kimi-k2*".to_string(),
        // OpenRouter / third-party gateways often namespace Kimi as
        // `moonshot/...` or `moonshotai/...`. Match those routings too so
        // users hitting Kimi through OpenRouter get the same suppression.
        "moonshot*".to_string(),
        "moonshotai/*".to_string(),
    ]
}

impl Config {
    /// Resolve the root directory where chunk `.md` files are stored.
    ///
    /// Resolution order:
    /// 1. `memory_tree.content_dir` if `Some`.
    /// 2. Default: `<workspace_dir>/memory_tree/content/`.
    ///
    /// This is the only place in the codebase that should compute the content
    /// root — all code that needs the path should call this method.
    pub fn memory_tree_content_root(&self) -> PathBuf {
        self.memory_tree
            .content_dir
            .clone()
            .unwrap_or_else(|| self.workspace_dir.join("memory_tree").join("content"))
    }

    /// Read the per-workload provider string and return the local model id
    /// when the workload is routed to Ollama.
    ///
    /// Recognised workload names:
    /// `"chat"`, `"reasoning"`, `"agentic"`, `"coding"`, `"memory"`, `"embeddings"`,
    /// `"heartbeat"`, `"learning"`, `"subconscious"`.
    ///
    /// Returns `None` when the provider isn't `"ollama:<model>"` (including
    /// when the field is unset, blank, `"cloud"`, or any other prefix).
    /// This is the single source of truth for "is this workload local?" —
    /// callers MUST NOT consult the legacy `local_ai.usage.*` booleans or
    /// `memory_tree.llm_backend`. Those fields are deprecated zombies kept
    /// for migration only.
    pub fn workload_local_model(&self, workload: &str) -> Option<String> {
        let raw = match workload {
            "chat" => self.chat_provider.as_deref(),
            "reasoning" => self.reasoning_provider.as_deref(),
            "agentic" => self.agentic_provider.as_deref(),
            "coding" => self.coding_provider.as_deref(),
            "memory" => self.memory_provider.as_deref(),
            "embeddings" => self.embeddings_provider.as_deref(),
            "heartbeat" => self.heartbeat_provider.as_deref(),
            "learning" => self.learning_provider.as_deref(),
            "subconscious" => self.subconscious_provider.as_deref(),
            _ => None,
        }?;
        let trimmed = raw.trim();
        let model = trimmed.strip_prefix("ollama:")?.trim();
        if model.is_empty() {
            None
        } else {
            Some(model.to_string())
        }
    }

    /// `true` when `workload_local_model` returns `Some` for the named
    /// workload. Convenience wrapper for the common "do I dispatch
    /// locally?" branch.
    pub fn workload_uses_local(&self, workload: &str) -> bool {
        self.workload_local_model(workload).is_some()
    }

    /// Resolve an exact model pin for an agent, if configured.
    ///
    /// Precedence is intentionally narrow and deterministic:
    /// 1. `orchestrator.model` when resolving the front-line orchestrator.
    /// 2. `[teams.<agent_id>]` entries, with `lead_model` used for agents
    ///    that can delegate and `agent_model` used for leaf workers.
    /// 3. Built-in aliases such as `[teams.research]` for `researcher` and
    ///    `[teams.code]` for `code_executor`, matching the issue examples.
    ///
    /// Empty strings are ignored so partially-written configs fall back to
    /// the existing auto-routing path.
    pub fn configured_agent_model(&self, agent_id: &str, is_team_lead: bool) -> Option<&str> {
        fn clean(model: Option<&str>) -> Option<&str> {
            model.map(str::trim).filter(|value| !value.is_empty())
        }

        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return None;
        }

        if agent_id == "orchestrator" {
            if let Some(model) = clean(self.orchestrator.model.as_deref()) {
                return Some(model);
            }
        }

        if let Some(model) = self
            .teams
            .get(agent_id)
            .and_then(|team| team.model_for_role(is_team_lead))
        {
            return Some(model);
        }

        if let Some(stripped) = agent_id.strip_suffix("_agent") {
            if let Some(model) = self
                .teams
                .get(stripped)
                .and_then(|team| team.model_for_role(is_team_lead))
            {
                return Some(model);
            }
        }

        let aliases: &[&str] = match agent_id {
            "researcher" => &["research"],
            "code_executor" => &["code"],
            "tool_maker" | "tools_agent" => &["tools"],
            "integrations_agent" => &["integrations"],
            _ => &[],
        };

        aliases.iter().find_map(|alias| {
            self.teams
                .get(*alias)
                .and_then(|team| team.model_for_role(is_team_lead))
        })
    }
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
            schema_version: 0,
            api_url: None,
            api_key: None,
            inference_url: None,
            default_model: Some(DEFAULT_MODEL.to_string()),
            default_temperature: DEFAULT_TEMPERATURE,
            temperature_unsupported_models: default_temperature_unsupported_models(),
            observability: ObservabilityConfig::default(),
            autonomy: AutonomyConfig::default(),
            runtime: RuntimeConfig::default(),
            screen_intelligence: ScreenIntelligenceConfig::default(),
            autocomplete: AutocompleteConfig::default(),
            reliability: ReliabilityConfig::default(),
            scheduler: SchedulerConfig::default(),
            scheduler_gate: SchedulerGateConfig::default(),
            agent: AgentConfig::default(),
            orchestrator: OrchestratorModelConfig::default(),
            teams: HashMap::new(),
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
            mcp_client: McpClientConfig::default(),
            multimodal: MultimodalConfig::default(),
            seltz: SeltzConfig::default(),
            searxng: SearxngConfig::default(),
            web_search: WebSearchConfig::default(),
            proxy: ProxyConfig::default(),
            cost: CostConfig::default(),
            computer_control: ComputerControlConfig::default(),
            agents: HashMap::new(),
            local_ai: LocalAiConfig::default(),
            cloud_providers: Vec::new(),
            primary_cloud: None,
            chat_provider: None,
            reasoning_provider: None,
            agentic_provider: None,
            coding_provider: None,
            memory_provider: None,
            embeddings_provider: None,
            heartbeat_provider: None,
            learning_provider: None,
            subconscious_provider: None,
            node: NodeConfig::default(),
            runtime_python: RuntimePythonConfig::default(),
            voice_server: VoiceServerConfig::default(),
            integrations: IntegrationsConfig::default(),
            learning: LearningConfig::default(),
            update: UpdateConfig::default(),
            dictation: DictationConfig::default(),
            meet: MeetConfig::default(),
            onboarding_completed: false,
            chat_onboarding_completed: false,
        }
    }
}

// Load/save and env overrides extend Config in load.rs

#[cfg(test)]
mod model_pin_tests {
    use super::*;

    #[test]
    fn config_parses_orchestrator_and_team_model_pins() {
        let config: Config = toml::from_str(
            r#"
                [orchestrator]
                model = "deepseek/deepseek-r2"

                [teams.research]
                lead_model = "minimax/m2"
                agent_model = "deepseek/v3.2"

                [teams.code]
                agent_model = "qwen/qwen3"
            "#,
        )
        .expect("config should parse model pin tables");

        assert_eq!(
            config.configured_agent_model("orchestrator", true),
            Some("deepseek/deepseek-r2")
        );
        assert_eq!(
            config.configured_agent_model("researcher", false),
            Some("deepseek/v3.2")
        );
        assert_eq!(
            config.configured_agent_model("researcher", true),
            Some("minimax/m2")
        );
        assert_eq!(
            config.configured_agent_model("code_executor", false),
            Some("qwen/qwen3")
        );
    }

    #[test]
    fn empty_model_pin_values_fall_back_to_auto_routing() {
        let mut config = Config::default();
        config.orchestrator.model = Some("   ".to_string());
        config.teams.insert(
            "research".to_string(),
            TeamModelConfig {
                lead_model: Some("".to_string()),
                agent_model: Some("  ".to_string()),
            },
        );

        assert_eq!(config.configured_agent_model("orchestrator", true), None);
        assert_eq!(config.configured_agent_model("researcher", false), None);
    }
}
