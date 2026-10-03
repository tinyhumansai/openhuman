//! Data types shared across the prompt-plumbing pipeline.
//!
//! Everything in this file is pure data (structs, enums, traits,
//! constants). The rendering logic — section implementations,
//! `SystemPromptBuilder`, `render_subagent_system_prompt` — lives in
//! the sibling `mod.rs` so type edits don't pull in the whole 2 000-line
//! renderer.

use crate::skills::Workflow;
use anyhow::Result;
use std::path::Path;
use tinytools::Tool;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const BOOTSTRAP_MAX_CHARS: usize = 20_000;

// ─────────────────────────────────────────────────────────────────────────────
// Connected integrations (Composio toolkits)
// ─────────────────────────────────────────────────────────────────────────────

/// Identity of a single active connection within a toolkit.
///
/// Surfaced in the system prompt so the orchestrator can disambiguate
/// multiple accounts for the same toolkit (e.g. "work Gmail" vs
/// "personal Gmail") and pass the correct `connection_id` to the
/// execute pipeline.
#[derive(Debug, Clone)]
pub struct IntegrationConnection {
    /// Composio connection ID — passed to execute when the user/agent
    /// targets a specific account.
    pub connection_id: String,
    /// Human-readable label derived from the connection's identity
    /// fields: `account_email`, `workspace`, or `username` (first
    /// non-empty wins). `None` when identity hasn't been enriched yet.
    pub label: Option<String>,
    /// Whether this is the default connection for the toolkit (oldest
    /// active connection by `created_at`).
    pub is_default: bool,
}

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
    /// All active connections for this toolkit, sorted by `created_at`
    /// ascending (oldest first). The first entry is the default.
    /// Empty when `connected == false`.
    pub connections: Vec<IntegrationConnection>,
    /// Raw upstream connection status when a connection row exists but
    /// is not `ACTIVE` — e.g. `"INITIATED"`, `"INITIALIZING"`,
    /// `"FAILED"`, `"EXPIRED"`. `None` means either the user is
    /// `ACTIVE` (use `connected = true`) OR there is no connection
    /// row at all (truly disconnected).
    ///
    /// Distinguishes an OAuth flow still in flight, an expired token and
    /// a connection never started (issue #2365).
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
    pub name: std::borrow::Cow<'a, str>,
    pub description: std::borrow::Cow<'a, str>,
    pub parameters_schema: Option<String>,
}

impl<'a> PromptTool<'a> {
    pub fn new(name: &'a str, description: &'a str) -> Self {
        Self {
            name: std::borrow::Cow::Borrowed(name),
            description: std::borrow::Cow::Borrowed(description),
            parameters_schema: None,
        }
    }

    /// An entry the catalogue owns rather than borrows: a tool that exists
    /// only for this prompt build (the harness's `tool_search`
    /// bridge), with no registration to borrow a name from.
    pub fn owned(
        name: String,
        description: String,
        parameters_schema: String,
    ) -> PromptTool<'static> {
        PromptTool {
            name: std::borrow::Cow::Owned(name),
            description: std::borrow::Cow::Owned(description),
            parameters_schema: Some(parameters_schema),
        }
    }

    pub fn with_schema(name: &'a str, description: &'a str, parameters_schema: String) -> Self {
        Self {
            name: std::borrow::Cow::Borrowed(name),
            description: std::borrow::Cow::Borrowed(description),
            parameters_schema: Some(parameters_schema),
        }
    }

    /// Adapt a `Box<dyn Tool>` slice into a `Vec<PromptTool<'_>>`.
    pub fn from_tools(tools: &'a [Box<dyn Tool>]) -> Vec<PromptTool<'a>> {
        Self::from_tool_refs(tools.iter().map(|t| t.as_ref()))
    }

    /// Adapt any iterator of borrowed tools into a `Vec<PromptTool<'_>>`.
    ///
    /// An agent's callable surface is not one contiguous slice: the durable
    /// registry and the freshly-synthesised delegation set live in separate
    /// `Arc`s (see `OpenHumanSessionHost::synthesized_tools`), and the prompt catalogue must
    /// render both. Taking an iterator lets the caller chain them without
    /// materialising a combined `Vec<Box<dyn Tool>>` — which is impossible
    /// anyway, since `Box<dyn Tool>` is not cloneable.
    pub fn from_tool_refs(tools: impl IntoIterator<Item = &'a dyn Tool>) -> Vec<PromptTool<'a>> {
        tools
            .into_iter()
            .map(|t| PromptTool {
                name: std::borrow::Cow::Borrowed(t.name()),
                description: std::borrow::Cow::Borrowed(t.description()),
                parameters_schema: Some(t.parameters_schema().to_string()),
            })
            .collect()
    }
}

/// Swap a prompt catalogue's `Deferred` entries for the discovery bridge.
///
/// On a TEXT dialect (P-Format / code) the catalogue this prompt renders IS
/// the model's callable surface: the harness folds it into the system prompt
/// and clears `request.tools`. Two things follow, and both were wrong before
/// this helper existed:
///
/// * **Deferred tools must leave the catalogue.** The filter each prompt site
///   used is the policy's allow-set, which deliberately admits deferred names
///   so a found tool stays *callable* (`reachable_names` in the session
///   builder). Filtering the prompt by it rendered every deferred schema into
///   the prompt — measured live at 107 connected Composio actions for 55 KB of
///   a 71 KB prompt, the exact cost deferral exists to avoid — and told the
///   model to search for an action whose signature it could already read.
///
/// * **The bridge must take their place.** The harness mints `tool_search`
///   onto `request.tools`, which a text dialect drops, and with
///   `host_renders_tool_catalogue` it appends nothing itself. Without this
///   entry the model has no signature for `tool_search`; observed live as a
///   turn that narrates the call it is about to make and then stops.
///
/// A native-tool-calling provider is unaffected: it reads `request.tools`,
/// where the harness already puts exactly this pair.
pub fn swap_deferred_for_discovery_bridge<'a>(
    prompt_tools: &mut Vec<PromptTool<'a>>,
    visible_tool_names: &mut std::collections::HashSet<String>,
    deferred_tool_names: &std::collections::HashSet<String>,
) {
    if deferred_tool_names.is_empty() {
        return;
    }
    visible_tool_names.retain(|name| !deferred_tool_names.contains(name));
    for bridge in
        crate::agent::tinyagents::discovery::bridge_prompt_tools(deferred_tool_names.len())
    {
        visible_tool_names.insert(bridge.name.to_string());
        prompt_tools.push(bridge);
    }
}

/// How TinyTools should render and parse an agent's tool calls.
///
/// The prompt layer carries this only to select the TinyTools dialect; it does
/// not define a tool-call protocol of its own.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallFormat {
    /// Compact positional legacy dialect.
    PFormat,
    /// JSON-in-tag rendering with full schemas. The default.
    #[default]
    Json,
    /// Provider supplies structured tool calls — catalogue is
    /// informational. Renders in the same JSON-schema form as `Json`.
    Native,
    /// Python `def` signatures; the model calls `name(arg="value")`.
    Python,
    /// TypeScript `function` signatures; the model calls `name({arg: "value"})`.
    TypeScript,
}

impl ToolCallFormat {
    /// The harness policy that speaks this format.
    ///
    /// `Native` maps to `Auto` rather than forcing native: the session only
    /// picks it when the provider profile supports native tools, and `Auto`
    /// resolves to the same thing while still letting the harness fall back
    /// for a model that turns out not to.
    pub(crate) fn harness_dispatcher(self) -> tinyagents_harness::config::ToolDispatcher {
        use tinyagents_harness::config::ToolDispatcher;
        match self {
            ToolCallFormat::PFormat => ToolDispatcher::Pformat,
            ToolCallFormat::Json => ToolDispatcher::Xml,
            ToolCallFormat::Native => ToolDispatcher::Auto,
            ToolCallFormat::Python => ToolDispatcher::Python,
            ToolCallFormat::TypeScript => ToolDispatcher::Typescript,
        }
    }

    /// The code style behind a code-call format, `None` for the others.
    pub(crate) fn code_style(self) -> Option<tinytools_agent::dialect::CodeStyle> {
        match self {
            ToolCallFormat::Python => Some(tinytools_agent::dialect::CodeStyle::Python),
            ToolCallFormat::TypeScript => Some(tinytools_agent::dialect::CodeStyle::TypeScript),
            ToolCallFormat::PFormat | ToolCallFormat::Json | ToolCallFormat::Native => None,
        }
    }
}

/// Map the canonical dialect's catalogue spelling onto the host prompt wire
/// vocabulary at the prompt boundary.
pub(crate) fn tool_call_format_from_dialect(
    format: tinytools_agent::dialect::ToolCallFormat,
) -> ToolCallFormat {
    match format {
        tinytools_agent::dialect::ToolCallFormat::PFormat => ToolCallFormat::PFormat,
        tinytools_agent::dialect::ToolCallFormat::Json => ToolCallFormat::Json,
        tinytools_agent::dialect::ToolCallFormat::Native => ToolCallFormat::Native,
        tinytools_agent::dialect::ToolCallFormat::Python => ToolCallFormat::Python,
        tinytools_agent::dialect::ToolCallFormat::TypeScript => ToolCallFormat::TypeScript,
    }
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
/// constructed from the stored `auth_set_credential` user payload in
/// `credentials::identity::peek_credential_user_identity`, which strips
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
    pub workflows: &'a [Workflow],
    pub dispatcher_instructions: &'a str,
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
    /// Authenticated user identity (id/name/email) when available — see
    /// [`UserIdentity`]. `None` for unauthenticated paths (CLI without a
    /// session, tests). Pre-fetched by the caller from the
    /// stored credential user payload so prompt builders never reach the network.
    pub user_identity: Option<UserIdentity>,
    /// Non-self personality roster entries for the master agent's prompt.
    /// Empty for non-master agents.
    pub personality_roster: Vec<PersonalityRosterEntry>,
    /// Pre-loaded global `AGENTS.md` content (`<workspace_dir>/AGENTS.md`),
    /// injected by [`crate::agent::prompts::sections::AgentsInstructionsSection`].
    /// `None` when the file is absent/empty or the `agents_md_enabled` config
    /// gate is off. Loaded once at system-prompt build time (never re-read per
    /// turn) so the frozen system-prompt prefix / KV-cache contract holds.
    pub agents_md_global: Option<String>,
    /// Pre-loaded project-layer `AGENTS.md` content — `<action_dir>/AGENTS.md`,
    /// or a sub-agent's `worktree_action_dir` override. `None` when the file is
    /// absent/empty, deduplicated against the global layer (same dir), or the
    /// gate is off. Rendered after the global layer.
    pub agents_md_local: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// PromptSection trait + rendered output
// ─────────────────────────────────────────────────────────────────────────────

pub trait PromptSection: Send + Sync {
    fn name(&self) -> &str;
    fn build(&self, ctx: &PromptContext<'_>) -> Result<String>;

    /// The section's bytes split by cache tier.
    ///
    /// Most sections live in exactly one tier, so the default is one part in
    /// [`Self::tier`]. A section whose body spans tiers (the orchestrator's
    /// dynamic builder renders identity, per-install context and the user's
    /// state in one pass) overrides this so the builder can place each slice
    /// with its peers instead of dragging the stable bytes into the volatile
    /// tail.
    fn build_parts(&self, ctx: &PromptContext<'_>) -> Result<Vec<(PromptTier, String)>> {
        Ok(vec![(self.tier(), self.build(ctx)?)])
    }

    /// Which cache tier this section's bytes belong to.
    ///
    /// Defaults to [`PromptTier::Stable`], which is right for the large
    /// majority: identity, role, rules, safety, grounding and style are the
    /// same bytes on every turn of every session. A section must override this
    /// only if its output can change — and then it **must**, because a volatile
    /// section rendered inside the stable tier invalidates every byte after it.
    fn tier(&self) -> PromptTier {
        PromptTier::Stable
    }
}

/// How stable a [`PromptSection`]'s bytes are, which decides where in the
/// assembled prompt they are emitted.
///
/// The prompt is frozen after turn 1 (`session/turn/core.rs`), so within a
/// session nothing here moves. The tiers matter *across* sessions and to
/// providers that must be told where to cache: a prefix is reusable only up to
/// the first byte that differs, so the ordering rule is simply "most stable
/// first". Put the user's memory near the front — as this builder did until
/// #5701's successor — and one memory write invalidates the identity, the
/// rules, the safety contract and the entire tool catalogue behind it.
///
/// Hermes reaches the same three-way split from the same reasoning
/// (`agent/system_prompt.py`'s `stable` / `context` / `volatile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PromptTier {
    /// Identical across sessions for a given build and agent: identity, role,
    /// delegation rules, safety, grounding, writing style.
    Stable,
    /// Stable for the life of a session but not across sessions — project
    /// instructions (`AGENTS.md`) and the resolved workspace.
    Context,
    /// Changes whenever the user's state does: memory, profile, the skills
    /// index, connected integrations, the clock.
    Volatile,
}

/// Marker a dynamic prompt builder emits on its own line to say "everything
/// after this belongs to the `Context` tier".
///
/// A [`PromptSource::Dynamic`](crate::agent::harness::definition::PromptSource)
/// builder returns one string. Splitting it on these markers is how it
/// declares tiers without a second builder signature, and the markers never
/// reach the model: [`split_prompt_tiers`] removes them, and a renderer that
/// bypasses the builder sees an HTML comment the model ignores.
pub const PROMPT_TIER_CONTEXT_MARKER: &str = "<!--prompt-tier:context-->";
/// Marker for the start of the `Volatile` tier. See [`PROMPT_TIER_CONTEXT_MARKER`].
pub const PROMPT_TIER_VOLATILE_MARKER: &str = "<!--prompt-tier:volatile-->";

/// Split a dynamic builder's body on the tier markers.
///
/// Text before the first marker is `default_tier` (the tier the section
/// declares); text after [`PROMPT_TIER_CONTEXT_MARKER`] is `Context` and text
/// after [`PROMPT_TIER_VOLATILE_MARKER`] is `Volatile`. Markers may appear in
/// either order and at most once each; empty slices are dropped.
#[must_use]
pub fn split_prompt_tiers(body: &str, default_tier: PromptTier) -> Vec<(PromptTier, String)> {
    let mut parts: Vec<(PromptTier, String)> = Vec::new();
    let mut tier = default_tier;
    let mut current = String::new();
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim();
        let next = if trimmed == PROMPT_TIER_CONTEXT_MARKER {
            Some(PromptTier::Context)
        } else if trimmed == PROMPT_TIER_VOLATILE_MARKER {
            Some(PromptTier::Volatile)
        } else {
            None
        };
        match next {
            Some(next_tier) => {
                if !current.trim().is_empty() {
                    parts.push((tier, std::mem::take(&mut current)));
                } else {
                    current.clear();
                }
                tier = next_tier;
            }
            None => current.push_str(line),
        }
    }
    if !current.trim().is_empty() {
        parts.push((tier, current));
    }
    parts
}

// ─────────────────────────────────────────────────────────────────────────────
// Sub-agent render options (per-definition flags)
// ─────────────────────────────────────────────────────────────────────────────

/// Per-definition rendering flags passed into the sub-agent prompt
/// renderer. Mirrors the `omit_*` fields on
/// [`crate::agent::harness::definition::AgentDefinition`]
/// but inverted into positive-sense `include_*` form.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubagentRenderOptions {
    pub include_safety_preamble: bool,
    pub include_identity: bool,
}

impl SubagentRenderOptions {
    /// Build the narrow default (every section off).
    pub fn narrow() -> Self {
        Self::default()
    }

    /// Construct from per-definition `omit_*` flags, inverting into the
    /// positive-sense `include_*` shape.
    pub fn from_definition_flags(omit_identity: bool, omit_safety_preamble: bool) -> Self {
        Self {
            include_identity: !omit_identity,
            include_safety_preamble: !omit_safety_preamble,
        }
    }
}
