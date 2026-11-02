//! Agent and delegate agent configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Optional model pin for the front-line orchestrator.
///
/// This is intentionally a small exact-model override: provider routing
/// still comes from the normal reasoning workload, and this field only
/// replaces the final model id when present.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OrchestratorModelConfig {
    pub model: Option<String>,
}

/// Optional per-team model pins used by delegation.
///
/// `lead_model` applies to agents that themselves expose sub-agents;
/// `agent_model` applies to leaf workers. Callers fall back across the
/// pair so configs can specify only one tier without breaking routing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TeamModelConfig {
    pub lead_model: Option<String>,
    pub agent_model: Option<String>,
}

impl TeamModelConfig {
    pub fn model_for_role(&self, is_team_lead: bool) -> Option<&str> {
        let lead_model = self
            .lead_model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty());
        let agent_model = self
            .agent_model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty());

        if is_team_lead {
            lead_model.or(agent_model)
        } else {
            agent_model.or(lead_model)
        }
    }
}

/// A per-agent contract requiring a structured JSON block in every turn's final
/// reply (issue #4117).
///
/// Some agents are consumed by downstream parsing/routing that expects a
/// mandated JSON block — e.g. a `thoughts` block like
/// `{"thoughts": "…", "next_action": "…"}` — on **every** turn. Models
/// frequently omit it, leaving those consumers with nothing. When this contract
/// is set on [`AgentConfig::required_output`], the turn engine validates the
/// reply and repairs an omitted block before the turn is accepted (see
/// `tinyagents_harness::config`), so consumers always get
/// a well-formed block.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RequiredOutputContract {
    /// The JSON key that identifies the required block (e.g. `"thoughts"`). The
    /// contract is satisfied when the reply contains a JSON object carrying this
    /// key — plus every key in [`required_keys`](Self::required_keys) — with a
    /// non-null value. A blank `block_key` makes the contract inert (enforcement
    /// is skipped), so it is a safe no-op default.
    pub block_key: String,
    /// Additional sibling keys that must also be present and non-null in the
    /// same block (e.g. `["next_action"]`). The `block_key` is always required
    /// and need not be repeated here.
    pub required_keys: Vec<String>,
}

impl RequiredOutputContract {
    /// Construct a contract from a block key with no extra required siblings.
    pub fn new(block_key: impl Into<String>) -> Self {
        Self {
            block_key: block_key.into(),
            required_keys: Vec::new(),
        }
    }

    /// Every key that must be present — the block key followed by any declared
    /// siblings — order-preserving, trimmed, and de-duplicated. Empty when the
    /// contract carries no non-blank keys, in which case it is inert and the
    /// turn engine skips enforcement.
    pub fn all_keys(&self) -> Vec<String> {
        // The block key is the contract's defining key — a blank one makes the
        // whole contract inert, even if `required_keys` lists siblings, so the
        // feature never accepts or synthesizes a block missing that key.
        let block_key = self.block_key.trim();
        if block_key.is_empty() {
            return Vec::new();
        }

        let mut keys: Vec<String> = vec![block_key.to_string()];
        for key in &self.required_keys {
            let trimmed = key.trim();
            if !trimmed.is_empty() && !keys.iter().any(|k| k == trimmed) {
                keys.push(trimmed.to_string());
            }
        }
        keys
    }

    /// Whether this contract actually constrains output. A contract with no
    /// non-blank keys is inert and enforcement is skipped.
    pub fn is_active(&self) -> bool {
        !self.all_keys().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AgentConfig {
    /// When true: bootstrap_max_chars=6000, rag_chunk_limit=2. Use for 13B or smaller models.
    #[serde(default)]
    pub compact_context: bool,
    /// Tool-iteration cap for a turn whose agent has **no** definition of its
    /// own. A named definition's `effective_max_iterations()` replaces it (see
    /// `session_host::builder::iteration_cap`), so raising this does not lift
    /// the orchestrator or any other defined agent; use
    /// [`max_tool_iterations_override`](Self::max_tool_iterations_override)
    /// for that.
    #[serde(default = "default_agent_max_tool_iterations")]
    pub max_tool_iterations: usize,
    /// Explicit operator cap that wins over every agent definition's cap,
    /// raising or lowering it, for the agent a turn runs as (sub-agents it
    /// spawns keep their own definition caps). `None`, the default, leaves the
    /// definition in charge. Also set per launch by
    /// `OPENHUMAN_AGENT_MAX_TOOL_ITERATIONS`. Added for #6958, where the only
    /// way to give a coding turn a larger budget was patching `agent.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_iterations_override: Option<usize>,
    /// Agent the web-chat path (`channel_web_chat`, what the desktop composer
    /// calls) routes a turn to. `None` — the default — means `orchestrator`,
    /// which is what the shipped app runs.
    ///
    /// This is the only way to move that path off the orchestrator. A named
    /// definition's `effective_max_iterations()` replaces
    /// `max_tool_iterations` at the single resolution point in
    /// `session_host::builder::iteration_cap`, so pick the agent here for its
    /// prompt and tools; to change only the budget, set
    /// `max_tool_iterations_override` instead. The RPC path already takes an
    /// `agent_id` per call; web chat carries no such field, and adding one to
    /// that wire contract to satisfy an operator preference would be the wrong
    /// seam.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_agent_id: Option<String>,
    /// Deprecated and ignored: the session driver no longer trims history by
    /// message count. Context size is bounded by the token-aware context
    /// ladder (microcompaction, message trimming and summarisation, sized to
    /// the model's window), which cuts rarely. A count cut dropped the oldest
    /// messages on every turn past the bound, which moved the head of the
    /// provider's cached prompt prefix and made each such turn a full cache
    /// miss. Still parsed so existing `config.toml` files keep loading.
    #[serde(default = "default_agent_max_history_messages")]
    pub max_history_messages: usize,
    #[serde(default)]
    pub parallel_tools: bool,
    /// Maximum number of tool calls to execute concurrently when `parallel_tools` is true.
    #[serde(default = "default_max_parallel_tools")]
    pub max_parallel_tools: usize,
    /// How the agent formats tool calls to its provider.
    /// - `"auto"` (default): native structured tool-calling when the provider
    ///   supports it, otherwise JSON-in-tag (`<tool_call>{…}</tool_call>`).
    /// - `"python"`: code-style calls against Python signatures in the prompt
    ///   (`def read_file(path: str, limit: int = None) -> str`, called as
    ///   `read_file(path="x")`). Opt-in; mis-parses on some models.
    /// - `"native"`: force provider-native structured tool calls.
    /// - `"xml"`: force JSON-in-tag.
    /// - `"pformat"`: force compact positional P-Format (`tool[a|b]`); it
    ///   mis-parses on some models.
    /// - `"typescript"`: like `"python"` with TypeScript signatures and
    ///   `read_file({path: "x"})` calls.
    ///
    /// The `OPENHUMAN_TOOL_DISPATCHER` environment variable overrides this
    /// field for one launch.
    #[serde(default = "default_agent_tool_dispatcher")]
    pub tool_dispatcher: String,
    /// Per-channel maximum permission level for tool execution.
    /// Keys are channel names (e.g., "telegram", "discord", "web", "cli").
    /// Values are permission levels: "none", "readonly" (or "read_only"),
    /// "write", "execute", "dangerous".
    ///
    /// Runtime semantics (see
    /// [`tinytools::engine::ToolPolicyEngine`]):
    ///
    /// * **Empty map** — the policy engine preserves the legacy
    ///   unrestricted surface and returns `PermissionLevel::Dangerous`
    ///   for every channel. This branch only matters before the
    ///   one-time migration runs.
    /// * **Non-empty map, channel present** — the configured level is
    ///   used.
    /// * **Non-empty map, channel absent** — the engine falls back to
    ///   `PermissionLevel::ReadOnly` (the fail-closed default for an
    ///   already-policy-managed install).
    ///
    /// [`AgentConfig::migrate_channel_permissions_if_legacy`] seeds the
    /// map with `web=Execute` + each configured channel = `Execute` on
    /// first boot after upgrade, so legacy installs land in the
    /// non-empty branch before any tool dispatch happens. New installs
    /// ship with an explicit map. The empty-map "Dangerous" branch is
    /// effectively reachable only by an operator manually wiping the
    /// map in their on-disk config; if you change that branch's
    /// behavior, update `AGENTS.md` and the engine docstring in lock-step.
    #[serde(default)]
    pub channel_permissions: std::collections::HashMap<String, String>,

    /// Channel name → id of the agent that answers that channel.
    ///
    /// An inbound message on a channel named here (`telegram = "teeny-chat"`)
    /// runs as that agent instead of the orchestrator: the agent is looked up
    /// through the host agent resolver
    /// ([`crate::agent::host_agents`]), so an embedder's own agent answers
    /// with its own system prompt and host tools. A channel that is not named
    /// here is routed exactly as before. Keyed by channel name rather than set
    /// on each provider's config because those provider structs belong to
    /// `tinychannels`; routing a channel to an agent is host policy.
    ///
    /// A binding the resolver cannot satisfy is refused, never widened to
    /// the orchestrator; see `channels::runtime::dispatch::host_agent`.
    #[serde(default)]
    pub channel_agents: std::collections::HashMap<String, String>,

    /// Maximum byte length of a single tool-result body before the
    /// TinyAgents tool-output middleware budget stage truncates it. Applied
    /// inline at tool-execution time (before the result enters history),
    /// so it is cache-safe. `0` disables the cap. Defaults to 16 KiB.
    #[serde(default = "default_tool_result_budget_bytes")]
    pub tool_result_budget_bytes: usize,

    /// Wall-clock timeout, in seconds, for a single tool/action execution
    /// (and the per-agent delegated chat call). Bounded to
    /// `tool_timeout::MIN_TIMEOUT_SECS..=tool_timeout::MAX_TIMEOUT_SECS`
    /// (`1..=3600`); the default is `tool_timeout::DEFAULT_TIMEOUT_SECS`
    /// (120). Surfaced in **Settings → Agent OS access → Action timeout** so
    /// users running large local models can extend it without editing config
    /// files (issue #3100). Pushed into the live
    /// [`crate::tools::timeout`] runtime on save; the
    /// `OPENHUMAN_TOOL_TIMEOUT_SECS` env var still overrides it when set.
    #[serde(default = "default_agent_timeout_secs")]
    pub agent_timeout_secs: u64,

    /// **Retired, no effect.** Once mirrored each completed turn from the
    /// `session_raw/*.jsonl` transcript into a second, file-shaped layout
    /// (`{workspace}/tinyagents_store/{kv,journal}`). Sessions now live in the
    /// storage backend and nothing is mirrored; the field is kept so existing
    /// `config.toml` files still load.
    #[serde(default = "default_session_dual_write")]
    pub session_dual_write: bool,

    /// **Retired, no effect.** Once compared a resumed session's transcript
    /// with its mirror in the file-shaped store and logged any divergence
    /// (`[session_shadow_read]`). The mirror is gone with the dual-write; the
    /// field is kept so existing `config.toml` files still load.
    #[serde(default = "default_session_shadow_reads")]
    pub session_shadow_reads: bool,

    /// Optional required structured-output contract. When set to an active
    /// contract, every turn's final reply must contain the mandated JSON block;
    /// the turn engine validates and repairs an omitted block before the turn is
    /// accepted (issue #4117). `None` (the default) disables enforcement so
    /// existing agents are unaffected.
    #[serde(default)]
    pub required_output: Option<RequiredOutputContract>,

    /// Whether to load `AGENTS.md` instruction files into the agent's system
    /// prompt — OpenHuman's analog of Claude Code's `CLAUDE.md` / Codex's
    /// `AGENTS.md`. When `true` (the default), the harness reads
    /// `<workspace_dir>/AGENTS.md` (global) and `<action_dir>/AGENTS.md`
    /// (project) once at system-prompt build time and injects them as
    /// `## Project instructions (AGENTS.md)`. When `false`, no AGENTS.md content
    /// is loaded or injected. Missing/empty files are always a silent no-op, so
    /// leaving this on has zero effect until the user actually creates an
    /// `AGENTS.md`.
    #[serde(default = "default_agents_md_enabled")]
    pub agents_md_enabled: bool,

    /// How the harness's `tool_search` bridge ranks deferred tools against
    /// the model's query. See [`ToolSearchConfig`].
    #[serde(default)]
    pub tool_search: ToolSearchConfig,
}

/// Configuration of the `tool_search` bridge: which ranker answers a search
/// over the tools that left the wire under `ToolExposure::Deferred`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ToolSearchConfig {
    /// Which ranker serves the search.
    ///
    /// - `"jev"` (default): the installed decision-model ranker (Jev, via
    ///   `openhuman-tinyhumans`), falling back to BM25 only when it fails or
    ///   the process has no credential for the configured Jev route
    ///   ([`ToolSearchConfig::jev_route`]).
    /// - `"auto"`: the installed ranker when the process has one and a
    ///   credential for the Jev route; BM25 otherwise.
    /// - `"bm25"`: the built-in lexical ranker alone, no network.
    /// - `"compare"`: serve the installed ranker and record the BM25 ranking
    ///   alongside it in the `tool.searched` telemetry, so the two can be
    ///   judged on live traffic without changing what the model sees.
    pub ranker: String,
    /// Where the Jev ranker's decision calls go and what authenticates them.
    /// Jev is reachable three ways, and which one a process uses is the
    /// operator's call, not something to infer from whatever credential
    /// happens to be stored:
    ///
    /// - `"auto"` (default): the TinyHumans credential when the process has
    ///   one, else `TYPESAFE_API_KEY` (direct), else an OpenRouter key
    ///   (`OPENROUTER_API_KEY`, or the stored `openrouter` BYOK key).
    /// - `"tinyhumans"`: the TinyHumans backend's proxy, TinyHumans credential only.
    /// - `"typesafe"`: TypeSafe's own API, `TYPESAFE_API_KEY` only.
    /// - `"openrouter"`: OpenRouter's System One API, OpenRouter key only.
    ///
    /// `OPENHUMAN_JEV_ROUTE` overrides this for one launch.
    pub jev_route: String,
    /// Replaces the API origin of the `"typesafe"` / `"openrouter"` routes
    /// (a metering proxy, a regional mirror, a test double). Remote origins
    /// must be HTTPS; plain HTTP is accepted for literal loopback IPs only,
    /// which the Jev client enforces. `OPENHUMAN_JEV_BASE_URL` overrides this
    /// for one launch. Has no effect on the `"tinyhumans"` route.
    pub jev_base_url: Option<String>,
    /// Matches a search returns when the model does not ask for a number.
    /// Three: enough for the model to choose, few enough that the schemas
    /// returned do not undo the saving deferral made.
    pub top_k: usize,
}

impl Default for ToolSearchConfig {
    fn default() -> Self {
        Self {
            ranker: "jev".into(),
            jev_route: "auto".into(),
            jev_base_url: None,
            top_k: 3,
        }
    }
}

fn default_agents_md_enabled() -> bool {
    true
}

fn default_session_dual_write() -> bool {
    true
}

fn default_session_shadow_reads() -> bool {
    // ON for the Phase 2 parity soak. Observation-only: the legacy read stays
    // authoritative and the probe runs on a background task, so the worst case
    // of a bad soak is log noise, not a broken resume. Disable per-workspace in
    // config, or globally with `OPENHUMAN_SESSION_SHADOW_READS=0`.
    //
    // This default only covers workspaces whose config predates the key.
    // `Config::save` writes every field, so an already-saved workspace carries
    // a literal `session_shadow_reads = false` that serde never overrides —
    // the 8 -> 9 `enable_session_shadow_reads` migration is what opts those in.
    true
}

fn default_tool_result_budget_bytes() -> usize {
    tinyagents_harness::config::TurnConfig::default().tool_result_budget_bytes
}

fn default_agent_timeout_secs() -> u64 {
    tinyagents_harness_tools::exec::timeout::DEFAULT_TIMEOUT_SECS
}

fn default_agent_max_tool_iterations() -> usize {
    10
}

fn default_agent_max_history_messages() -> usize {
    50
}

fn default_max_parallel_tools() -> usize {
    4
}

fn default_agent_tool_dispatcher() -> String {
    "auto".into()
}

impl AgentConfig {
    /// Seed legacy installs whose channel-permissions map is empty and
    /// that already have at least one non-web channel configured,
    /// writing explicit per-channel execute entries.
    ///
    /// The engine layer keeps its legacy empty-map shortcut; this
    /// migration replaces it with an explicit policy so the
    /// per-channel cap engages on the very first boot after upgrade.
    /// `known_channels` is the set of channels the user has configured
    /// in `channels::ChannelsConfig`. The web channel is always added
    /// on top so the desktop UI stays usable.
    ///
    /// Returns `true` when a migration write is required so the caller
    /// can save and reload; returns `false` when the map was already
    /// populated, no non-web channels were configured (fresh install,
    /// engine's legacy unrestricted shortcut continues), or the
    /// migration is otherwise a no-op. Idempotent.
    pub fn migrate_channel_permissions_if_legacy<I, S>(&mut self, known_channels: I) -> bool
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        if !self.channel_permissions.is_empty() {
            return false;
        }
        let extra: Vec<String> = known_channels
            .into_iter()
            .map(|s| s.as_ref().to_string())
            .filter(|s| !s.is_empty() && s != "web")
            .collect();
        if extra.is_empty() {
            // No channels configured yet — leave the map empty so the
            // engine's legacy "empty == unrestricted" shortcut keeps
            // ruling fresh installs the same way it did pre-PR. The
            // migration is for installs that already have channels
            // active under the legacy unrestricted surface.
            return false;
        }
        // Seed web + every known channel = execute so the engine's
        // per-channel cap evaluates against an explicit policy instead
        // of the legacy unrestricted default.
        let names: Vec<String> = std::iter::once("web".to_string()).chain(extra).collect();
        for name in &names {
            self.channel_permissions
                .insert(name.clone(), "execute".to_string());
        }
        log::info!(
            target: "openhuman::config",
            "[agent-config] channel_permissions: migrated {} legacy channels to execute (preserved pre-PR behavior): {:?}",
            names.len(),
            names
        );
        true
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            compact_context: false,
            max_tool_iterations: default_agent_max_tool_iterations(),
            max_tool_iterations_override: None,
            chat_agent_id: None,
            max_history_messages: default_agent_max_history_messages(),
            parallel_tools: false,
            max_parallel_tools: default_max_parallel_tools(),
            tool_dispatcher: default_agent_tool_dispatcher(),
            channel_permissions: std::collections::HashMap::new(),
            channel_agents: std::collections::HashMap::new(),
            tool_result_budget_bytes: default_tool_result_budget_bytes(),
            agent_timeout_secs: default_agent_timeout_secs(),
            session_dual_write: default_session_dual_write(),
            session_shadow_reads: default_session_shadow_reads(),
            required_output: None,
            agents_md_enabled: default_agents_md_enabled(),
            tool_search: ToolSearchConfig::default(),
        }
    }
}
