//! `OpenHumanSessionHost` and `SessionHostBuilder` struct definitions.
//!
//! The data shapes live here, separate from their behaviour, so the
//! rest of the sub-module (`builder.rs`, `turn.rs`, `runtime.rs`) can
//! focus on logic. Fields are `pub(super)` so sibling files that
//! `impl OpenHumanSessionHost`/`impl SessionHostBuilder` can see them without the whole
//! crate gaining field access.

use crate::agent::context::ContextManager;
use crate::agent::hooks::PostTurnHook;
use crate::agent::progress::AgentProgress;
use crate::agent::prompts::SystemPromptBuilder;
use crate::agent::tinyagents::TurnModelSource;
use crate::agent::tool_policy::ToolPolicy;
use crate::tools::agent_policy::ToolPolicySession;
use std::sync::Arc;
use tinytools::{Tool, ToolSpec};
use tinytools_agent::dialect::ToolDialect;

/// Per-turn behaviour overrides applied to a **single** [`OpenHumanSessionHost::turn`] call.
///
/// Defaults to all-`false`, so an agent built and driven exactly as before
/// behaves identically — the overrides only take effect when a caller opts in
/// via [`OpenHumanSessionHost::set_next_turn_overrides`] before dispatching a turn. The turn
/// consumes (takes) them at its start, so they apply to exactly one turn and
/// then reset to the default; a caller that wants a run of chat turns re-sets
/// them each time.
///
/// The motivating case (opencompany issue #1725) is a bare greeting / small-talk
/// turn that should run as a cheap conversational reply instead of the full
/// agentic task loop: no tools to loop on and no stale per-thread goal re-injected from a prior task. Each field is an
/// independent, additive suppression so a caller can compose exactly the
/// reduction it wants.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TurnOverrides {
    /// Skip loading, auto-resuming, and injecting this thread's durable
    /// `[active_goal]` block for this turn (and skip arming the goal budget
    /// stop hook). Prevents an uncompleted goal left by a prior task from
    /// steering an unrelated chat turn.
    pub suppress_active_goal: bool,
    /// Run this turn with **no** tools regardless of the agent's built tool
    /// set — the provider request carries an empty tool schema, so the model
    /// cannot enter the tool loop and answers in one shot. The agent's durable
    /// `tools` / `tool_specs` are left untouched, so the next (un-overridden)
    /// turn has its full toolbelt back.
    pub suppress_tools: bool,
    /// Skip auto-resuming this turn from the agent's most-recent on-disk
    /// transcript (`try_load_session_transcript`, which resolves the *latest*
    /// transcript for the agent name -- NOT thread-scoped). A host that has just
    /// re-bound the in-memory history to a different chat sets this so a cleared
    /// history is not silently repopulated from an unrelated thread's transcript
    /// (opencompany #1725). A thread-bound session no longer needs this: it
    /// resumes by durable session identity (`ResumeMode::Session`), which is an
    /// exact lookup and can never reach another thread's transcript.
    pub suppress_transcript_autoload: bool,
}

impl OpenHumanSessionHost {
    /// How this turn resumes its history.
    ///
    /// `suppress_transcript_autoload` is decided here, BEFORE `turn()` runs its
    /// explicit identity-keyed resume. `begin_turn_resume` applies the same
    /// override later, inside the lifecycle's resume hook, which is too late for
    /// a thread-bound session: that resume has already loaded the thread's own
    /// transcript into the history, so the override suppressed nothing (#6377).
    pub(super) fn turn_resume_mode(&self) -> tinyagents_runtime::ResumeMode {
        use tinyagents_runtime::ResumeMode;
        let suppressed = self
            .runtime_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pending_turn_overrides
            .suppress_transcript_autoload;
        if suppressed {
            tracing::debug!(
                thread_id = ?self.thread_id,
                "[session_host] transcript autoload suppressed for this turn"
            );
            ResumeMode::Never
        } else if self.session.is_some() {
            // Exact, identity-keyed resume. Unlike `LatestForAgent` it cannot
            // splice a different thread's transcript into this turn, and the
            // file it reads is the file the turn appends to.
            ResumeMode::Session
        } else if self
            .runtime_session
            .as_ref()
            .is_some_and(|session| session.history().is_empty())
        {
            ResumeMode::LatestForAgent
        } else {
            ResumeMode::Never
        }
    }
}

/// An autonomous or semi-autonomous AI agent.
///
/// The `OpenHumanSessionHost` is the central component that manages conversation state,
/// executes tools based on model requests, and interacts with the memory
/// system to maintain context across turns.
pub struct OpenHumanSessionHost {
    /// The sole live owner of model history, lossless transcript rows, prefix
    /// stability and per-turn immutable tool snapshots. Constructed lazily
    /// after the factory has installed hosted authority.
    pub(super) runtime_session:
        Option<tinyagents_runtime::Session<crate::agent::tinyagents::host::OpenHumanRunContext>>,
    /// Host-only mutable preparation/finalization state shared with the
    /// runtime hooks. It deliberately contains no generic session mechanics.
    pub(super) runtime_state:
        std::sync::Arc<std::sync::Mutex<super::runtime_session::OpenHumanSessionState>>,
    /// The turn's model source — builds this agent's tiered crate `ChatModel`
    /// set per turn (issue #4249, Phase 3 / Motion A). Replaces the raw
    /// `Arc<dyn Provider>`; the harness names crate model types only.
    pub(super) turn_model_source: TurnModelSource,
    /// Durable tool registry — everything the session was built with.
    /// Sub-agents pull from this via [`ParentExecutionContext::all_tools`].
    ///
    /// Fixed for the life of the agent: the synthesised delegation surface,
    /// which *does* change mid-session, lives in [`Self::synthesized_tools`]
    /// instead. See that field for why.
    pub(super) tools: Arc<Vec<Box<dyn Tool>>>,
    /// The delegation tools synthesised for the current connection set —
    /// `delegate_<toolkit>` skill tools and archetype delegates, produced by
    /// [`crate::tools::orchestrator_tools::collect_orchestrator_tools`].
    ///
    /// Held apart from [`Self::tools`] because it is the only part of the
    /// surface that changes mid-session, and `Box<dyn Tool>` is not cloneable:
    /// reconciling it *inside* `tools` meant `Arc::get_mut`, which fails
    /// whenever any reader holds a clone — in practice a detached sub-agent's
    /// cloned `ParentExecutionContext`, which outlives the turn that spawned
    /// it. The old code then reconciled `tool_specs` anyway and left the
    /// instances as they were, so the two halves drifted (#6145): a newly
    /// connected toolkit's delegate had a spec but no instance — and, the
    /// policy snapshot being built from the instances, no decision either, so
    /// the fail-closed visibility filter silently hid it until a unique-owner
    /// refresh — while a revoked toolkit's delegate lost its spec but stayed
    /// registered, advertised through its adapter, and callable.
    ///
    /// Because these instances are regenerated from scratch on every refresh,
    /// this `Arc` can always be *replaced* wholesale — no unique ownership
    /// required, so reconciliation cannot fail. Readers holding the previous
    /// `Arc` keep a consistent view for the rest of their turn, and the
    /// superseded instances are freed once the last of them drops.
    ///
    /// Disjoint from [`Self::tools`] by construction: a synthesised tool whose
    /// name a durable tool owns is dropped at build time and on every refresh
    /// (`builder::drop_synthesized_name_collisions`), so the durable tool wins
    /// on every surface. Every reader enumerates `tools` first and this set
    /// second — [`Self::tool_specs`], [`Self::all_tool_refs`], turn dispatch —
    /// so a name resolves in the same order everywhere. Empty for agents that
    /// do not delegate.
    pub(super) synthesized_tools: Arc<Vec<Box<dyn Tool>>>,
    /// Full tool specs: [`Self::tools`]' specs first, then the synthesised
    /// half, which [`OpenHumanSessionHost::refresh_delegation_tools`] swaps in place.
    ///
    /// The leaves are `Arc<ToolSpec>` and are **shared** with
    /// [`Self::durable_tool_specs`] and [`Self::visible_tool_specs`]: all three
    /// views point at the same schema objects, so a JSON-Schema `parameters`
    /// value is resident once per agent rather than three times
    /// (openhuman#6218 — it was ~1.1 MiB of the ~2.5 MiB a live agent cost).
    /// `refresh_delegation_tools` preserves that: `Arc::make_mut` clones the
    /// vector of pointers, never the schemas behind them. Anything that
    /// rebuilds an entry instead of cloning its `Arc` silently reintroduces the
    /// copy, which is why
    /// `builder_tests::part_01_tests::the_three_spec_views_share_their_leaf_schemas`
    /// asserts pointer identity rather than equal contents.
    pub(super) tool_specs: Arc<Vec<Arc<ToolSpec>>>,
    /// The specs of [`Self::tools`] alone, index for index. Sub-agents receive
    /// these via [`ParentExecutionContext::all_tool_specs`] beside
    /// [`Self::tools`], so a child's spec list can never name a synthesised
    /// delegate it holds no instance for (#4452). Fixed for the life of the
    /// agent, like the registry it describes.
    pub(super) durable_tool_specs: Arc<Vec<Arc<ToolSpec>>>,
    /// Tool specs filtered by the visible-tool allowlist and session
    /// permission policy. These are the specs actually sent to the
    /// provider in the main agent's chat requests.
    pub(super) visible_tool_specs: Arc<Vec<Arc<ToolSpec>>>,
    /// When non-empty, only these tool names are visible in the main
    /// agent's prompt and callable by the main agent. Sub-agents intersect
    /// their per-definition scopes with the effective parent-visible set.
    /// Empty = no filter (all tools visible, backward compat).
    pub(super) visible_tool_names: std::collections::HashSet<String>,
    /// Registered tools that left the wire under
    /// [`ToolExposure::Deferred`](tinytools::ToolExposure): never in
    /// [`Self::visible_tool_names`], never in the prompt's spec list, but
    /// handed to the harness beside the visible set so its intrinsic
    /// `tool_search` bridge can find them, and they are called by name. Empty for a
    /// belt that did not opt into discovery. Classified `Allow` by the
    /// policy session exactly like a visible tool — a found tool the gate
    /// refused as "prompt-hidden" would be the old unusable find again.
    pub(super) deferred_tool_names: std::collections::HashSet<String>,
    /// Whether this belt reaches deferred tools at all: a wildcard belt
    /// always does, a `[tools] named` belt only by listing `tool_search`.
    /// Fixed at build; the refresh paths use it to recompute
    /// [`Self::deferred_tool_names`] when the synthesised set changes.
    pub(super) discovery_enabled: bool,
    /// The agent definition's `deferred_tools`: `Direct` tools this agent
    /// reaches through `tool_search` instead of its wire. Kept so every
    /// recompute of [`Self::deferred_tool_names`] applies it again.
    pub(super) requested_deferred_tools: Arc<[String]>,
    /// Explicit profile/channel ceiling inherited by delegated agents.
    ///
    /// This is deliberately separate from [`Self::visible_tool_names`]: a
    /// coordinator can have a narrow direct surface while delegating work to a
    /// specialist with broader tools. Empty means no inherited ceiling.
    pub(super) subagent_tool_ceiling_names: std::collections::HashSet<String>,
    pub(super) tool_policy_session: ToolPolicySession,
    // `Arc` (not `Box`) so the tinyagents turn path can hold a cheap clone of
    // the dispatcher without borrowing the `OpenHumanSessionHost` while session state mutates.
    pub(super) tool_dispatcher: Arc<dyn ToolDialect>,
    pub(super) config: crate::config::AgentConfig,
    pub(super) model_name: String,
    /// User-configured vision capability for [`Self::model_name`], evaluated at
    /// session build from `model_vision_enabled(&model, config)`. Surfaced to the
    /// tinyagents image gate via the `current_model_vision` task-local so a
    /// custom/BYOK model the user flagged can forward images. Defaults to `false`.
    pub(super) model_vision: bool,
    pub(super) temperature: f64,
    pub(super) workspace_dir: std::path::PathBuf,
    pub(super) action_dir: std::path::PathBuf,
    /// Optional turn workspace descriptor. When set by an embedder, it is
    /// threaded into the top-level chat turn so acting tools (shell/file/git)
    /// resolve their default cwd to its root. `None` preserves the shared
    /// `action_dir` cwd behaviour.
    pub(super) workspace_descriptor: Option<tinytools::WorkspaceDescriptor>,
    pub(super) workflows: Vec<crate::skills::Workflow>,
    /// Last per-turn context block (active goal, running sub-agents) built for
    /// the current turn. Stored so it can be forwarded to subagents via
    /// `ParentExecutionContext`.
    pub(super) last_memory_context: Option<String>,
    /// Holistic token/cost/context accounting for the most recent turn (parent +
    /// any sub-agents spawned during it). Consumed by web-channel delivery to
    /// surface session token/cost/context meters in the UI footer. `None` until
    /// the first turn completes.
    /// Whether the most recent turn's tinyagents loop paused because it hit
    /// `max_tool_iterations` (`TinyagentsTurnOutcome::hit_cap`), rather than
    /// finishing naturally. `false` until the first turn completes, and reset
    /// on every subsequent turn — so it only ever reflects the LAST turn, not
    /// "any turn ever". Consumed by callers that run a single headless turn
    /// via [`run_single`](super::runtime) (e.g. `flows_build`) and need to
    /// distinguish "the agent paused mid-work" from "the agent asked a
    /// question" or "the agent finished" — `run_single` only returns the
    /// checkpoint/final text, with no other signal for which case occurred.
    pub(super) post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    pub(super) event_session_id: String,
    pub(super) event_channel: String,
    /// Backend/session thread explicitly owned by this agent run. It is kept
    /// separate from event identity: CLI and worker event sessions are not
    /// necessarily user conversation threads.
    pub(super) thread_id: Option<String>,
    /// Human-readable agent definition name (e.g. `"main"`,
    /// `"task_manager_agent"`). Used as the `{agent}` component in session
    /// transcript paths: `sessions/DDMMYYYY/{agent}_{index}.md`.
    ///
    /// May be rewritten mid-session by
    /// [`OpenHumanSessionHost::set_agent_definition_name`] (e.g. the web channel
    /// stamps `"orchestrator_<short_thread>"` so each thread gets its
    /// own transcript namespace). Anything that needs to resolve the
    /// session back to its registry entry must use
    /// [`Self::agent_definition_id`], not this field.
    pub(super) agent_definition_name: String,
    /// Canonical agent id as registered in
    /// [`AgentDefinitionRegistry`] (e.g. `"orchestrator"`,
    /// `"planner"`). Set once at build time and never
    /// rewritten — `set_agent_definition_name` only touches the
    /// transcript-facing `agent_definition_name`, so registry lookups
    /// (e.g. `refresh_delegation_tools` re-resolving the agent's
    /// `subagents` list post-fetch) stay correct even after the web
    /// channel's per-thread rename.
    ///
    /// [`AgentDefinitionRegistry`]: crate::agent::harness::definition::AgentDefinitionRegistry
    pub(super) agent_definition_id: String,
    /// Injected transcript locator, or `None` to use real files.
    ///
    /// The single injection point for the whole transcript seam: it resolves
    /// both resume reads (`latest_for_agent`, `root_for_thread`) and binds this
    /// session's write handle (`open_stem`). `None` is the production default
    /// and is resolved *lazily* by
    /// [`OpenHumanSessionHost::session_locator`][Self::session_locator] into a
    /// [`FileTranscriptLocator`][tinyagents_session::transcript::FileTranscriptLocator]
    /// over the **current** `workspace_dir` — never
    /// captured at build time, because callers (tests especially) reassign
    /// `workspace_dir` after `build()` and a frozen locator would silently keep
    /// reading the old directory.
    pub(super) session_history_locator:
        Option<std::sync::Arc<dyn tinyagents_session::transcript::TranscriptLocator>>,
    /// First-call memo for [`OpenHumanSessionHost::session_locator`][Self::session_locator]'s
    /// lazily-constructed `FileTranscriptLocator` (the `session_history_locator` `None` branch).
    ///
    /// tinyagents' `SessionBuilder` only accepts a later transcript-target
    /// change when it is the *same* locator object (`Arc::ptr_eq`), not merely
    /// an equivalent one — see `tinyagents_session::transcript`'s
    /// `TranscriptTarget::same_binding`. `session_locator` is called from more
    /// than one place while building a session's runtime turn machinery (the
    /// `before_resume` resume target and the eager construction-time bind), and
    /// without this memo each call minted a fresh `Arc` over the same
    /// destination, so the second bind was rejected with "cannot change a
    /// transcript target after it is bound or committed" even though both
    /// calls agreed on the file. `OnceLock` keeps the resolution lazy — still
    /// read from `workspace_dir` at first use, not frozen at struct-build time
    /// — while guaranteeing every later caller in this host's lifetime gets
    /// back the identical `Arc`.
    pub(super) session_history_locator_memo:
        std::sync::OnceLock<std::sync::Arc<dyn tinyagents_session::transcript::TranscriptLocator>>,
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
    /// Durable identity of the conversation this root session serves.
    ///
    /// Set by [`set_thread_id`][super::runtime::accessors] once the host knows
    /// the thread, and `None` for a sub-agent or an unthreaded session. When
    /// present it, not [`Self::session_key`], addresses the transcript: the
    /// stem it derives carries no timestamp, so every restart and every
    /// process resolves the same conversation to the same file. The
    /// timestamped `session_key` remains correct for sub-agents, where each
    /// spawn genuinely is a new transcript.
    pub(super) session: Option<tinyagents_session::transcript::SessionRef>,
    /// Per-session [`ContextManager`] — owns the system-prompt
    /// builder, the layered reduction pipeline (tool-result budget →
    /// microcompact → autocompact signal → session-memory extraction
    /// trigger), the guard's compaction circuit breaker, and the LLM
    /// summarizer that runs when the pipeline asks for autocompaction.
    /// Constructed once at session start so its budget counters and
    /// session-memory deltas persist across turns. See
    /// [`crate::agent::context`] for the full surface.
    /// Host-only context/prompt and session-memory bookkeeping shared by the
    /// runtime hooks; it is never a duplicate conversation history.
    pub(super) context: Arc<std::sync::Mutex<ContextManager>>,
    /// Optional progress event sender for real-time turn progress.
    /// When set, the turn loop emits [`AgentProgress`] events through
    /// this channel so callers (e.g. web channel) can surface live
    /// tool-call and iteration updates to the UI.
    pub(super) on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    /// Optional active-run queue for mid-turn steering. When set, the
    /// engine drains steers/collects at iteration boundaries.
    pub(super) run_queue:
        Option<Arc<tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
    /// Active Composio integrations the user has connected. Populated at
    /// agent build time and threaded into each agent's `prompt.rs` so
    /// the delegator / skill-executor voices can render their own
    /// integration blocks.
    pub(super) connected_integrations: Vec<crate::agent::prompts::ConnectedIntegration>,
    /// Whether `connected_integrations` is an authoritative session-start
    /// snapshot (prewarmed from the shared Composio cache or fetched
    /// explicitly) versus the default empty placeholder installed by
    /// `SessionHostBuilder::build`. Turn 1 uses this to decide whether it must
    /// still pay the cold-start fetch cost before freezing the system prompt.
    pub(super) connected_integrations_initialized: bool,
    /// Full runtime config snapshot for integration-cache reads and the
    /// best-effort fallback fetch path. Session agents built from
    /// `Config` carry this directly so the turn loop does not need to
    /// re-run `Config::load_or_init()` on the hot path just to key into
    /// the Composio cache.
    pub(super) runtime_config: Option<Arc<crate::config::Config>>,
    /// Durable inputs for hosted TinyAgents invocations. Built once by the
    /// production factory; per-turn tools and progress remain outside it.
    pub(super) hosted_base: Option<Arc<crate::agent::tinyagents::host::OpenHumanHostBase>>,
    /// The definition this session was built from, when the factory had one.
    ///
    /// Read back through [`OpenHumanSessionHost::resolved_definition`] by the in-turn sites
    /// that need the definition's `sandbox_mode` or `subagents` — so an agent
    /// built from an explicit definition (a library host's per-agent spec)
    /// keeps those settings instead of having them silently replaced by
    /// whatever the process-global registry holds under the same id.
    pub(super) definition: Option<Arc<crate::agent::harness::definition::AgentDefinition>>,
    /// Mirrors the agent definition's `omit_memory_context` flag: when set,
    /// a new session does not get the compiled `context.md` prepended to its
    /// first user message. Defaults to `false` (inject) for agents built
    /// without a definition.
    pub(super) omit_memory_context: bool,
    /// Optional payload-summarizer wired in at agent-build time.
    /// Currently set only for the orchestrator session
    /// (see [`super::builder`]). TinyAgents `ToolOutputMiddleware` uses this
    /// when oversized tool results need summarizer-subagent compression before
    /// they enter agent history.
    pub(super) payload_summarizer:
        Option<Arc<dyn crate::agent::tinyagents::payload_summarizer::PayloadSummarizer>>,
    /// Per-agent TokenJuice profile for tool results entering this session's
    /// model context.
    pub(super) tokenjuice_compression: crate::inference::tokenjuice::AgentTokenjuiceCompression,
    /// Pre-execution policy hook for tool calls in this session. The
    /// default policy allows all calls so existing agents keep their
    /// behaviour unless a caller opts into stricter policy.
    pub(super) tool_policy: Arc<dyn ToolPolicy>,
    /// Hash of the Composio connection set this OpenHumanSessionHost last reconciled
    /// against. Compared at top-of-turn to a fresh hash computed from
    /// [`crate::integrations::composio::cached_active_integrations`]; on
    /// diff, [`OpenHumanSessionHost::refresh_delegation_tools`] re-synthesises the
    /// `delegate_<toolkit>` surface to match the live connected set.
    ///
    /// Initialised to `0` at construction. Turn 1's existing refresh
    /// path (gated by `history.is_empty()`) writes the first real hash
    /// after [`OpenHumanSessionHost::fetch_connected_integrations`] populates
    /// [`OpenHumanSessionHost::connected_integrations`], so the per-turn check is
    /// dormant on session startup and only fires when integrations
    /// actually change mid-conversation.
    pub(super) last_seen_integrations_hash: u64,
    /// Per-session raw receiver for `DomainEvent::ComposioIntegrationsChanged`.
    /// Armed lazily on first turn when the global event bus is available.
    /// Drained before each provider dispatch so a connection that flips to
    /// ACTIVE mid-turn can refresh the delegation schema in the same thread.
    pub(super) composio_integrations_rx:
        Option<tinybus::events::EventReceiver<crate::core::events::DomainEvent>>,
    /// Lazily-armed global-bus receiver for [`DomainEvent::WorkflowsChanged`]
    /// (skill install / uninstall / create). Drained at each turn boundary so
    /// `refresh_workflows` only re-scans disk when the installed set actually
    /// changed — no per-turn filesystem walk on the steady-state hot path.
    pub(super) skill_events_rx:
        Option<tinybus::events::EventReceiver<crate::core::events::DomainEvent>>,
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
    /// [`Self::pending_integration_announcement`]. Connected servers' tools are
    /// `Deferred` registrations found through `tool_search` (no per-server
    /// schema on the wire to refresh), so this prose note on the user turn is
    /// the entire mid-session-connect mechanism for MCP. The
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
    /// Names of every tool currently in [`OpenHumanSessionHost::synthesized_tools`] — those
    /// produced by [`crate::tools::orchestrator_tools::collect_orchestrator_tools`]
    /// (i.e. `delegate_<toolkit>` skill tools and archetype-delegation
    /// tools). Tracked so
    /// [`OpenHumanSessionHost::refresh_delegation_tools`] can drop the entire
    /// previously-synthesised subset of [`OpenHumanSessionHost::tool_specs`] on each refresh
    /// and append the fresh set — without that mask we'd risk either leaking
    /// stale `delegate_<toolkit>` specs on revoke or accidentally removing
    /// direct tools (`cron_add`, …) that share a name
    /// prefix.
    ///
    /// Seeded by [`SessionHostBuilder::build`] from the set handed to
    /// [`SessionHostBuilder::synthesized_tools`], then replaced by
    /// `refresh_delegation_tools` on every refresh.
    ///
    /// Invariant: this set is the name mask for **both** [`Self::tool_specs`]'
    /// synthesised half and [`Self::synthesized_tools`], which reconcile
    /// together on every refresh. There is no longer a case where one advances
    /// without the other — the schema and the executable instances cannot
    /// drift (#6145).
    pub(super) synthesized_tool_names: std::collections::HashSet<String>,
}

/// A builder for creating `OpenHumanSessionHost` instances with custom configuration.
pub struct SessionHostBuilder {
    pub(super) turn_model_source: Option<TurnModelSource>,
    pub(super) tools: Option<Vec<Box<dyn Tool>>>,
    /// Delegation tools synthesised for the session's initial connection set.
    /// Held in [`OpenHumanSessionHost::synthesized_tools`], never inside [`OpenHumanSessionHost::tools`].
    pub(super) synthesized_tools: Option<Vec<Box<dyn Tool>>>,
    /// When set, restricts which tools the main agent sees/calls.
    pub(super) visible_tool_names: Option<std::collections::HashSet<String>>,
    /// Names removed from the final provider-visible set after wildcard
    /// expansion and tool synthesis.
    pub(super) withheld_tool_names: std::collections::HashSet<String>,
    /// See [`SessionHostBuilder::deferred_tools`].
    pub(super) deferred_tools: Vec<String>,
    /// Optional explicit profile ceiling for tools delegated agents may inherit.
    /// Channel-policy restrictions are intersected during [`Self::build`].
    pub(super) subagent_tool_ceiling_names: Option<std::collections::HashSet<String>>,
    pub(super) prompt_builder: Option<SystemPromptBuilder>,
    pub(super) tool_dispatcher: Option<Box<dyn ToolDialect>>,
    pub(super) config: Option<crate::config::AgentConfig>,
    /// Optional [`ContextConfig`] override threaded through from
    /// `OpenHumanSessionHost::from_config`. When unset the builder falls back to
    /// [`crate::config::ContextConfig::default`].
    pub(super) context_config: Option<crate::config::ContextConfig>,
    pub(super) model_name: Option<String>,
    /// User vision flag for the resolved model; `None` → `false` in `build()`.
    pub(super) model_vision: Option<bool>,
    pub(super) temperature: Option<f64>,
    pub(super) workspace_dir: Option<std::path::PathBuf>,
    pub(super) action_dir: Option<std::path::PathBuf>,
    /// Optional turn workspace descriptor forwarded to [`OpenHumanSessionHost`] at build time.
    /// Defaults to `None` (shared `action_dir` cwd).
    pub(super) workspace_descriptor: Option<tinytools::WorkspaceDescriptor>,
    pub(super) workflows: Option<Vec<crate::skills::Workflow>>,
    /// OpenHumanSessionHost workflows to surface in the prompt. Populated from `load_workflows`
    /// at session start; defaults to empty when not explicitly set.
    pub(super) post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    pub(super) event_session_id: Option<String>,
    pub(super) event_channel: Option<String>,
    pub(super) agent_definition_name: Option<String>,
    /// The session's own definition, when the caller has one rather than a
    /// registry id to name.
    ///
    /// A hosted root invocation resolves its agent id against the host
    /// catalogue before it composes a message, and refuses the turn when the
    /// id is not there. `from_config_with_definition` stamps a caller's
    /// definition on the session for exactly that reason; a direct builder
    /// caller had no equivalent, so its only catalogue was the process-wide
    /// [`AgentDefinitionRegistry`] — read once from `<workspace>/agents/*.toml`
    /// at startup, with no refresh.
    ///
    /// Set here it becomes
    /// [`OpenHumanHostBase::session_definition`](crate::agent::tinyagents::host::OpenHumanHostBase::session_definition),
    /// which outranks the registry for this session's own id.
    ///
    /// `None` (the default) leaves every existing caller as it was.
    ///
    /// [`AgentDefinitionRegistry`]: crate::agent::harness::definition::AgentDefinitionRegistry
    pub(super) session_definition: Option<Arc<crate::agent::harness::definition::AgentDefinition>>,
    /// Directory chain of parent session keys for a sub-agent. `None`
    /// (default) means this is a root session — its transcript lands
    /// flat in `session_raw/DDMMYYYY/{session_key}.jsonl`. Populated
    /// by the sub-agent runner so nested delegations produce a tree.
    pub(super) session_parent_prefix: Option<String>,
    /// Forwarded to [`OpenHumanSessionHost::session_history_locator`]. `None` (default) means
    /// real files; set it with
    /// [`with_session_history_locator`][super::builder::SessionHostBuilder::with_session_history_locator]
    /// to substitute the transcript backing store for the whole turn path.
    pub(super) session_history_locator:
        Option<std::sync::Arc<dyn tinyagents_session::transcript::TranscriptLocator>>,
    /// Forwarded to [`OpenHumanSessionHost::omit_memory_context`] at `build()`
    /// time; `None` falls back to `false` (inject `context.md`).
    pub(super) omit_memory_context: Option<bool>,
    /// Optional payload-summarizer threaded through to [`OpenHumanSessionHost`] at
    /// build time. Defaults to `None`; the orchestrator branch in
    /// [`super::builder::OpenHumanSessionHost::build_session_agent_inner`] sets this
    /// to a `SubagentPayloadSummarizer` instance.
    pub(super) payload_summarizer:
        Option<Arc<dyn crate::agent::tinyagents::payload_summarizer::PayloadSummarizer>>,
    /// Per-agent TokenJuice tool-output compression profile.
    pub(super) tokenjuice_compression: crate::inference::tokenjuice::AgentTokenjuiceCompression,
    /// Optional pre-execution tool policy. Defaults to allow-all.
    pub(super) tool_policy: Option<Arc<dyn ToolPolicy>>,
}

impl Default for SessionHostBuilder {
    fn default() -> Self {
        Self::new()
    }
}
