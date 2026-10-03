//! The [`AgentDefinition`] record itself plus the small policy enums it
//! embeds directly ([`IterationPolicy`]) and the
//! serde default helpers its `#[serde(default = ...)]` attributes name.

use serde::{Deserialize, Serialize};

use super::execution_spec::{ModelSpec, SandboxMode, ToolScope};
use super::prompt_source::PromptSource;
use super::source::DefinitionSource;
use super::subagents::{deserialize_subagent_entries, SubagentEntry};
use super::tier::AgentTier;
use crate::inference::tokenjuice::AgentTokenjuiceCompression;

/// Iteration ceiling for an [`IterationPolicy::Extended`] agent — the higher
/// bound a long-running agent (orchestrator, deep research) is allowed to reach
/// before the harness stops it. Lives here, the sole consumer, since the legacy
/// `tool_loop` that originally defined it was removed in the tinyagents
/// migration (issue #4249).
pub const EXTENDED_MAX_TOOL_ITERATIONS: usize = 50;

/// Iteration-cap policy for a sub-agent.
///
/// Controls how the harness enforces [`AgentDefinition::max_iterations`]:
///
/// * **Strict** — hard-fail at `max_iterations` (the current default).
///   Right for short-running agents (summarizer, triage) where hitting
///   the cap signals a likely loop.
/// * **Extended** — the per-agent `max_iterations` is replaced at runtime
///   by a higher harness-wide constant
///   ([`EXTENDED_MAX_TOOL_ITERATIONS`])
///   so the agent can complete realistic multi-tool workflows. The
///   repeated-failure circuit breaker and cost budget still apply. The
///   UI omits the denominator ("step N" instead of "turn N/M") to avoid
///   a misleading terminal countdown.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum IterationPolicy {
    /// Hard cap at `max_iterations`. Default for most agents.
    #[default]
    Strict,
    /// Raised cap for multi-step specialists. Guards still apply.
    Extended,
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent definition
// ─────────────────────────────────────────────────────────────────────────────

/// A fully specified sub-agent archetype: what it knows, what it can do, and how to prompt it.
///
/// Definitions are used by the `spawn_subagent` tool to initialize a new
/// specialized agent. They can be built-in or loaded from custom TOML files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    // ── identity ────────────────────────────────────────────────────────
    /// Unique identifier for this archetype (e.g., `planner`, `code_executor`).
    pub id: String,

    /// Human-readable description explaining when this agent should be used.
    /// Shown to the parent model to help it decide whether to delegate.
    pub when_to_use: String,

    /// Optional display name for UI and log output.
    #[serde(default)]
    pub display_name: Option<String>,

    // ── prompt ──────────────────────────────────────────────────────────
    /// The core system prompt body for this specialized agent.
    #[serde(default = "defaults::empty_inline_prompt")]
    pub system_prompt: PromptSource,

    /// If `true`, the parent's identity section is stripped from the prompt.
    #[serde(default = "defaults::true_")]
    pub omit_identity: bool,

    /// If `true`, the compiled memory context (`context.md`) is not injected
    /// as the first user message of a new session for this agent.
    #[serde(default = "defaults::true_")]
    pub omit_memory_context: bool,

    /// If `true`, the standard safety preamble is stripped.
    #[serde(default = "defaults::true_")]
    pub omit_safety_preamble: bool,

    // ── model ───────────────────────────────────────────────────────────
    /// Strategy for picking which model to use for this sub-agent.
    #[serde(default)]
    pub model: ModelSpec,

    /// Sampling temperature for the model.
    #[serde(default = "defaults::subagent_temperature")]
    pub temperature: f64,

    // ── tools ───────────────────────────────────────────────────────────
    /// Which tools from the parent's registry should be available to the sub-agent.
    #[serde(default)]
    pub tools: ToolScope,

    /// Explicit list of tool names to block, even if they match the scope.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,

    /// Filter to only tools belonging to a specific skill (e.g., `notion`).
    #[serde(default)]
    pub skill_filter: Option<String>,

    /// Named tools that should always be visible to this agent in
    /// addition to its [`ToolScope`]. Historically this was a bypass
    /// list for the now-removed `category_filter`; kept as a generic
    /// "also include these" hook for custom definitions.
    ///
    /// Entries are still subject to [`AgentDefinition::disallowed_tools`].
    #[serde(default)]
    pub extra_tools: Vec<String>,

    /// Tools in this agent's scope that should leave its wire and be served
    /// through `tool_search` instead: still registered, found by a search,
    /// and callable by their own name. Unlike `ToolExposure::Deferred`, which
    /// defers a tool for every agent, this defers it for this agent only.
    /// Takes effect only on a belt that opted into discovery (a wildcard
    /// belt, or `named` listing `tool_search`).
    #[serde(default)]
    pub deferred_tools: Vec<String>,

    // ── runtime limits ──────────────────────────────────────────────────
    /// Maximum number of tool iterations for this sub-agent's task.
    #[serde(default = "defaults::max_iterations")]
    pub max_iterations: usize,

    /// Iteration-cap policy. See [`IterationPolicy`] for semantics.
    /// Defaults to [`IterationPolicy::Strict`]; long-running specialists
    /// set `iteration_policy = "extended"` in their `agent.toml`.
    #[serde(default)]
    pub iteration_policy: IterationPolicy,

    /// Maximum character length for this sub-agent's output before the
    /// harness truncates it before feeding it back as a tool result to the
    /// parent. `None` means no cap (the default for most agents). Set to
    /// a value for research/planner/code agents to prevent context flooding
    /// from large outputs.
    #[serde(default)]
    pub max_result_chars: Option<usize>,

    /// Optional per-LLM-call output token cap for this agent. When unset, the
    /// shared agent-turn cap is used. Narrow agents can set a smaller cap so
    /// a single verbose turn cannot flood the sub-agent loop before the final
    /// result is truncated.
    #[serde(default)]
    pub max_turn_output_tokens: Option<u32>,

    /// Wall-clock timeout for the sub-agent's execution (seconds).
    #[serde(default)]
    pub timeout_secs: Option<u64>,

    /// Sandbox level for tool execution.
    #[serde(default)]
    pub sandbox_mode: SandboxMode,

    /// Reserved for background (asynchronous) execution support.
    #[serde(default)]
    pub background: bool,

    /// Per-agent TokenJuice tool-result compression profile.
    ///
    /// `auto` keeps compression on for normal agents, but resolves coding-model
    /// agents to `light` so CCR-backed lossy compression does not replace raw
    /// build/test/diff/search text that coding agents often need exactly.
    #[serde(default)]
    pub tokenjuice_compression: AgentTokenjuiceCompression,

    // ── delegation surface ─────────────────────────────────────────────
    /// Subagents this agent is allowed to spawn via synthesised
    /// `delegate_*` tools. Each entry expands at agent-build time into
    /// one tool the LLM can call in its function-calling schema:
    ///
    /// * [`SubagentEntry::AgentId`] — one [`ArchetypeDelegationTool`]
    ///   whose name defaults to `delegate_{agent_id}` (or the target
    ///   agent's `delegate_name` override) and whose description is the
    ///   target agent's [`AgentDefinition::when_to_use`].
    ///
    /// * [`SubagentEntry::Skills`] — no delegation tool. The connected
    ///   Composio toolkits' actions join this agent's `Deferred` catalogue
    ///   (reached through `tool_search`, called directly), and the entry
    ///   admits no sub-agent id: see [`AgentDefinition::allowed_subagent_ids`].
    ///
    /// `subagents` is intentionally separate from [`AgentDefinition::tools`]
    /// so that reading a TOML makes the distinction obvious: `tools` is
    /// "what I execute directly", `subagents` is "what I can delegate to".
    ///
    /// [`ArchetypeDelegationTool`]: crate::agent::orchestration::tools::ArchetypeDelegationTool
    #[serde(default, deserialize_with = "deserialize_subagent_entries")]
    pub subagents: Vec<SubagentEntry>,

    /// Optional override for the tool name this agent is exposed as when
    /// another agent lists it in its [`subagents`]. Defaults to
    /// `delegate_{id}` when absent. Kept separate from `display_name` so
    /// the UI display and the LLM tool name can diverge (e.g.
    /// `display_name = "Researcher"`, `delegate_name = "research"`).
    #[serde(default)]
    pub delegate_name: Option<String>,

    // ── spawn hierarchy ────────────────────────────────────────────────
    /// Tier this archetype occupies in the spawn hierarchy
    /// (`chat` → `reasoning` → `worker`). Drives loader-time validation
    /// of [`AgentDefinition::subagents`] and runtime depth gating in the
    /// sub-agent runner. Defaults to [`AgentTier::Worker`] so existing
    /// specialists fit the "leaf" role without per-file edits.
    ///
    /// **Hierarchy contract** (enforced by
    /// [`super::super::agents::loader`] at registry build time):
    ///
    /// * `Chat` MUST NOT list another `Chat` agent in `subagents`. The
    ///   user-facing fast tier is a leaf in its own dimension — it
    ///   hands off to `Reasoning` or `Worker`, never to itself.
    /// * `Reasoning` MUST NOT list another `Reasoning` agent in
    ///   `subagents`. Reasoning composes downward into `Worker`s.
    /// * `Worker` MUST NOT list open-ended subagents. Workers execute;
    ///   they do not orchestrate.
    /// * `{ skills = "*" }` entries admit no sub-agent, so they are always
    ///   allowed.
    ///
    /// Combined with the harness's `MAX_SPAWN_DEPTH = 3` task-local
    /// gate, this means any execution chain bottoms out within three
    /// hops: `chat → reasoning → worker` (or `chat → worker` for the
    /// fast path).
    #[serde(default)]
    pub agent_tier: AgentTier,

    // ── source bookkeeping ──────────────────────────────────────────────
    /// Tracks where the definition was loaded from (Builtin vs. File).
    #[serde(skip)]
    pub source: DefinitionSource,

    // ── turn graph ──────────────────────────────────────────────────────
    /// How this agent's turn is driven (issue #4249). Injected post-load from
    /// the agent folder's `graph.rs::graph()` (mirrors how
    /// [`PromptSource::Dynamic`] is injected from `prompt.rs::build`); TOML-
    /// authored agents cannot set it, so it is `#[serde(skip)]` and defaults to
    /// [`AgentGraph::Default`] (the shared default turn graph).
    #[serde(skip, default)]
    pub graph: crate::agent::harness::agent_graph::AgentGraph,
}

impl AgentDefinition {
    /// The agent ids this definition may spawn, derived from
    /// [`AgentDefinition::subagents`]. Only [`SubagentEntry::AgentId`]
    /// entries admit a target; the `{ skills = "*" }` wildcard admits none —
    /// a chat agent searches for and calls an integration action itself. The runner's spawn gate (`parent.allowed_subagent_ids`) reads
    /// this, so a definition without a bare id for an agent cannot reach it
    /// through `spawn_async_subagent` either.
    pub fn allowed_subagent_ids(&self) -> Vec<String> {
        self.subagents
            .iter()
            .filter_map(|entry| match entry {
                SubagentEntry::AgentId(id) => Some(id.clone()),
                SubagentEntry::Skills(_) => None,
            })
            .collect()
    }

    /// Display name with fallback to id.
    pub fn display_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.id)
    }

    /// Effective iteration cap after applying [`IterationPolicy`].
    ///
    /// * `Strict` → `self.max_iterations` unchanged.
    /// * `Extended` → the higher of `self.max_iterations` and the
    ///   harness-wide [`EXTENDED_MAX_TOOL_ITERATIONS`].
    pub fn effective_max_iterations(&self) -> usize {
        match self.iteration_policy {
            IterationPolicy::Strict => self.max_iterations,
            IterationPolicy::Extended => self.max_iterations.max(EXTENDED_MAX_TOOL_ITERATIONS),
        }
    }

    /// Resolve the authored TokenJuice profile to the concrete per-call policy.
    pub fn effective_tokenjuice_compression(&self) -> AgentTokenjuiceCompression {
        match self.tokenjuice_compression {
            AgentTokenjuiceCompression::Auto => match &self.model {
                ModelSpec::Hint(hint) if hint.trim().eq_ignore_ascii_case("coding") => {
                    AgentTokenjuiceCompression::Light
                }
                _ => AgentTokenjuiceCompression::Full,
            },
            other => other,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Defaults module — referenced by `#[serde(default = ...)]`
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) mod defaults {
    use super::PromptSource;

    pub(crate) fn true_() -> bool {
        true
    }

    pub(crate) fn subagent_temperature() -> f64 {
        0.4
    }

    pub(crate) fn max_iterations() -> usize {
        8
    }

    /// Placeholder for [`super::AgentDefinition::system_prompt`] when the
    /// TOML omits the field. The built-in loader overwrites this with
    /// the rendered sibling `prompt.md`; custom TOMLs that omit the
    /// field get a no-op empty prompt (and should not).
    pub(crate) fn empty_inline_prompt() -> PromptSource {
        PromptSource::Inline(String::new())
    }
}
