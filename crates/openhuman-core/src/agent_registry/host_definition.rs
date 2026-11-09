//! Host side of the agent-definition vocabulary.
//!
//! The portable record (`AgentDefinition<X>`), its model/tool/tier/iteration
//! vocabulary, the TOML loader and the layered [`DefinitionSet`] live in
//! `tinyagents-definition`. OpenHuman keeps only what it owns here: the
//! [`AgentExt`] extension slot (prompt omissions, TokenJuice profile, the MCP
//! flag and the runtime-only prompt builder), the process-wide and per-context
//! [`AgentDefinitionRegistry`], and path resolution for user definitions
//! (`<workspace>/agents`, `$OPENHUMAN_HOME/agents`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tinyagents_definition::{DefinitionSet, DefinitionSource, DefinitionValidation, ModelSpec};
use tinyjuice_bus::types::AgentTokenjuiceCompression;

#[cfg(test)]
#[path = "host_definition_tests.rs"]
mod tests;

/// Sentinel used to represent an explicit zero-tool scope.
pub const NO_TOOLS_SENTINEL: &str = "__no_tools__";

/// Returns whether a visible-tool set represents an explicit zero-tool scope.
pub fn is_empty_tool_scope(visible: &std::collections::HashSet<String>) -> bool {
    visible.is_empty() || (visible.len() == 1 && visible.contains(NO_TOOLS_SENTINEL))
}

/// Builder function for a function-driven system prompt. Built-in agents only;
/// the portable definition carries an inert `PromptSource::Dynamic(())` marker
/// and the executable builder rides in [`AgentExt::prompt_builder`].
pub type PromptBuilder = fn(&crate::agent::prompts::PromptContext<'_>) -> anyhow::Result<String>;

/// An agent definition with OpenHuman's host fields.
pub type AgentDefinition = tinyagents_definition::AgentDefinition<AgentExt>;

fn true_() -> bool {
    true
}

/// OpenHuman-only authored and runtime fields of an [`AgentDefinition`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExt {
    /// Strip the parent's identity section from the prompt.
    #[serde(default = "true_")]
    pub omit_identity: bool,
    /// Do not inject the compiled memory context as the first user message.
    #[serde(default = "true_")]
    pub omit_memory_context: bool,
    /// Strip the standard safety preamble.
    #[serde(default = "true_")]
    pub omit_safety_preamble: bool,
    /// Per-agent TokenJuice tool-result compression profile. `auto` keeps
    /// compression on for normal agents and resolves coding-model agents to
    /// `light`; see [`AgentExt::tokenjuice_for`].
    #[serde(default)]
    pub tokenjuice_compression: AgentTokenjuiceCompression,
    /// Whether this agent's threads take up the prompt it renders now when
    /// they resume, rather than replay the one recorded on their transcript.
    ///
    /// Off by default: a resumed thread reuses its recorded system messages
    /// verbatim, which keeps the provider's prefix cache warm across a
    /// restart, so prompt edits reach only new threads. On, the prompt is
    /// rendered again on every resumed turn, and a thread whose recorded
    /// prompt differs continues under the current one with its conversation
    /// intact: the session seals the transcript generation it was on and
    /// carries the conversation into the next, and that turn's user message
    /// opens with a one-time note that the instructions changed, so the model
    /// does not hold to what it said under the old ones. An unchanged prompt
    /// writes nothing. A composed prompt also refreshes when the live state it
    /// renders changes (connected integrations, skills, AGENTS.md), never
    /// before the session has read its integrations.
    #[serde(default)]
    pub refresh_prompt_on_resume: bool,
    /// A memory block at the end of the system prompt: what long-term
    /// memory knows about the user, rendered when a transcript generation
    /// starts and re-sent verbatim until the next one
    /// (`agent/session_host/runtime_session/memory_block.rs`). `None` sends
    /// none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_block: Option<MemoryBlock>,
    /// Whether this agent's turns can search and call the tools of every
    /// connected MCP server (deferred, through `tool_search`).
    #[serde(default)]
    pub searches_connected_mcp: bool,
    /// Function-driven system prompt, set with `PromptSource::Dynamic(())`.
    /// Runtime only: never serialised and not authorable in TOML.
    #[serde(skip)]
    pub prompt_builder: Option<PromptBuilder>,
    /// Original builtin archetype whose prompt is loaded through TinyFlows.
    /// Runtime only: actor ids may change when an embedder clones a builtin.
    #[serde(skip)]
    pub tinyflows_prompt_owner: Option<String>,
}

/// The memory block an agent's system prompt ends with; see
/// [`AgentExt::memory_block`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryBlock {
    /// The most tokens the block may take, markers and instruction included.
    pub max_tokens: u32,
}

impl Default for AgentExt {
    fn default() -> Self {
        Self {
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            tokenjuice_compression: AgentTokenjuiceCompression::default(),
            refresh_prompt_on_resume: false,
            memory_block: None,
            searches_connected_mcp: false,
            prompt_builder: None,
            tinyflows_prompt_owner: None,
        }
    }
}

impl AgentExt {
    /// Resolve the authored TokenJuice profile to the concrete per-call policy
    /// for an agent whose model pin is `model`.
    pub fn tokenjuice_for(&self, model: &ModelSpec) -> AgentTokenjuiceCompression {
        match self.tokenjuice_compression {
            AgentTokenjuiceCompression::Auto => match model {
                ModelSpec::Hint(hint) if hint.trim().eq_ignore_ascii_case("coding") => {
                    AgentTokenjuiceCompression::Light
                }
                _ => AgentTokenjuiceCompression::Full,
            },
            other => other,
        }
    }
}

/// Whether two definitions serialise identically, which covers every authored
/// field (the process-local `source` and `prompt_builder` are not serialised).
fn same_definition(a: &AgentDefinition, b: &AgentDefinition) -> bool {
    match (serde_json::to_value(a), serde_json::to_value(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Host accessor on the portable [`AgentDefinition`].
pub trait AgentDefinitionExt {
    /// The prompt builder behind a `PromptSource::Dynamic(())` marker.
    fn prompt_fn(&self) -> Result<PromptBuilder>;
}

impl AgentDefinitionExt for AgentDefinition {
    fn prompt_fn(&self) -> Result<PromptBuilder> {
        self.extensions.prompt_builder.ok_or_else(|| {
            anyhow::anyhow!(
                "agent `{}` declares a dynamic prompt but has no builder",
                self.id
            )
        })
    }
}

/// Directories scanned for user definitions, in override order: the workspace
/// `agents/` folder, then `$OPENHUMAN_HOME/agents` (or `~/.openhuman/agents`).
fn user_definition_dirs(workspace: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![workspace.join("agents")];
    if let Some(home) = user_home_agents_dir() {
        dirs.push(home);
    }
    dirs
}

fn user_home_agents_dir() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("OPENHUMAN_HOME") {
        return Some(PathBuf::from(custom).join("agents"));
    }
    match crate::config::default_root_openhuman_dir() {
        Ok(dir) => Some(dir.join("agents")),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "[agent-definition-loader] resolving root openhuman dir failed"
            );
            None
        }
    }
}

/// In-memory registry of all known [`AgentDefinition`]s.
///
/// One singleton instance is initialised at startup via
/// [`AgentDefinitionRegistry::init_global`]. Built-ins are registered
/// unconditionally; custom TOML definitions (if a workspace is provided) are
/// loaded next and override built-ins on `id` collision.
#[derive(Debug, Default)]
pub struct AgentDefinitionRegistry {
    set: DefinitionSet<AgentExt>,
}

static GLOBAL: OnceLock<Arc<AgentDefinitionRegistry>> = OnceLock::new();

impl AgentDefinitionRegistry {
    /// Build a registry containing only the built-in definitions.
    pub fn builtins_only() -> Self {
        let mut reg = Self::default();
        for def in crate::agent_registry::agents::load_builtins()
            .expect("built-in agent TOML must always parse")
        {
            reg.insert(def);
        }
        #[cfg(test)]
        for def in [
            crate::agent::harness::test_support::test_main_def(),
            crate::agent::harness::test_support::test_inherit_echo_def(),
            crate::agent::harness::test_support::test_inherit_parallel_worker_def(),
        ] {
            reg.insert(def);
        }
        reg
    }

    /// Built-ins plus any custom TOML definitions found under
    /// `<workspace>/agents/*.toml` and the user-global agents directory.
    /// Custom definitions override built-ins on `id` collision; files that
    /// fail to parse are logged and skipped, and the merged spawn hierarchy is
    /// re-validated.
    pub fn load(workspace: &Path) -> Result<Self> {
        let mut reg = Self::builtins_only();
        let dirs = user_definition_dirs(workspace);
        let dir_refs: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
        reg.set
            .load_dirs(&dir_refs, DefinitionValidation::NonemptyInlinePrompt)
            .map_err(|e| {
                anyhow::anyhow!(
                    "agent registry rejected after merging workspace overrides from {}: {}",
                    workspace.display(),
                    e
                )
            })?;
        Ok(reg)
    }

    /// Resolve the default workspace via [`crate::config::Config::load_or_init`]
    /// and load from it. Must NOT be called from an existing tokio runtime.
    pub async fn load_for_default_workspace() -> Result<Self> {
        let config = crate::config::Config::load_or_init().await?;
        Self::load(&config.workspace_dir)
    }

    /// Insert (or replace) a definition by id.
    pub fn insert(&mut self, def: AgentDefinition) {
        self.set.insert(def);
    }

    /// Look up a definition by id.
    pub fn get(&self, id: &str) -> Option<&AgentDefinition> {
        self.set.get(id)
    }

    /// All definitions, in insertion order.
    pub fn list(&self) -> Vec<&AgentDefinition> {
        self.set.list()
    }

    /// Number of registered definitions.
    pub fn len(&self) -> usize {
        self.set.len()
    }

    /// True when the registry has no definitions.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Whether this registry is exactly the built-in set: the same ids and
    /// contents as [`Self::builtins_only`], and no definition loaded from a
    /// file or synthesized from a user entry. SaaS boot refuses anything else.
    pub fn holds_builtins_only(&self) -> bool {
        let builtins = Self::builtins_only();
        self.len() == builtins.len()
            && self.list().iter().all(|def| {
                def.source == DefinitionSource::Builtin
                    && builtins
                        .get(&def.id)
                        .is_some_and(|builtin| same_definition(def, builtin))
            })
    }

    /// Initialise the global registry. Subsequent calls are no-ops.
    pub fn init_global(workspace: &Path) -> Result<()> {
        let registry = Self::load(workspace)?;
        match GLOBAL.set(Arc::new(registry)) {
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

    /// Initialise the global registry with builtins only (no workspace scan).
    pub fn init_global_builtins() -> Result<()> {
        let _ = GLOBAL.set(Arc::new(Self::builtins_only()));
        Ok(())
    }

    /// Borrow the global registry, if initialised.
    pub fn global() -> Option<&'static Self> {
        GLOBAL.get().map(Arc::as_ref)
    }

    /// Clones the durable process registry for a host capability bundle.
    pub fn global_arc() -> Option<Arc<Self>> {
        GLOBAL.get().cloned()
    }

    /// Resolve definitions from an explicitly supplied core before falling
    /// back to the process registry. Execution paths with a live parent must
    /// pass that parent's core so concurrent tenant/run contexts cannot read
    /// another invocation's ambient catalogue.
    pub fn for_core_context(
        core: Option<Arc<crate::core::runtime::CoreContext>>,
    ) -> Option<Arc<Self>> {
        core.and_then(|core| core.definitions())
            .or_else(Self::global_arc)
    }

    /// The catalogue sub-agent lookups resolve through: the ambient agent
    /// context's own registry when it carries one, else the process registry.
    pub fn current() -> Option<Arc<Self>> {
        crate::core::runtime::CoreContext::current()
            .and_then(|ctx| ctx.definitions())
            .or_else(Self::global_arc)
    }

    /// A copy of this registry with `definitions` added, replacing any
    /// definition with the same id.
    pub fn with_definitions(&self, definitions: impl IntoIterator<Item = AgentDefinition>) -> Self {
        Self {
            set: self.set.with_definitions(definitions),
        }
    }
}
