//! `Agent` and `AgentBuilder` struct definitions.
//!
//! The data shapes live here, separate from their behaviour, so the
//! rest of the sub-module (`builder.rs`, `turn.rs`, `runtime.rs`) can
//! focus on logic. Fields are `pub(super)` so sibling files that
//! `impl Agent`/`impl AgentBuilder` can see them without the whole
//! crate gaining field access.

use crate::openhuman::agent::dispatcher::ToolDispatcher;
use crate::openhuman::agent::harness::archivist::ArchivistHook;
use crate::openhuman::agent::harness::definition::TriggerMemoryAgent;
use crate::openhuman::agent::hooks::PostTurnHook;
use crate::openhuman::agent::memory_loader::MemoryLoader;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::tool_policy::ToolPolicy;
use crate::openhuman::agent_tool_policy::ToolPolicySession;
use crate::openhuman::context::prompt::SystemPromptBuilder;
use crate::openhuman::context::ContextManager;
use crate::openhuman::inference::provider::{ChatMessage, ConversationMessage, Provider};
use crate::openhuman::memory::Memory;
use crate::openhuman::tools::{Tool, ToolSpec};
use std::path::PathBuf;
use std::sync::Arc;

/// An autonomous or semi-autonomous AI agent.
///
/// The `Agent` is the central component that manages conversation state,
/// executes tools based on model requests, and interacts with the memory
/// system to maintain context across turns.
pub struct Agent {
    pub(super) provider: Arc<dyn Provider>,
    /// Full tool registry. Sub-agents pull from this via
    /// [`ParentExecutionContext::all_tools`].
    pub(super) tools: Arc<Vec<Box<dyn Tool>>>,
    /// Full tool specs — sub-agents receive these via
    /// [`ParentExecutionContext::all_tool_specs`].
    pub(super) tool_specs: Arc<Vec<ToolSpec>>,
    /// Tool specs filtered by the visible-tool allowlist and session
    /// permission policy. These are the specs actually sent to the
    /// provider in the main agent's chat requests.
    pub(super) visible_tool_specs: Arc<Vec<ToolSpec>>,
    /// When non-empty, only these tool names are visible in the main
    /// agent's prompt and callable by the main agent. Sub-agents ignore
    /// this filter — they apply per-definition whitelists in the runner.
    /// Empty = no filter (all tools visible, backward compat).
    pub(super) visible_tool_names: std::collections::HashSet<String>,
    pub(super) tool_policy_session: ToolPolicySession,
    pub(super) memory: Arc<dyn Memory>,
    // `Arc` (not `Box`) so the turn engine's parser seam can hold a cheap clone
    // of the dispatcher without borrowing the `Agent` (which the turn observer
    // borrows mutably) — see `engine::DispatcherParser`.
    pub(super) tool_dispatcher: Arc<dyn ToolDispatcher>,
    pub(super) memory_loader: Box<dyn MemoryLoader>,
    pub(super) config: crate::openhuman::config::AgentConfig,
    pub(super) model_name: String,
    /// User-configured vision capability for [`Self::model_name`], evaluated at
    /// session build from `model_vision_enabled(&model, config)`. Surfaced to the
    /// turn engine's image gate via the `current_model_vision` task-local so a
    /// custom/BYOK model the user flagged can forward images. Defaults to `false`.
    pub(super) model_vision: bool,
    pub(super) temperature: f64,
    pub(super) workspace_dir: std::path::PathBuf,
    pub(super) action_dir: std::path::PathBuf,
    pub(super) workflows: Vec<crate::openhuman::workflows::Workflow>,
    /// Agent workflows discovered at session start.
    pub(super) auto_save: bool,
    /// Last memory context loaded for the current turn. Stored so it can
    /// be forwarded to subagents via `ParentExecutionContext`.
    pub(super) last_memory_context: Option<String>,
    /// Citation metadata collected from memory recall for the most recent turn.
    /// Consumed by web-channel delivery to render source chips in the UI.
    pub(super) last_turn_citations: Vec<crate::openhuman::agent::memory_loader::MemoryCitation>,
    /// Holistic token/cost/context accounting for the most recent turn (parent +
    /// any sub-agents spawned during it). Consumed by web-channel delivery to
    /// surface session token/cost/context meters in the UI footer. `None` until
    /// the first turn completes.
    pub(super) last_turn_usage_totals:
        Option<crate::openhuman::agent::harness::turn_subagent_usage::LastTurnUsage>,
    pub(super) history: Vec<ConversationMessage>,
    pub(super) post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    pub(super) learning_enabled: bool,
    /// When `true`, pinned preferences stored via `remember_preference` are
    /// fetched from the `user_profile` namespace and injected into the system
    /// prompt on every turn, independent of `learning_enabled`.
    pub(super) explicit_preferences_enabled: bool,
    pub(super) event_session_id: String,
    pub(super) event_channel: String,
    /// Human-readable agent definition name (e.g. `"main"`,
    /// `"code_executor"`). Used as the `{agent}` component in session
    /// transcript paths: `sessions/DDMMYYYY/{agent}_{index}.md`.
    ///
    /// May be rewritten mid-session by
    /// [`Agent::set_agent_definition_name`] (e.g. the web channel
    /// stamps `"orchestrator_<short_thread>"` so each thread gets its
    /// own transcript namespace). Anything that needs to resolve the
    /// session back to its registry entry must use
    /// [`Self::agent_definition_id`], not this field.
    pub(super) agent_definition_name: String,
    /// Canonical agent id as registered in
    /// [`AgentDefinitionRegistry`] (e.g. `"orchestrator"`,
    /// `"integrations_agent"`). Set once at build time and never
    /// rewritten — `set_agent_definition_name` only touches the
    /// transcript-facing `agent_definition_name`, so registry lookups
    /// (e.g. `refresh_delegation_tools` re-resolving the agent's
    /// `subagents` list post-fetch) stay correct even after the web
    /// channel's per-thread rename.
    ///
    /// [`AgentDefinitionRegistry`]: crate::openhuman::agent::harness::definition::AgentDefinitionRegistry
    pub(super) agent_definition_id: String,
    /// Resolved filesystem path for this session's transcript file.
    /// Set on first write, reused for subsequent overwrites within the
    /// same session.
    pub(super) session_transcript_path: Option<PathBuf>,
    /// Unique transcript key for this session, formatted as
    /// `"{unix_ts}_{agent_id}"`. Generated once at agent-build time so
    /// every transcript write in this session uses the same filename
    /// stem. Sub-agents chain their parent's key into the transcript
    /// directory to produce a hierarchical layout —
    /// `session_raw/DDMMYYYY/{parent_key}/{child_key}.jsonl`.
    pub(super) session_key: String,
    /// Directory chain of parent session keys for a sub-agent, or
    /// `None` for a root session. A planner spawned by the orchestrator
    /// carries `Some("1713000000_orchestrator")`; a critic spawned by
    /// that planner carries
    /// `Some("1713000000_orchestrator/1713000123_planner")` so nested
    /// delegations produce a tree on disk.
    pub(super) session_parent_prefix: Option<String>,
    /// Messages loaded from a previous session transcript on resume.
    /// Consumed once (via `.take()`) on the first turn to provide a
    /// byte-identical prefix for KV cache reuse.
    pub(super) cached_transcript_messages: Option<Vec<ChatMessage>>,
    /// Per-session [`ContextManager`] — owns the system-prompt
    /// builder, the layered reduction pipeline (tool-result budget →
    /// microcompact → autocompact signal → session-memory extraction
    /// trigger), the guard's compaction circuit breaker, and the LLM
    /// summarizer that runs when the pipeline asks for autocompaction.
    /// Constructed once at session start so its budget counters and
    /// session-memory deltas persist across turns. See
    /// [`crate::openhuman::context`] for the full surface.
    pub(super) context: ContextManager,
    /// Optional progress event sender for real-time turn progress.
    /// When set, the turn loop emits [`AgentProgress`] events through
    /// this channel so callers (e.g. web channel) can surface live
    /// tool-call and iteration updates to the UI.
    pub(super) on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    /// Optional active-run queue for mid-turn steering. When set, the
    /// engine drains steers/collects at iteration boundaries.
    pub(super) run_queue: Option<Arc<crate::openhuman::agent::harness::run_queue::RunQueue>>,
    /// Active Composio integrations the user has connected. Populated at
    /// agent build time and threaded into each agent's `prompt.rs` so
    /// the delegator / skill-executor voices can render their own
    /// integration blocks.
    pub(super) connected_integrations: Vec<crate::openhuman::context::prompt::ConnectedIntegration>,
    /// Whether `connected_integrations` is an authoritative session-start
    /// snapshot (prewarmed from the shared Composio cache or fetched
    /// explicitly) versus the default empty placeholder installed by
    /// `AgentBuilder::build`. Turn 1 uses this to decide whether it must
    /// still pay the cold-start fetch cost before freezing the system prompt.
    pub(super) connected_integrations_initialized: bool,
    /// Full runtime config snapshot for integration-cache reads and the
    /// best-effort fallback fetch path. Session agents built from
    /// `Config` carry this directly so the turn loop does not need to
    /// re-run `Config::load_or_init()` on the hot path just to key into
    /// the Composio cache.
    pub(super) integration_runtime_config: Option<crate::openhuman::config::Config>,
    /// Mirrors the agent definition's `omit_profile` flag. Threaded into
    /// [`PromptContext::include_profile`] in `turn::build_system_prompt`
    /// so only user-facing agents (welcome, orchestrator, triggers)
    /// inject `PROFILE.md`. Defaults to `true` (omit) for custom / legacy
    /// agents built without a definition.
    pub(super) omit_profile: bool,
    /// Mirrors the agent definition's `omit_memory_md` flag. Forwarded to
    /// [`PromptContext::include_memory_md`] at prompt-build time. Same
    /// session-freeze contract as `omit_profile`.
    pub(super) omit_memory_md: bool,
    /// Optional payload-summarizer wired in at agent-build time.
    /// Currently set only for the orchestrator session
    /// (see [`super::builder`]). When `Some`, oversized tool results
    /// produced by [`Agent::execute_tool_call`] are routed through the
    /// summarizer sub-agent before they enter agent history.
    pub(super) payload_summarizer:
        Option<Arc<dyn crate::openhuman::agent::harness::payload_summarizer::PayloadSummarizer>>,
    /// Mirrors the agent definition's `trigger_memory_agent` policy.
    /// `Always` runs the dedicated memory retrieval agent once before
    /// the user's prompt is sent to this agent.
    pub(super) trigger_memory_agent: TriggerMemoryAgent,
    /// Per-agent TokenJuice profile for tool results entering this session's
    /// model context.
    pub(super) tokenjuice_compression: crate::openhuman::tokenjuice::AgentTokenjuiceCompression,
    /// Pre-execution policy hook for tool calls in this session. The
    /// default policy allows all calls so existing agents keep their
    /// behaviour unless a caller opts into stricter policy.
    pub(super) tool_policy: Arc<dyn ToolPolicy>,
    /// Hash of the Composio connection set this Agent last reconciled
    /// against. Compared at top-of-turn to a fresh hash computed from
    /// [`crate::openhuman::composio::cached_active_integrations`]; on
    /// diff, [`Agent::refresh_delegation_tools`] re-synthesises the
    /// `delegate_<toolkit>` surface to match the live connected set.
    ///
    /// Initialised to `0` at construction. Turn 1's existing refresh
    /// path (gated by `history.is_empty()`) writes the first real hash
    /// after [`Agent::fetch_connected_integrations`] populates
    /// [`Agent::connected_integrations`], so the per-turn check is
    /// dormant on session startup and only fires when integrations
    /// actually change mid-conversation.
    pub(super) last_seen_integrations_hash: u64,
    /// Per-session raw receiver for `DomainEvent::ComposioIntegrationsChanged`.
    /// Armed lazily on first turn when the global event bus is available.
    /// Drained before each provider dispatch so a connection that flips to
    /// ACTIVE mid-turn can refresh the delegation schema in the same thread.
    pub(super) composio_integrations_rx:
        Option<tokio::sync::broadcast::Receiver<crate::core::event_bus::DomainEvent>>,
    /// Lazily-armed global-bus receiver for [`DomainEvent::WorkflowsChanged`]
    /// (skill install / uninstall / create). Drained at each turn boundary so
    /// `refresh_workflows` only re-scans disk when the installed set actually
    /// changed — no per-turn filesystem walk on the steady-state hot path.
    pub(super) skill_events_rx:
        Option<tokio::sync::broadcast::Receiver<crate::core::event_bus::DomainEvent>>,
    /// Toolkit slugs already surfaced to the model as freshly-connected
    /// this session. Seeded at turn 1 with the startup connected set, then
    /// extended whenever a mid-session connect is announced — so each new
    /// toolkit is announced exactly once, never re-announced per turn.
    pub(super) announced_integrations: std::collections::HashSet<String>,
    /// Toolkit slugs that connected mid-session and still need announcing on
    /// the next user message ("X connected this session, use it now"). Parked
    /// by `refresh_delegation_tools_from_cached_integrations` and rendered +
    /// cleared when the next user message is built — the note rides on the
    /// user turn (NOT the system prompt) so the KV-cache prefix stays
    /// byte-identical.
    ///
    /// Accumulated as a list (not a single rendered string) so two connects
    /// between consecutive user turns both surface: a second connect appends
    /// its slug instead of overwriting the first's note. Order-preserving +
    /// de-duped on insert.
    pub(super) pending_integration_announcement: Vec<String>,
    /// MCP server qualified-names already surfaced to the model as
    /// freshly-connected this session. The MCP analogue of
    /// [`Self::announced_integrations`]: seeded at turn 1 with the startup
    /// connected set, extended as mid-session connects are announced, so each
    /// server is announced exactly once (never re-announced per turn).
    pub(super) announced_mcp_servers: std::collections::HashSet<String>,
    /// MCP servers that connected mid-session and still need announcing on the
    /// next user message. The MCP analogue of
    /// [`Self::pending_integration_announcement`]. `use_mcp_server` is a single
    /// static delegate (no per-server schema to refresh), so this prose note on
    /// the user turn is the entire mid-session-connect mechanism for MCP. The
    /// note rides the user turn (NOT the system prompt) so the KV-cache prefix
    /// stays byte-identical. Order-preserving + de-duped on insert.
    pub(super) pending_mcp_announcement: Vec<String>,
    /// Skill ids discovered mid-session (installed after session build) that
    /// still need announcing on the next user message. Mirrors
    /// [`Self::pending_integration_announcement`] for the `## Installed Skills`
    /// catalogue: parked by `refresh_workflows`, rendered + cleared when the
    /// next user message is built so the note rides the user turn (NOT the
    /// system prompt) and the KV-cache prefix stays byte-identical.
    pub(super) pending_skill_announcement: Vec<String>,
    /// Skill ids removed mid-session (uninstalled after session build) that
    /// still need retracting on the next user message. Symmetric to
    /// [`Self::pending_skill_announcement`]: parked by `refresh_workflows`,
    /// rendered + cleared when the next user message is built so the retraction
    /// note rides the user turn (NOT the system prompt) and the KV-cache prefix
    /// stays byte-identical.
    pub(super) pending_skill_retraction: Vec<String>,
    /// Skill ids already surfaced to the model as installed this session, so
    /// each newly-installed skill is announced exactly once and never
    /// re-announced per turn. Seeded from the session-build catalogue.
    pub(super) announced_skills: std::collections::HashSet<String>,
    /// Optional reference to the `ArchivistHook` registered in
    /// `post_turn_hooks`. Kept separately so the turn loop can call
    /// `flush_open_segment` at session-memory-extraction time (the
    /// closest available signal to "session is ending") to finalize the
    /// trailing open segment with an LLM recap + embedding.
    pub(super) archivist_hook: Option<Arc<ArchivistHook>>,
    /// Names of every tool currently in [`Agent::tools`] that was
    /// produced by [`crate::openhuman::tools::orchestrator_tools::collect_orchestrator_tools`]
    /// (i.e. `delegate_<toolkit>` skill tools and archetype-delegation
    /// tools like `delegate_archivist`). Tracked so
    /// [`Agent::refresh_delegation_tools`] can drop the entire
    /// previously-synthesised subset on each refresh and append the
    /// fresh set — without that mask we'd risk either leaking stale
    /// `delegate_<toolkit>` entries on revoke or accidentally removing
    /// direct tools (`query_memory`, `cron_add`, …) that share a name
    /// prefix.
    ///
    /// Populated by `refresh_delegation_tools` itself; empty at
    /// construction time.
    ///
    /// Invariant: this tracks the names whose **`tool_specs`** are currently
    /// live. `tool_specs` reconcile on every refresh (they're cloneable
    /// data), so this set always equals the most recent synthesised set —
    /// even when the executable `tools` Vec could not be reconciled because
    /// its `Arc` was shared. Removing stale `tools` entries is tracked
    /// separately by [`Self::pending_synthesized_tools_mask`].
    pub(super) synthesized_tool_names: std::collections::HashSet<String>,
    /// Names of synthesised tool *instances* still present in [`Agent::tools`]
    /// that a future unique-owner refresh must drop.
    ///
    /// When `refresh_delegation_tools` updates `tool_specs` but cannot
    /// reconcile `tools` (the `Arc` is shared — the normal case while
    /// `AgentToolSource` holds a clone during `before_dispatch`), the
    /// previously-synthesised tool objects remain in `tools`. Their names are
    /// accumulated here so the next refresh that *does* own `tools` uniquely
    /// removes them — instead of overloading `synthesized_tool_names` (which
    /// must stay in sync with `tool_specs`) and corrupting the spec
    /// reconciliation on the following refresh (duplicate `ToolSpec`s, #3044).
    ///
    /// Empty at construction time and whenever `tools` is fully reconciled.
    pub(super) pending_synthesized_tools_mask: std::collections::HashSet<String>,
}

/// A builder for creating `Agent` instances with custom configuration.
pub struct AgentBuilder {
    pub(super) provider: Option<Arc<dyn Provider>>,
    pub(super) tools: Option<Vec<Box<dyn Tool>>>,
    /// When set, restricts which tools the main agent sees/calls.
    pub(super) visible_tool_names: Option<std::collections::HashSet<String>>,
    pub(super) memory: Option<Arc<dyn Memory>>,
    pub(super) prompt_builder: Option<SystemPromptBuilder>,
    pub(super) tool_dispatcher: Option<Box<dyn ToolDispatcher>>,
    pub(super) memory_loader: Option<Box<dyn MemoryLoader>>,
    pub(super) config: Option<crate::openhuman::config::AgentConfig>,
    /// Optional [`ContextConfig`] override threaded through from
    /// `Agent::from_config`. When unset the builder falls back to
    /// [`crate::openhuman::config::ContextConfig::default`].
    pub(super) context_config: Option<crate::openhuman::config::ContextConfig>,
    pub(super) model_name: Option<String>,
    /// User vision flag for the resolved model; `None` → `false` in `build()`.
    pub(super) model_vision: Option<bool>,
    pub(super) temperature: Option<f64>,
    pub(super) workspace_dir: Option<std::path::PathBuf>,
    pub(super) action_dir: Option<std::path::PathBuf>,
    pub(super) workflows: Option<Vec<crate::openhuman::workflows::Workflow>>,
    /// Agent workflows to surface in the prompt. Populated from `load_workflows`
    /// at session start; defaults to empty when not explicitly set.
    pub(super) auto_save: Option<bool>,
    pub(super) post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    pub(super) learning_enabled: bool,
    pub(super) explicit_preferences_enabled: bool,
    pub(super) event_session_id: Option<String>,
    pub(super) event_channel: Option<String>,
    pub(super) agent_definition_name: Option<String>,
    /// Directory chain of parent session keys for a sub-agent. `None`
    /// (default) means this is a root session — its transcript lands
    /// flat in `session_raw/DDMMYYYY/{session_key}.jsonl`. Populated
    /// by the sub-agent runner so nested delegations produce a tree.
    pub(super) session_parent_prefix: Option<String>,
    /// Forwarded to [`Agent::omit_profile`] at `build()` time. Mirrors the
    /// target definition's `omit_profile` flag; `None` means "fall back
    /// to the safe default" (omit).
    pub(super) omit_profile: Option<bool>,
    /// Forwarded to [`Agent::omit_memory_md`]. Same shape as
    /// `omit_profile` — `None` falls back to the "omit" default.
    pub(super) omit_memory_md: Option<bool>,
    /// Optional payload-summarizer threaded through to [`Agent`] at
    /// build time. Defaults to `None`; the orchestrator branch in
    /// [`super::builder::Agent::build_session_agent_inner`] sets this
    /// to a `SubagentPayloadSummarizer` instance.
    pub(super) payload_summarizer:
        Option<Arc<dyn crate::openhuman::agent::harness::payload_summarizer::PayloadSummarizer>>,
    /// Forwarded to [`Agent::trigger_memory_agent`] at build time.
    pub(super) trigger_memory_agent: Option<TriggerMemoryAgent>,
    /// Per-agent TokenJuice tool-output compression profile.
    pub(super) tokenjuice_compression: crate::openhuman::tokenjuice::AgentTokenjuiceCompression,
    /// Optional pre-execution tool policy. Defaults to allow-all.
    pub(super) tool_policy: Option<Arc<dyn ToolPolicy>>,
    /// Optional reference to the production `ArchivistHook`. Set when
    /// `config.learning.episodic_capture_enabled` is true. Used to call
    /// `flush_open_segment` at the closest available session-end signal.
    pub(super) archivist_hook: Option<Arc<ArchivistHook>>,
    /// Phase 1.5 — when `true` AND `archivist_hook` is `Some`, the
    /// `ContextManager`'s summarizer is wrapped with a
    /// `SegmentRecapSummarizer` that routes compaction through the
    /// archivist's rolling segment recap (one summarizer, soft-fallback).
    /// When `false` (or archivist absent), the plain `ProviderSummarizer`
    /// is used and Phase 1.5 is completely absent from the hot path.
    /// Default: `true` (mirrors `LearningConfig::unified_compaction_enabled`).
    pub(super) unified_compaction_enabled: bool,
}

impl Default for AgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_builder_default_matches_new() {
        let builder = AgentBuilder::new();
        let default_builder = AgentBuilder::default();

        assert_eq!(builder.learning_enabled, default_builder.learning_enabled);
        assert_eq!(builder.auto_save, default_builder.auto_save);
        assert!(builder.provider.is_none());
        assert!(builder.tools.is_none());
        assert!(builder.memory.is_none());
        assert!(builder.event_session_id.is_none());
        assert!(builder.event_channel.is_none());
        assert!(builder.agent_definition_name.is_none());
        assert!(builder.post_turn_hooks.is_empty());
    }
}
