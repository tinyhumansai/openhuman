//! Data types shared across the prompt-plumbing pipeline.
//!
//! Everything in this file is pure data (structs, enums, traits,
//! constants). The rendering logic — section implementations,
//! `SystemPromptBuilder`, `render_subagent_system_prompt` — lives in
//! the sibling `mod.rs` so type edits don't pull in the whole 2 000-line
//! renderer.

use crate::openhuman::skills::Skill;
use crate::openhuman::tools::Tool;
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::path::Path;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const BOOTSTRAP_MAX_CHARS: usize = 20_000;

/// Tight per-file budget for user-specific, potentially growing files —
/// currently `PROFILE.md` (onboarding enrichment output) and `MEMORY.md`
/// (archivist-curated long-term memory). Caps the prompt footprint so
/// either file can reach at most ~1000 tokens (a few % of a typical
/// context window) regardless of how large the on-disk version has
/// grown.
pub(crate) const USER_FILE_MAX_CHARS: usize = 2_000;

/// Per-namespace cap when injecting tree summarizer root summaries into
/// the prompt. ~8 000 chars ≈ 2 000 tokens — that's the floor the user
/// asked for ("at least 2000 tokens of user memory") for a single
/// namespace, and matches what the tree summarizer's `Day` level
/// already enforces upstream.
///
/// **Note**: this constant matches the `Balanced` preset of
/// [`crate::openhuman::config::schema::agent::MemoryContextWindow`] —
/// the live agent harness now resolves the per-namespace cap from that
/// preset (see `AgentConfig::resolved_memory_limits`). The constant is
/// kept as the documented baseline for prompt-section authors.
#[allow(dead_code)]
pub(crate) const USER_MEMORY_PER_NAMESPACE_MAX_CHARS: usize = 8_000;

/// Hard ceiling across all namespaces, so a workspace with 30 namespaces
/// doesn't burn the entire context window. ~32 000 chars ≈ 8 000 tokens.
///
/// **Note**: same Balanced-preset baseline relationship as
/// `USER_MEMORY_PER_NAMESPACE_MAX_CHARS` — see its rustdoc.
#[allow(dead_code)]
pub(crate) const USER_MEMORY_TOTAL_MAX_CHARS: usize = 32_000;

// ─────────────────────────────────────────────────────────────────────────────
// Learned context (pre-fetched, not blocking)
// ─────────────────────────────────────────────────────────────────────────────

/// Pre-fetched learned context data for prompt sections (avoids blocking the runtime).
#[derive(Debug, Clone, Default)]
pub struct LearnedContextData {
    /// Recent observations from the learning subsystem.
    pub observations: Vec<String>,
    /// Recognized patterns.
    pub patterns: Vec<String>,
    /// Learned user profile entries.
    pub user_profile: Vec<String>,
    /// Explicit user reflections captured from chat — distinct, high-priority
    /// memory class. These are the user's own intentional self-statements
    /// ("remember that I…", "going forward…", "I realized…") and are
    /// privileged above generic [`Self::tree_root_summaries`] when the
    /// orchestrator assembles its system prompt. Empty when the learning
    /// subsystem is off or no reflections have been captured yet.
    pub reflections: Vec<String>,
    /// Pre-fetched root-level summaries from the tree summarizer, one per
    /// namespace that has a root node on disk. Empty when the tree
    /// summarizer hasn't run.
    ///
    /// Each entry carries the namespace's root `updated_at` so the
    /// renderer can stamp how current the memory is. Without that stamp
    /// the model treats distilled memory as present-tense and can serve
    /// a stale summary as today's update (#2944).
    pub tree_root_summaries: Vec<NamespaceSummary>,
}

/// A single memory-namespace root summary fetched from the tree
/// summarizer, paired with the timestamp of its root node.
///
/// `updated_at` is rendered as an absolute date (not a relative
/// "N days ago") on purpose: this block sits near the front of the
/// KV-cache-stable system prompt, so a label that changes every day
/// would bust the cached prefix for everything after it. An absolute
/// date only changes when the underlying memory does; the model judges
/// freshness by comparing it against the `## Current Date & Time`
/// section. See [`LearnedContextData::tree_root_summaries`] (#2944).
#[derive(Debug, Clone)]
pub struct NamespaceSummary {
    /// Memory namespace this root summary belongs to (e.g. `activities`).
    pub namespace: String,
    /// The distilled root summary text.
    pub body: String,
    /// When the namespace's root node was last updated on disk.
    pub updated_at: DateTime<Utc>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Connected integrations (Composio toolkits)
// ─────────────────────────────────────────────────────────────────────────────

/// An external integration (e.g. a Composio OAuth-backed toolkit)
/// surfaced in the system prompt so the orchestrator knows which
/// services are available — both **already connected** and **available
/// to authorize**.
#[derive(Debug, Clone)]
pub struct ConnectedIntegration {
    /// Toolkit slug, e.g. `"gmail"`, `"notion"`.
    pub toolkit: String,
    /// Human-readable one-line description of what this integration can do.
    pub description: String,
    /// Per-action catalogue (only populated when `connected == true`).
    pub tools: Vec<ConnectedIntegrationTool>,
    /// Per-action catalogue for actions that the toolkit **does** support but
    /// the user has **not** unlocked via their per-toolkit scope preferences.
    /// The prompt renderer surfaces these descriptively (name + one-line +
    /// which scope is missing) so the agent can honestly answer "do you have
    /// X?" with "yes, but you need to flip the {scope} toggle in
    /// Connections → {toolkit}" — instead of silently claiming the
    /// capability doesn't exist (which is what happens when the agent has
    /// zero awareness of pref-gated actions).
    ///
    /// The agent CANNOT directly invoke these (no `parameters` schema is
    /// exposed; the LLM lacks the function definition) and it cannot flip
    /// the gating scope itself — there is no agent-callable scope-elevate
    /// tool. Intended flow: agent sees a gated tool → tells the user what
    /// it does + names the `unlock_paths` from the data → the user toggles
    /// the scope in the Connections UI → on the next turn the action
    /// graduates from `gated_tools` to `tools` and becomes callable.
    pub gated_tools: Vec<GatedIntegrationTool>,
    /// Whether the user has an active OAuth connection for this
    /// toolkit. When `false`, the toolkit is in the backend allowlist
    /// but no authorization has been completed yet — `tools` is empty
    /// and the orchestrator must point the user at Settings instead of
    /// attempting to delegate.
    pub connected: bool,
    /// Raw upstream connection status when a connection row exists but
    /// is not `ACTIVE` — e.g. `"INITIATED"`, `"INITIALIZING"`,
    /// `"FAILED"`, `"EXPIRED"`. `None` means either the user is
    /// `ACTIVE` (use `connected = true`) OR there is no connection
    /// row at all (truly disconnected).
    ///
    /// Used by the `integrations_agent` spawn-gate to surface the
    /// real reason a delegation can't proceed — see issue #2365
    /// ("Agent says Gmail is disconnected when sending email"). The
    /// gate previously emitted the same "not authorized yet" message
    /// regardless of whether OAuth was mid-flight, the token had
    /// expired, or the user had simply never started the flow.
    pub non_active_status: Option<String>,
}

/// A toolkit action that exists in the catalog but is currently hidden from
/// the agent's callable function list because the user's scope preference
/// for this toolkit does not allow the action's required scope.
///
/// Deliberately no `parameters` field: the LLM should NOT be able to construct
/// a call envelope for a gated tool — it can only describe its existence and
/// point the user at the unlock path. The agent has no scope-elevate tool;
/// once the user toggles the gating scope in the Connections UI, the action
/// moves from `ConnectedIntegration.gated_tools` to `ConnectedIntegration.tools`
/// on the next prompt rebuild and becomes a real callable function.
#[derive(Debug, Clone)]
pub struct GatedIntegrationTool {
    /// Action slug, e.g. `"GMAIL_BATCH_DELETE_MESSAGES"`.
    pub name: String,
    /// One-line description of the action.
    pub description: String,
    /// Which scope the user must enable for this action to become callable.
    /// Lowercase: `"read"`, `"write"`, `"admin"`. The vast majority of gated
    /// rows are `"admin"` (destructive actions); `"write"` only appears for
    /// users who have explicitly turned write off, which is unusual.
    pub required_scope: String,
    /// Literal lines the agent should show the user, verbatim, when offering
    /// to unlock this action — one entry per available path (typically: the
    /// agent-side meta-tool, and the manual UI toggle). Populated at
    /// partition time in `composio::ops`. The prompt-side rule is "show
    /// these to the user, don't substitute your own framing" — keeping the
    /// text in the data (not in the system prompt) lets us tweak wording
    /// without invalidating the KV-cache prefix and avoids biasing the
    /// model toward a memorized template that drops options.
    pub unlock_paths: Vec<String>,
}

/// A single action available on a connected integration.
#[derive(Debug, Clone)]
pub struct ConnectedIntegrationTool {
    /// Action slug, e.g. `"GMAIL_SEND_EMAIL"`.
    pub name: String,
    /// One-line description of the action.
    pub description: String,
    /// JSON schema for the action's parameters. `None` when the backend
    /// didn't supply a schema.
    pub parameters: Option<serde_json::Value>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool descriptor + call-format
// ─────────────────────────────────────────────────────────────────────────────

/// A lightweight tool descriptor for prompt rendering.
///
/// Shared shape so every call-site that builds a system prompt can feed
/// the same rendering pipeline — main agents (which own `Box<dyn Tool>`),
/// sub-agents, and channel runtimes (which only have `(name,
/// description)` tuples) all adapt to this.
#[derive(Debug, Clone)]
pub struct PromptTool<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub parameters_schema: Option<String>,
}

impl<'a> PromptTool<'a> {
    pub fn new(name: &'a str, description: &'a str) -> Self {
        Self {
            name,
            description,
            parameters_schema: None,
        }
    }

    pub fn with_schema(name: &'a str, description: &'a str, parameters_schema: String) -> Self {
        Self {
            name,
            description,
            parameters_schema: Some(parameters_schema),
        }
    }

    /// Adapt a `Box<dyn Tool>` slice into a `Vec<PromptTool<'_>>`.
    pub fn from_tools(tools: &'a [Box<dyn Tool>]) -> Vec<PromptTool<'a>> {
        tools
            .iter()
            .map(|t| PromptTool {
                name: t.name(),
                description: t.description(),
                parameters_schema: Some(t.parameters_schema().to_string()),
            })
            .collect()
    }
}

/// How the tool catalogue should render each tool entry. Driven by the
/// dispatcher choice on the agent — JSON-schema rendering is the
/// historic format; P-Format is the new default text protocol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ToolCallFormat {
    /// `tool_name[arg1|arg2|...]` — compact, positional. Default.
    #[default]
    PFormat,
    /// Legacy JSON-in-tag rendering with full schemas.
    Json,
    /// Provider supplies structured tool calls — catalogue is
    /// informational. Renders in the same JSON-schema form as `Json`.
    Native,
}

// ─────────────────────────────────────────────────────────────────────────────
// Authenticated user identity
// ─────────────────────────────────────────────────────────────────────────────

/// Non-secret user identity fields surfaced to the prompt layer so
/// agents stop asking the user for information the app already has —
/// see issue #926.
///
/// Only **identifying** fields land here; tokens, refresh tokens, and
/// any opaque credential material are forbidden. The struct is
/// constructed from the cached `auth_get_me` response in
/// `app_state::ops::peek_cached_current_user_identity`, which strips
/// everything but `id` / `email` / `name` before returning.
#[derive(Debug, Clone, Default)]
pub struct UserIdentity {
    pub id: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
}

impl UserIdentity {
    pub fn is_empty(&self) -> bool {
        self.id.is_none() && self.name.is_none() && self.email.is_none()
    }
}

/// Frozen `MEMORY.md` + `USER.md` bodies for prompt injection.
///
/// Lives in the prompt layer (not `openhuman::curated_memory`) so agent
/// prompt plumbing compiles in builds where the curated-memory domain
/// module is not present.
#[derive(Debug, Clone)]
pub struct CuratedMemoryPromptSnapshot {
    pub memory: String,
    pub user: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Prompt context (everything a section needs)
// ─────────────────────────────────────────────────────────────────────────────

/// An entry in the master agent's personality roster prompt section.
#[derive(Debug, Clone, Default)]
pub struct PersonalityRosterEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub memory_summary: Option<String>,
}

pub struct PromptContext<'a> {
    pub workspace_dir: &'a Path,
    pub model_name: &'a str,
    /// Id of the agent this prompt is being built for.
    pub agent_id: &'a str,
    pub tools: &'a [PromptTool<'a>],
    pub skills: &'a [Skill],
    pub dispatcher_instructions: &'a str,
    /// Pre-fetched learned context (empty when learning is disabled).
    pub learned: LearnedContextData,
    /// When non-empty, only tools in this set are rendered. Skills
    /// section is also omitted when a filter is active.
    pub visible_tool_names: &'a std::collections::HashSet<String>,
    pub tool_call_format: ToolCallFormat,
    /// Active Composio integrations the user has connected.
    pub connected_integrations: &'a [ConnectedIntegration],
    /// Pre-rendered `## Connected Identities` markdown block loaded once
    /// by the caller so prompt builders remain deterministic and avoid
    /// hidden global reads during `build(ctx)`.
    pub connected_identities_md: String,
    /// When `true`, inject `PROFILE.md` (onboarding enrichment output).
    pub include_profile: bool,
    /// When `true`, inject `MEMORY.md` (archivist-curated long-term
    /// memory). Capped at [`USER_FILE_MAX_CHARS`] and frozen per session.
    pub include_memory_md: bool,
    /// Session-scoped curated-memory snapshot (`MEMORY.md` + `USER.md`)
    /// captured once at turn start and reused by every delegated
    /// sub-agent to keep prompt context byte-identical within the turn.
    /// `None` when no snapshot is attached (unit tests, curated-memory
    /// runtime unavailable) — [`UserFilesSection`] falls back to workspace
    /// files.
    pub curated_snapshot: Option<std::sync::Arc<CuratedMemoryPromptSnapshot>>,
    /// Authenticated user identity (id/name/email) when available — see
    /// [`UserIdentity`]. `None` for unauthenticated paths (CLI without a
    /// session, tests). Pre-fetched by the caller from the
    /// `auth_get_me` cache so prompt builders never reach the network.
    pub user_identity: Option<UserIdentity>,
    /// Personality-specific SOUL.md content. When `Some`, the
    /// `IdentitySection` uses this instead of reading the workspace
    /// root `SOUL.md`. `None` falls back to existing behavior.
    pub personality_soul_md: Option<String>,
    /// Personality-specific MEMORY.md content. When `Some`, the
    /// `UserFilesSection` uses this instead of reading the workspace
    /// root `MEMORY.md`. `None` falls back to existing behavior.
    pub personality_memory_md: Option<String>,
    /// Non-self personality roster entries for the master agent's prompt.
    /// Empty for non-master agents.
    pub personality_roster: Vec<PersonalityRosterEntry>,
    /// Agent workflows available in this session. Injected into the prompt
    /// so agents know which workflows they can invoke via `workflow_phase`.
    /// Empty when no workflows are installed or the harness has not yet loaded
    /// them (sub-agent paths, tests, etc.).
    pub workflows: &'a [crate::openhuman::agent_workflows::Workflow],
}

// ─────────────────────────────────────────────────────────────────────────────
// PromptSection trait + rendered output
// ─────────────────────────────────────────────────────────────────────────────

pub trait PromptSection: Send + Sync {
    fn name(&self) -> &str;
    fn build(&self, ctx: &PromptContext<'_>) -> Result<String>;
}

// ─────────────────────────────────────────────────────────────────────────────
// Sub-agent render options (per-definition flags)
// ─────────────────────────────────────────────────────────────────────────────

/// Per-definition rendering flags passed into the sub-agent prompt
/// renderer. Mirrors the `omit_*` fields on
/// [`crate::openhuman::agent::harness::definition::AgentDefinition`]
/// but inverted into positive-sense `include_*` form.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubagentRenderOptions {
    pub include_safety_preamble: bool,
    pub include_identity: bool,
    pub include_skills_catalog: bool,
    pub include_profile: bool,
    pub include_memory_md: bool,
}

impl SubagentRenderOptions {
    /// Build the narrow default (every section off).
    pub fn narrow() -> Self {
        Self::default()
    }

    /// Construct from per-definition `omit_*` flags, inverting into the
    /// positive-sense `include_*` shape.
    pub fn from_definition_flags(
        omit_identity: bool,
        omit_safety_preamble: bool,
        omit_skills_catalog: bool,
        omit_profile: bool,
        omit_memory_md: bool,
    ) -> Self {
        Self {
            include_identity: !omit_identity,
            include_safety_preamble: !omit_safety_preamble,
            include_skills_catalog: !omit_skills_catalog,
            include_profile: !omit_profile,
            include_memory_md: !omit_memory_md,
        }
    }
}
