//! Data-driven agent definitions.
//!
//! An [`AgentDefinition`] fully specifies a sub-agent: its core prompt, model,
//! allowed tool set, runtime limits, and which sections of the parent system
//! prompt to omit. Built-in definitions are derived from
//! [`super::archetypes::AgentArchetype`] in
//! [`super::builtin_definitions`]; users can ship custom definitions as YAML
//! files under `$OPENHUMAN_WORKSPACE/agents/*.yaml` (or
//! `~/.openhuman/agents/*.yaml`) which override built-ins on id collision.
//!
//! Sub-agents are dispatched at runtime by the `spawn_subagent` tool, which
//! looks up an [`AgentDefinition`] by id in the global
//! [`AgentDefinitionRegistry`] and hands it to
//! [`super::subagent_runner::run_subagent`].
//!
//! This file intentionally has zero references to the rest of the agent
//! runtime — it is pure data so the model can be unit-tested in isolation
//! and serialised straight from disk.

use crate::openhuman::tools::ToolCategory;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ─────────────────────────────────────────────────────────────────────────────
// Agent definition
// ─────────────────────────────────────────────────────────────────────────────

/// A fully specified sub-agent: what it knows, what it can do, how to prompt it.
///
/// Built-ins live in [`super::builtin_definitions`]; custom ones load from
/// YAML at startup. The [`AgentDefinitionRegistry`] merges them and is the
/// single source of truth that `SpawnSubagentTool` queries.
///
/// All `omit_*` flags default to `true` for sub-agents — sub-agents are
/// narrow specialists and pay no token tax for the parent's identity,
/// memory, safety, or skills sections. Override per-archetype if a
/// section is needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    // ── identity ────────────────────────────────────────────────────────
    /// Unique id, referenced from `spawn_subagent { agent_id: "…" }`.
    /// Convention: snake_case (e.g. `code_executor`, `notion_specialist`).
    pub id: String,

    /// One-line description shown in the orchestrator's `spawn_subagent`
    /// tool schema so the parent model knows when to delegate to this agent.
    pub when_to_use: String,

    /// Optional display name for UI/logs. Falls back to `id`.
    #[serde(default)]
    pub display_name: Option<String>,

    // ── prompt ──────────────────────────────────────────────────────────
    /// Source of the sub-agent's core system prompt. Inline for YAML-defined
    /// agents, or a path to a file under `agent/prompts/` for built-ins.
    pub system_prompt: PromptSource,

    /// Sections of the main agent's prompt to strip when this sub-agent runs.
    /// Defaults to `true` (strip) — sub-agents are narrow and don't need the
    /// parent's identity scaffolding.
    #[serde(default = "defaults::true_")]
    pub omit_identity: bool,
    #[serde(default = "defaults::true_")]
    pub omit_memory_context: bool,
    #[serde(default = "defaults::true_")]
    pub omit_safety_preamble: bool,
    #[serde(default = "defaults::true_")]
    pub omit_skills_catalog: bool,

    // ── model ───────────────────────────────────────────────────────────
    /// Model selection: inherit parent, hint to router, or pinned name.
    #[serde(default)]
    pub model: ModelSpec,

    /// Sampling temperature. Sub-agents default to `0.4` for precision.
    #[serde(default = "defaults::subagent_temperature")]
    pub temperature: f64,

    // ── tools ───────────────────────────────────────────────────────────
    /// Either [`ToolScope::Wildcard`] (all tools the parent has) or
    /// [`ToolScope::Named`] (an explicit allowlist).
    #[serde(default)]
    pub tools: ToolScope,

    /// Tools that are explicitly banned even if `tools == Wildcard`.
    /// Built-ins default-deny dangerous ops for read-only archetypes.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,

    /// If set, the resolved tool list is further filtered to only those whose
    /// name starts with `{skill_filter}__`. Gives us per-API specialists
    /// (Notion, Gmail, …) without enum variants. Overridable per-spawn.
    #[serde(default)]
    pub skill_filter: Option<String>,

    /// If set, the resolved tool list is restricted to tools whose
    /// [`crate::openhuman::tools::Tool::category`] matches this value.
    /// This is the *primary* mechanism the orchestrator uses to spawn
    /// dedicated tool-execution sub-agents:
    /// - `Some(Skill)` → sub-agent only sees skill-bridge tools
    ///   (Notion, Gmail, Telegram, …). Pair with `ModelSpec::Hint("agentic")`
    ///   to route to the backend's agentic model.
    /// - `Some(System)` → sub-agent only sees built-in Rust tools.
    /// - `None` (default) → no category restriction; `tools` /
    ///   `disallowed_tools` / `skill_filter` still apply.
    ///
    /// Category filtering happens *before* the `tools`/`disallowed_tools`
    /// scope check, so a `Named` scope is a stricter-intersection override.
    #[serde(default)]
    pub category_filter: Option<ToolCategory>,

    // ── runtime limits ──────────────────────────────────────────────────
    /// Maximum tool-call iterations per spawn. Sub-agents default to a
    /// shorter cap than the parent to keep cost bounded.
    #[serde(default = "defaults::max_iterations")]
    pub max_iterations: usize,

    /// Hard wall-clock timeout per turn. `None` falls back to
    /// `tool_execution_timeout_secs`.
    #[serde(default)]
    pub timeout_secs: Option<u64>,

    /// `none` / `read_only` / `sandboxed`. Mirrors
    /// [`super::archetypes::AgentArchetype::sandbox_mode`].
    #[serde(default)]
    pub sandbox_mode: SandboxMode,

    /// If true, spawn runs in the background and the call returns
    /// immediately with a placeholder. Reserved — not yet wired in v1.
    #[serde(default)]
    pub background: bool,

    /// Marker: when true, the runner skips its normal prompt-building path
    /// and uses the parent's pre-rendered prompt + tool schemas + message
    /// prefix from the [`super::fork_context::ForkContext`] task-local.
    /// Only the synthetic built-in `fork` definition has this set.
    #[serde(default, skip_serializing_if = "is_false")]
    pub uses_fork_context: bool,

    // ── source bookkeeping ──────────────────────────────────────────────
    /// Where this definition came from. Filled in by the loader/builder;
    /// not deserialised from YAML.
    #[serde(skip)]
    pub source: DefinitionSource,
}

fn is_false(b: &bool) -> bool {
    !b
}

impl AgentDefinition {
    /// Display name with fallback to id.
    pub fn display_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.id)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Prompt source
// ─────────────────────────────────────────────────────────────────────────────

/// Where the sub-agent's core system prompt comes from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSource {
    /// Inline prompt string (custom YAML-defined agents).
    Inline(String),
    /// Relative path under the workspace's `prompts/` directory or under
    /// `src/openhuman/agent/prompts/` for built-ins. Resolved by the runner
    /// at spawn time.
    File { path: String },
}

// ─────────────────────────────────────────────────────────────────────────────
// Model spec
// ─────────────────────────────────────────────────────────────────────────────

/// Model selection for a sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelSpec {
    /// Use the parent agent's currently-selected model at spawn time.
    #[default]
    Inherit,
    /// Exact model name (e.g. `"neocortex-mk1"`).
    Exact(String),
    /// Router hint (e.g. `"reasoning"`, `"coding"`, `"local"`). Resolved
    /// to a real model by the routing provider.
    Hint(String),
}

impl ModelSpec {
    /// Resolve this spec into the model name string the provider expects.
    /// `parent_model` is the model the parent agent is using right now.
    pub fn resolve(&self, parent_model: &str) -> String {
        match self {
            Self::Inherit => parent_model.to_string(),
            Self::Exact(name) => name.clone(),
            Self::Hint(hint) => format!("hint:{hint}"),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool scope
// ─────────────────────────────────────────────────────────────────────────────

/// Which tools a sub-agent is allowed to call.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolScope {
    /// All tools the parent has (subject to `disallowed_tools` and
    /// `skill_filter`).
    #[default]
    Wildcard,
    /// An explicit allowlist of tool names. Names not present in the parent
    /// registry at spawn time are silently dropped (logged at debug).
    Named(Vec<String>),
}

// ─────────────────────────────────────────────────────────────────────────────
// Sandbox mode
// ─────────────────────────────────────────────────────────────────────────────

/// Sandbox mode for a sub-agent's tool execution. Mirrors the existing
/// [`super::archetypes::AgentArchetype::sandbox_mode`] string for now;
/// in the future this may map directly into a `SecurityPolicy` builder.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// No additional sandboxing beyond what the parent already enforces.
    #[default]
    None,
    /// Read-only — write/execute tools are filtered out.
    ReadOnly,
    /// Drop privileges, restrict filesystem (Landlock / Bubblewrap).
    Sandboxed,
}

// ─────────────────────────────────────────────────────────────────────────────
// Definition source
// ─────────────────────────────────────────────────────────────────────────────

/// Where an [`AgentDefinition`] was loaded from. Used for telemetry and
/// the `agent::list_definitions` RPC reply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", content = "path")]
pub enum DefinitionSource {
    /// Built-in derived from an [`super::archetypes::AgentArchetype`].
    #[default]
    Builtin,
    /// Loaded from a TOML file at the given absolute path.
    File(PathBuf),
}

// ─────────────────────────────────────────────────────────────────────────────
// Defaults module — referenced by `#[serde(default = ...)]`
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) mod defaults {
    pub(crate) fn true_() -> bool {
        true
    }

    pub(crate) fn subagent_temperature() -> f64 {
        0.4
    }

    pub(crate) fn max_iterations() -> usize {
        8
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Registry
// ─────────────────────────────────────────────────────────────────────────────

use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// In-memory registry of all known [`AgentDefinition`]s.
///
/// One singleton instance is initialised at startup via
/// [`AgentDefinitionRegistry::init_global`]. Built-ins are registered
/// unconditionally; custom YAML definitions (if a workspace is provided)
/// are loaded next and override built-ins on `id` collision.
#[derive(Debug, Default)]
pub struct AgentDefinitionRegistry {
    by_id: HashMap<String, AgentDefinition>,
    /// Insertion-stable order for predictable `list()` output.
    order: Vec<String>,
}

static GLOBAL: OnceLock<AgentDefinitionRegistry> = OnceLock::new();

impl AgentDefinitionRegistry {
    /// Build a registry containing only the built-in definitions
    /// (no YAML loading). Useful for tests.
    pub fn builtins_only() -> Self {
        let mut reg = Self::default();
        for def in super::builtin_definitions::all() {
            reg.insert(def);
        }
        reg
    }

    /// Build a registry containing built-ins plus any custom YAML
    /// definitions found under `<workspace>/agents/*.yaml` (and the
    /// `~/.openhuman/agents/*.yaml` fallback).
    pub fn load(workspace: &Path) -> Result<Self> {
        let mut reg = Self::builtins_only();
        let custom = super::definition_loader::load_from_workspace(workspace)?;
        for def in custom {
            tracing::info!(
                id = %def.id,
                source = ?def.source,
                "[agent_defs] loaded custom definition (overrides any built-in with the same id)"
            );
            reg.insert(def);
        }
        Ok(reg)
    }

    /// Insert (or replace) a definition by id.
    pub fn insert(&mut self, def: AgentDefinition) {
        let id = def.id.clone();
        if self.by_id.insert(id.clone(), def).is_none() {
            self.order.push(id);
        }
    }

    /// Look up a definition by id.
    pub fn get(&self, id: &str) -> Option<&AgentDefinition> {
        self.by_id.get(id)
    }

    /// All definitions, in insertion order.
    pub fn list(&self) -> Vec<&AgentDefinition> {
        self.order
            .iter()
            .filter_map(|id| self.by_id.get(id))
            .collect()
    }

    /// Number of registered definitions.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// True when the registry has no definitions.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    // ── singleton API ──────────────────────────────────────────────────

    /// Initialise the global registry. Subsequent calls are no-ops (the
    /// `OnceLock` only fires once); use [`Self::reload_global`] to refresh
    /// custom definitions during development.
    pub fn init_global(workspace: &Path) -> Result<()> {
        let registry = Self::load(workspace)?;
        match GLOBAL.set(registry) {
            Ok(()) => {
                tracing::info!(
                    "[agent_defs] global registry initialised with {} definitions",
                    GLOBAL.get().map(|r| r.len()).unwrap_or(0)
                );
                Ok(())
            }
            Err(_) => {
                tracing::debug!("[agent_defs] global registry already initialised; ignoring");
                Ok(())
            }
        }
    }

    /// Initialise the global registry with builtins only (no workspace
    /// scan). Used by tests and by callers that don't have a workspace.
    pub fn init_global_builtins() -> Result<()> {
        let registry = Self::builtins_only();
        let _ = GLOBAL.set(registry);
        Ok(())
    }

    /// Borrow the global registry, if initialised.
    pub fn global() -> Option<&'static Self> {
        GLOBAL.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_def(id: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.into(),
            when_to_use: "test".into(),
            display_name: None,
            system_prompt: PromptSource::Inline("system".into()),
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            omit_skills_catalog: true,
            model: ModelSpec::Inherit,
            temperature: 0.4,
            tools: ToolScope::Wildcard,
            disallowed_tools: vec![],
            skill_filter: None,
            category_filter: None,
            max_iterations: 8,
            timeout_secs: None,
            sandbox_mode: SandboxMode::None,
            background: false,
            uses_fork_context: false,
            source: DefinitionSource::Builtin,
        }
    }

    #[test]
    fn registry_insert_and_lookup() {
        let mut reg = AgentDefinitionRegistry::default();
        reg.insert(make_def("alpha"));
        reg.insert(make_def("beta"));
        assert_eq!(reg.len(), 2);
        assert!(reg.get("alpha").is_some());
        assert!(reg.get("beta").is_some());
        assert!(reg.get("missing").is_none());
    }

    #[test]
    fn registry_replace_preserves_order() {
        let mut reg = AgentDefinitionRegistry::default();
        reg.insert(make_def("alpha"));
        reg.insert(make_def("beta"));
        let mut updated = make_def("alpha");
        updated.when_to_use = "replaced".into();
        reg.insert(updated);

        let list: Vec<&str> = reg.list().iter().map(|d| d.id.as_str()).collect();
        assert_eq!(list, vec!["alpha", "beta"]);
        assert_eq!(reg.get("alpha").unwrap().when_to_use, "replaced");
    }

    #[test]
    fn model_spec_resolve_inherit_uses_parent() {
        let spec = ModelSpec::Inherit;
        assert_eq!(spec.resolve("parent-model"), "parent-model");
    }

    #[test]
    fn model_spec_resolve_exact_uses_name() {
        let spec = ModelSpec::Exact("kimi-k2".into());
        assert_eq!(spec.resolve("parent-model"), "kimi-k2");
    }

    #[test]
    fn model_spec_resolve_hint_prefixes_router_marker() {
        let spec = ModelSpec::Hint("coding".into());
        assert_eq!(spec.resolve("parent-model"), "hint:coding");
    }

    #[test]
    fn display_name_falls_back_to_id() {
        let def = make_def("alpha");
        assert_eq!(def.display_name(), "alpha");
        let mut def2 = make_def("beta");
        def2.display_name = Some("Beta Specialist".into());
        assert_eq!(def2.display_name(), "Beta Specialist");
    }
}
