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
pub(crate) const USER_MEMORY_PER_NAMESPACE_MAX_CHARS: usize = 8_000;

/// Hard ceiling across all namespaces, so a workspace with 30 namespaces
/// doesn't burn the entire context window. ~32 000 chars ≈ 8 000 tokens.
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
    /// Pre-fetched root-level summaries from the tree summarizer, one per
    /// namespace that has a root node on disk. Each entry is
    /// `(namespace, body)`. Empty when the tree summarizer hasn't run.
    pub tree_root_summaries: Vec<(String, String)>,
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
    /// Whether the user has an active OAuth connection for this
    /// toolkit. When `false`, the toolkit is in the backend allowlist
    /// but no authorization has been completed yet — `tools` is empty
    /// and the orchestrator must point the user at Settings instead of
    /// attempting to delegate.
    pub connected: bool,
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
// Prompt context (everything a section needs)
// ─────────────────────────────────────────────────────────────────────────────

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
