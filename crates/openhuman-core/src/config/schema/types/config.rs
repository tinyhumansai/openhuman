//! The persisted [`Config`] document and the helper types nested directly in
//! it, plus the serde default functions its attributes reference.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::config::schema::*;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ModelRegistryEntry {
    pub id: String,
    pub provider: String,
    /// Standard prompt rate, USD per **million input tokens**. Used (together
    /// with [`Self::cost_per_1m_output`]) to estimate request cost when the
    /// provider doesn't echo an authoritative `charged_amount_usd`. `0.0` means
    /// "unknown" — callers fall back to the tier/catalog estimate. Pre-filled
    /// for known vendor models from [`crate::platform::cost::catalog`].
    #[serde(default)]
    pub cost_per_1m_input: f64,
    /// Cached-prefix prompt rate, USD per million cached input tokens (KV-cache
    /// read hits on supporting backends). `0.0` means "unknown".
    #[serde(default)]
    pub cost_per_1m_cached_input: f64,
    /// Completion rate, USD per **million output tokens**.
    #[serde(default)]
    pub cost_per_1m_output: f64,
    /// Maximum context window in tokens (published max input). `0` means
    /// "unknown". Providers differ widely (128K–1M+); callers use this to
    /// budget prompts, trigger compaction, and route work. Pre-filled for known
    /// vendor models from [`crate::platform::cost::catalog`].
    #[serde(default)]
    pub context_window: u32,
    #[serde(default)]
    pub vision: bool,
}

/// Last successfully verified Custom embeddings configuration.
///
/// The active provider lives in [`Config::embeddings_provider`] and
/// [`MemoryConfig::embedding_provider`]. Keeping this profile separately lets
/// users temporarily disable embeddings (or select another provider) without
/// losing the endpoint/model they configured for a later switch back.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct CustomEmbeddingsConfig {
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
}

/// Top-level configuration (config.toml root).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    #[serde(skip)]
    pub workspace_dir: PathBuf,
    /// Agent action sandbox root — the default cwd for shell/file/git tools.
    /// Kept separate from `workspace_dir` (which holds internal state like
    /// memory DBs, sessions, tokens). Defaults to `~/OpenHuman/projects`
    /// (`default_action_dir()`); overridable via `OPENHUMAN_ACTION_DIR`.
    ///
    /// This is the **resolved runtime value** and is `#[serde(skip)]` — it is
    /// recomputed on every load from the precedence chain
    /// (env `OPENHUMAN_ACTION_DIR` > [`Self::action_dir_override`] > default).
    /// To persist a user choice, write [`Self::action_dir_override`] instead.
    #[serde(skip)]
    pub action_dir: PathBuf,
    /// Persisted user override for [`Self::action_dir`], set via the Settings UI
    /// (`config.update_agent_paths` RPC). Unlike `action_dir`, this field **is**
    /// serialized so the choice survives restarts. Resolution precedence on load:
    /// env `OPENHUMAN_ACTION_DIR` wins, then this override (when `Some`), then the
    /// default projects dir. `None` means "use the default" — the env var still
    /// overrides at runtime so existing env-driven deployments are unaffected.
    #[serde(default)]
    pub action_dir_override: Option<PathBuf>,
    /// Persisted user choice for the folder agent deliverables are written to,
    /// set via Settings → Agent OS access (`config.update_agent_paths`,
    /// #5505). `None` means the default, `~/OpenHuman/projects/Files`. Read it
    /// through [`Self::files_dir`]. Changing it affects new artifacts only;
    /// existing ones keep the folder recorded in their metadata.
    #[serde(default)]
    pub files_dir_override: Option<PathBuf>,
    /// Files folders used before the current one. Artifacts created there keep
    /// resolving, because the artifact escape guard trusts only folders the
    /// core records here (plus the current one and the default), never a
    /// folder an artifact's own metadata claims. Appended by
    /// `config.update_agent_paths`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files_dir_history: Vec<PathBuf>,
    #[serde(skip)]
    pub config_path: PathBuf,
    /// Per-load snapshot used to remove standalone CLI inference overrides
    /// from a saved clone. Runtime-only and never serialized. Public only so
    /// external integration tests and embedding crates can continue to use
    /// struct-update syntax with this public configuration type.
    #[serde(skip)]
    #[schemars(skip)]
    #[doc(hidden)]
    pub cli_inference_snapshot: Option<AppliedInferenceOverride>,
    /// Runtime only — `true` when this config was produced by the loader's
    /// corruption-recovery path: the on-disk `config.toml` was unreadable
    /// (non-UTF-8) or unparseable, so it was renamed to `.corrupted.<ts>` and the
    /// config was reset to defaults (or restored from `.bak`). Never persisted and
    /// recomputed on every load. Read once at boot by `bootstrap_core_runtime` to
    /// raise a user-visible "settings were reset" notice (#5167).
    #[serde(skip)]
    pub recovered_from_corruption: bool,
    /// Workspace data-schema version. Bumped each time a one-shot data
    /// migration under [`crate::config::migrations`] runs successfully.
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

    /// Optional language for background LLM artifacts such as
    /// summaries and generated briefs. Accepts either
    /// a known UI locale tag (for example `zh-CN`) or a human-readable language
    /// name. `None` preserves the existing default-language behaviour.
    #[serde(default)]
    pub output_language: Option<String>,

    /// Models (by exact ID match OR shell-style glob like `gpt-5*`, `o1-*`) that
    /// MUST NOT receive a `temperature` parameter. Used for reasoning models
    /// that error out when temperature is set (OpenAI o-series, GPT-5).
    #[serde(default = "default_temperature_unsupported_models")]
    pub temperature_unsupported_models: Vec<String>,

    #[serde(default)]
    pub dashboard: DashboardConfig,

    #[serde(default)]
    pub observability: ObservabilityConfig,

    #[serde(default)]
    pub autonomy: AutonomyConfig,

    #[serde(default)]
    pub desktop: DesktopConfig,

    /// TinyComputer decision, planner and rescue models.
    #[serde(default)]
    pub computer: ComputerConfig,

    /// Host-level switches for the configurable hook system. The hooks
    /// themselves live in `hooks.json` files, not here — see
    /// [`HooksConfig`].
    #[serde(default)]
    pub hooks: HooksConfig,

    /// Data-egress posture (Privacy Mode). Distinct from `autonomy` (which
    /// governs agent *act* power). Missing `[privacy]` block → `Standard`
    /// (#4435, epic #4256).
    #[serde(default)]
    pub privacy: PrivacyConfig,

    #[serde(default)]
    pub sandbox: SandboxConfig,

    #[serde(default)]
    pub runtime: RuntimeConfig,

    #[serde(default)]
    pub shell: ShellConfig,

    /// `[web_chat]` — web chat presentation-layer toggles (currently just
    /// the post-turn follow-up-suggestions model call).
    #[serde(default)]
    pub web_chat: crate::config::schema::WebChatConfig,

    #[serde(default)]
    pub reliability: ReliabilityConfig,

    #[serde(default)]
    pub scheduler: SchedulerConfig,

    /// Background-AI scheduler gate — throttles embeddings and other
    /// LLM-bound background work based on power
    /// state, CPU pressure, and deployment mode. See
    /// [`crate::cron::scheduler_gate`].
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
    /// `[teams.research] lead_model = "minimax/m3" agent_model = "deepseek/v3.2"`.
    #[serde(default)]
    pub teams: HashMap<String, TeamModelConfig>,

    /// Global context management configuration — budget thresholds,
    /// summarization trigger, microcompact/autocompact toggles, and the
    /// session-memory extraction cadence. Consumed by
    /// [`crate::agent::context::ContextManager`].
    #[serde(default)]
    pub context: ContextConfig,

    #[serde(default)]
    pub model_routes: Vec<ModelRouteConfig>,

    #[serde(default)]
    pub embedding_routes: Vec<EmbeddingRouteConfig>,

    #[serde(default)]
    pub cron: CronConfig,

    /// Task-sources domain defaults — master switch + new-source
    /// defaults. Per-source records live in the domain's SQLite store.
    /// See [`crate::integrations::task_sources`].
    #[serde(default)]
    pub task_sources: TaskSourcesConfig,

    #[serde(default)]
    pub channels_config: ChannelsConfig,

    #[serde(default)]
    pub memory: MemoryConfig,

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

    /// Loadable native modules — whether they load, whether this host may fetch
    /// them, and where a developer's own build lives. The loadable *set* is
    /// compiled in, not configured: see `crate::modules::registry`.
    #[serde(default)]
    pub modules: ModulesConfig,

    /// Trust metadata for external capability providers. Empty by default so
    /// existing installations keep the same tool-discovery behavior.
    #[serde(default)]
    pub capability_providers: Vec<CapabilityProviderConfig>,

    #[serde(default)]
    pub multimodal: MultimodalConfig,

    #[serde(default)]
    pub multimodal_files: MultimodalFileConfig,

    #[serde(default)]
    pub seltz: SeltzConfig,

    #[serde(default)]
    pub searxng: SearxngConfig,

    #[serde(default)]
    pub web_search: WebSearchConfig,

    /// Unified search-engine selector. Picks exactly one engine
    /// (managed / parallel / brave) and layers the corresponding tools.
    #[serde(default)]
    pub search: SearchConfig,

    #[serde(default)]
    pub proxy: ProxyConfig,

    #[serde(default)]
    pub cost: CostConfig,

    /// Legacy v1 `[[memory_sources]]` entries, read only so they can be
    /// migrated into `[[memory.sources]]` on load
    /// (`config::ops::loader::normalize_loaded_config`). Each entry is kept as
    /// raw JSON, so a kind this build no longer knows (`twitter_query`) never
    /// fails the parse. Never written back.
    #[serde(default, rename = "memory_sources", skip_serializing)]
    pub legacy_memory_sources: Vec<serde_json::Value>,

    /// User-facing agent registry — shipped default agents plus user-authored
    /// custom agents and persisted enable/disable/tool-policy overrides.
    #[serde(default)]
    pub agent_registry: crate::agent::registry::types::AgentRegistryConfig,

    #[serde(default)]
    pub agents: HashMap<String, DelegateAgentConfig>,

    #[serde(default)]
    pub local_ai: LocalAiConfig,

    /// Claude Agent SDK provider configuration — routes inference through the
    /// `claude -p` CLI subprocess using the subscriber's Claude plan credit.
    #[serde(default)]
    pub claude_agent_sdk: ClaudeAgentSdkConfig,

    // ── Unified AI provider routing ──────────────────────────────────────────
    //
    // Provider-string grammar (consumed by `providers::factory`):
    //
    //   "cloud"                → resolves to `primary_cloud`; if primary is
    //                            openhuman, behaves identically to "openhuman"
    //   "openhuman"            → OpenHuman backend (api_url + api_key session JWT)
    //   "openai:<model>"       → look up cloud_providers entry of type=openai;
    //                            build crate OpenAiModel with Bearer auth
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
    pub cloud_providers: Vec<crate::config::schema::cloud_providers::CloudProviderCreds>,

    /// Id of the `cloud_providers` entry that "cloud" and "primary" resolve to.
    /// When `None`, the factory falls back to the OpenHuman entry.
    #[serde(default)]
    pub primary_cloud: Option<String>,

    /// Runtime only — where this one call's inference goes, when the caller
    /// named an endpoint and bearer for it.
    ///
    /// `#[serde(skip)]` is the whole point: a per-call route must not be able to
    /// reach `config.toml` and repoint the account's inference for good. See
    /// [`ephemeral_route`](crate::config::schema::ephemeral_route)
    /// for how it is installed and which roles it governs.
    #[serde(skip)]
    #[schemars(skip)]
    pub ephemeral_route: Option<crate::config::schema::ephemeral_route::EphemeralRoute>,

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

    /// Provider string for the multimodal / image-understanding workload
    /// (the vision sub-agent). Managed default resolves to `hint:vision`.
    #[serde(default)]
    pub vision_provider: Option<String>,

    /// Provider string for the summarisation workload.
    #[serde(default)]
    pub memory_provider: Option<String>,

    /// Provider string for embedding generation.
    #[serde(default)]
    pub embeddings_provider: Option<String>,

    /// Retained Custom profile; not the active-provider selector. See
    /// [`CustomEmbeddingsConfig`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_embeddings: Option<CustomEmbeddingsConfig>,

    /// Node.js managed runtime configuration (skills that need `node`/`npm`).
    #[serde(default)]
    pub node: NodeConfig,

    /// Python managed runtime configuration (Python-backed MCP servers and
    /// other Python subprocess integrations).
    #[serde(default)]
    pub runtime_python: RuntimePythonConfig,

    /// Shared language-runtime pool (long-lived `node`/`python` workers reused
    /// across skill runs and `node_exec` instead of one child per run, #5106).
    #[serde(default)]
    pub runtime_pool: RuntimePoolConfig,

    /// TokenJuice content-router / compaction configuration.
    #[serde(default)]
    pub tokenjuice: TokenjuiceConfig,

    /// Hosting provider credentials and switch (`hosting` feature).
    #[serde(default)]
    pub hosting: HostingConfig,

    #[serde(default)]
    pub voice_server: VoiceServerConfig,

    // ── Voice provider routing ──────────────────────────────────────────────
    //
    // Mirrors the LLM `cloud_providers` + per-workload routing pattern.
    //
    // Provider-string grammar (consumed by `voice::factory`):
    //
    //   "cloud" / "openhuman"  → OpenHuman backend proxy (STT or TTS)
    //   "piper"                → local Piper (TTS only)
    //   "<slug>:<model>"       → voice_providers entry matched by slug
    //
    // When `stt_provider` / `tts_provider` are `None`, the factory falls
    // back to `local_ai.stt_provider` / `local_ai.tts_provider` (legacy).
    // For STT the final fallback is `voice_server.stt_engine`; for TTS it is
    // `"cloud"`.
    /// Registered voice providers (STT/TTS). Analogous to `cloud_providers`
    /// for LLM inference.
    #[serde(default)]
    pub voice_providers: Vec<crate::config::schema::voice_providers::VoiceProviderCreds>,

    /// STT routing string. Grammar: `"cloud"` | `"<slug>:<model>"`.
    /// `"cloud"` (or unset) defers to `voice_server.stt_engine`.
    #[serde(default)]
    pub stt_provider: Option<String>,

    /// TTS routing string. Grammar: `"cloud"` | `"piper"` | `"<slug>:<voice>"`.
    #[serde(default)]
    pub tts_provider: Option<String>,

    #[serde(default)]
    pub integrations: IntegrationsConfig,

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
    #[serde(default)]
    pub onboarding_completed: bool,

    /// Deprecated — retained for backward-compatible deserialization of
    /// existing `config.toml` files. The welcome agent and its chat-based
    /// onboarding flow have been removed; all chat turns now route directly
    /// to the orchestrator regardless of this flag's value.
    #[serde(default)]
    pub chat_onboarding_completed: bool,

    #[serde(default)]
    pub model_registry: Vec<ModelRegistryEntry>,
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
pub(super) fn default_temperature_unsupported_models() -> Vec<String> {
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
