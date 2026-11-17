//! Host implementation of the TinyAgents **agent catalogue** seam.
//!
//! Adapts two OpenHuman domains onto
//! [`tinyagents_harness::host::DefinitionRegistry`]:
//!
//! * [`crate::agent_registry::host_definition::AgentDefinitionRegistry`] —
//!   the harness registry of built-in ([`load_builtins`](crate::agent_registry::agents::load_builtins)-parsed
//!   `agent.toml`)
//!   and workspace-override agent definitions, plus the config-backed
//!   user-authored fallback
//!   ([`crate::agent_registry::find_custom_in_config`] →
//!   [`crate::agent_registry::definition_from_registry_entry`]).
//!
//! This is `docs/specs/plan-agents.md` Phase 4. The crate-side
//! [`AgentDefinition`] is inert (`serde` + `std`); OpenHuman's harness
//! definition is far richer (prompt builders, sandbox mode, iteration policy,
//! TokenJuice profile, turn graph). Only the six fields the turn loop can act
//! on cross the seam; everything else stays host-side by design.
//!
//! # Contract mismatches resolved here
//!
//! **1. Absence must never be an error.** OpenHuman's own catalogue relies on
//! this: an agent may list a subagent id that a feature gate compiled out of
//! the build (`presentation_agent` without `documents`), and both existing resolution sites
//! tolerate it (`collect_orchestrator_tools` warns and skips;
//! `DefinitionSet::validate_tier_hierarchy` explicitly continues past unknown ids). So
//! `OpenHumanDefinitionRegistry::resolve` returns `Ok(None)` for every miss
//! and this adapter has no error path at all.
//!
//! **2. Declared vs authorized subagents.** The crate's
//! `AgentDefinition::subagents` is only what an agent *declares*;
//! `delegates_for` must return the **authorized** set. OpenHuman's authority is
//! the tier hierarchy, so `delegates_for` applies
//! [`validate_tier_transition`](tinyagents_definition::validate_tier_transition)
//! — the same single source of truth
//! `DefinitionSet::validate_tier_hierarchy` walks at boot — per declared pair. A `Worker`
//! parent yields an empty list; a tier-illegal child is dropped. Unknown child
//! ids are **kept**, matching the owner's validator and
//! the trait's note that an authorized id may still fail to resolve.
//!
//! **3. Tool scopes remain host-materialized.** The portable definition
//! carries an explicit named scope, after the host applies discovery,
//! companion-tool and denylist policy. Wildcards are materialized against
//! [`Self::with_registered_tools`] and fail closed when no catalogue exists.
//! Empty scopes retain [`NO_TOOLS_SENTINEL`] for compatibility with the host's
//! existing tool visibility contract; they never become portable wildcards.
//!
//! **4. `SubagentEntry::Skills` entries are omitted.** A `{ skills = "*" }`
//! entry is not an agent id — it expands to the connected integrations'
//! actions as searchable tools. Emitting a synthetic id here would invent a
//! delegate the host never authorized.
//!

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_definition::Result;
use tinyagents_definition::{AgentDefinition, DefinitionRegistry};

use crate::agent_registry::host_definition::NO_TOOLS_SENTINEL;
use crate::agent_registry::host_definition::{
    AgentDefinition as HostAgentDefinition, AgentDefinitionRegistry,
};
use crate::agent_registry::{definition_from_registry_entry, find_custom_in_config};
use crate::config::Config;
use tinyagents_definition::{AgentTier, ToolScope};

// ── Registry handle ───────────────────────────────────────────────────────────

/// Where this adapter reads harness definitions from.
///
/// [`AgentDefinitionRegistry`] is neither `Clone` nor cheaply rebuildable, and
/// the process-wide singleton is handed out as a `&'static`, so the two
/// realistic ownership shapes are modelled explicitly rather than forcing a
/// copy at construction.
enum RegistryHandle {
    /// A registry this adapter shares ownership of (tests, embeddings, an
    /// explicitly-loaded workspace registry).
    Shared(Arc<AgentDefinitionRegistry>),
    /// The process-wide singleton installed by
    /// [`AgentDefinitionRegistry::init_global`].
    Global(&'static AgentDefinitionRegistry),
}

impl RegistryHandle {
    fn get(&self) -> &AgentDefinitionRegistry {
        match self {
            Self::Shared(registry) => registry.as_ref(),
            Self::Global(registry) => registry,
        }
    }
}

impl std::fmt::Debug for RegistryHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryHandle")
            .field("len", &self.get().len())
            .finish()
    }
}

// ── Adapter ───────────────────────────────────────────────────────────────────

/// OpenHuman's agent catalogue, projected onto the crate's
/// [`DefinitionRegistry`] seam.
///
/// Read-only by construction: it borrows the harness registry, an optional
/// [`Config`] (for the user-authored custom-agent fallback), and never mutates
/// either. That matches the trait's rationale — a catalogue the runtime could
/// mutate would let a turn grant itself a delegate or a tool.
#[derive(Debug)]
pub struct OpenHumanDefinitionRegistry {
    /// Built-in + workspace-override definitions.
    registry: RegistryHandle,
    /// Config snapshot used only for the enabled-custom-agent fallback. When
    /// absent, custom config agents are simply not in the catalogue — an
    /// honest miss, never an error.
    config: Option<Arc<Config>>,
    /// Every tool name registered for this session, used to materialize a
    /// [`ToolScope::Wildcard`] definition that also carries a denylist.
    ///
    /// The host projects named scopes after applying its denylist against a
    /// concrete catalogue. When
    /// this is absent, a wildcard definition whose `disallowed_tools` is
    /// non-empty cannot be projected faithfully and [`Self::tools_for`] fails
    /// closed rather than re-granting the denied tools.
    registered_tools: Option<Arc<Vec<String>>>,
    /// Registered deferred tools. A named belt that lists the intrinsic
    /// `tool_search` bridge grants these names to the hosted harness too.
    deferred_tools: Option<Arc<Vec<String>>>,
    /// Per-invocation direct delegation routes synthesized beside the durable
    /// tool registry. They must augment a named root scope so the hosted loop
    /// authorizes the same hand-off routes it advertises.
    session_delegation_tools: Option<Arc<Vec<String>>>,
    /// The session's own caller-supplied definition, when it has one.
    ///
    /// A library host builds its agent from a definition it owns and passes by
    /// value (`AgentChatTarget::Definition`), which `from_config_with_definition`
    /// stamps on the session and inserts into no registry. `AgentSpec::into_core`
    /// re-stamps the built-in orchestrator under the caller's own id, so ids like
    /// `harness`, `alpha` and `beta` are names no registry has ever held — the
    /// hosted lookup below missed every time, the harness raised
    /// `TinyAgentsError::Validation`, and `hosted_error` sanitized that into
    /// "hosted agent invocation was rejected by policy" with zero provider calls
    /// (#6404, #6392, #6393).
    ///
    /// Consulted BEFORE the registry, which is the precedence
    /// `OpenHumanSessionHost::resolved_definition` already documents and
    /// `resolved_definition_prefers_the_sessions_own_over_a_same_id_registry_entry`
    /// already pins: a library host may legitimately reuse an id the process
    /// registry also knows, and its own definition must win.
    session_definition: Option<Arc<HostAgentDefinition>>,
}

/// Outcome of resolving a definition's own scope.
///
/// Modelled explicitly so a denylist-emptied scope, an explicitly tool-less
/// scope and a true wildcard never collapse onto the same host allowlist.
enum ResolvedScope {
    /// Every registered tool, with no denylist to apply.
    Wildcard,
    /// A concrete list. May legitimately be empty, meaning *no* tools.
    Named(Vec<String>),
}

impl OpenHumanDefinitionRegistry {
    /// Adapts an owned/shared harness registry.
    pub fn new(registry: Arc<AgentDefinitionRegistry>) -> Self {
        Self {
            registry: RegistryHandle::Shared(registry),
            config: None,
            registered_tools: None,
            deferred_tools: None,
            session_delegation_tools: None,
            session_definition: None,
        }
    }

    /// Adapts a freshly-built builtins-only registry (no workspace scan).
    pub fn builtins_only() -> Self {
        Self::new(Arc::new(AgentDefinitionRegistry::builtins_only()))
    }

    /// Attaches the config snapshot that backs the enabled-custom-agent
    /// fallback in [`Self::resolve`] and [`Self::list`].
    pub fn with_config(mut self, config: Arc<Config>) -> Self {
        self.config = Some(config);
        self
    }

    /// Attaches the session's registered tool names.
    ///
    /// Required to project a [`ToolScope::Wildcard`] definition that also
    /// carries a `disallowed_tools` denylist: the host materializes the allowed
    /// names before passing the scope to the harness. Without it such a
    /// definition fails closed — see [`Self::tools_for`].
    pub fn with_registered_tools(mut self, tools: Arc<Vec<String>>) -> Self {
        self.registered_tools = Some(tools);
        self
    }

    pub fn with_deferred_tools(mut self, tools: Arc<Vec<String>>) -> Self {
        self.deferred_tools = Some(tools);
        self
    }

    /// Attaches the root invocation's synthesized direct-delegation names.
    pub fn with_session_delegation_tools(mut self, tools: Arc<Vec<String>>) -> Self {
        self.session_delegation_tools = Some(tools);
        self
    }

    /// Attaches the session's own definition, which outranks the registry.
    ///
    /// See [`Self::session_definition`] — without this a caller-supplied
    /// definition is unresolvable by the id it was stamped with.
    pub fn with_session_definition(mut self, definition: Arc<HostAgentDefinition>) -> Self {
        self.session_definition = Some(definition);
        self
    }

    /// Resolves `id` to a **host** definition: harness registry first, then the
    /// enabled custom-agent config fallback.
    ///
    /// Mirrors the lookup order the agent factory uses
    /// (`agent/session_host/builder/factory.rs` falls back to
    /// [`find_custom_in_config`] on a harness-registry miss). The disabled and
    /// `Default`-source filters live inside [`find_custom_in_config`] and are
    /// deliberately not re-implemented here.
    fn host_definition(&self, id: &str) -> Option<HostAgentDefinition> {
        let id = id.trim();
        if let Some(def) = self
            .session_definition
            .as_deref()
            .filter(|def| def.id.trim() == id)
        {
            return Some(def.clone());
        }
        if let Some(def) = self.registry.get().get(id) {
            return Some(def.clone());
        }
        let entry = find_custom_in_config(self.config.as_deref()?, id)?;
        Some(definition_from_registry_entry(&entry))
    }

    /// Every host definition in the catalogue, in a stable order: harness
    /// definitions in registry insertion order, then enabled custom config
    /// agents that no harness definition already shadows.
    fn host_definitions(&self) -> Vec<HostAgentDefinition> {
        let mut defs: Vec<HostAgentDefinition> = self
            .registry
            .get()
            .list()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();

        if let Some(config) = self.config.as_deref() {
            let known: HashSet<String> = defs.iter().map(|def| def.id.clone()).collect();
            for entry in &config.agent_registry.entries {
                if known.contains(&entry.id) {
                    continue;
                }
                // Route through the shared accessor so the enabled + Custom
                // source guard has exactly one implementation.
                if let Some(entry) = find_custom_in_config(config, &entry.id) {
                    defs.push(definition_from_registry_entry(&entry));
                }
            }
        }

        defs
    }

    /// Projects one host definition onto the crate's inert
    /// [`AgentDefinition`].
    fn project(&self, def: &HostAgentDefinition) -> AgentDefinition {
        AgentDefinition {
            id: def.id.clone(),
            display_name: Some(def.display_name().to_string()),
            // `when_to_use` is exactly the trait's "capability summary shown to
            // a delegating parent" — the same string the harness feeds into a
            // synthesised `delegate_*` tool description.
            when_to_use: def.when_to_use.clone(),
            // The definition's validated tier is the host assertion the model
            // resolver consumes for workload routing.
            role: Some(def.agent_tier.to_string()),
            agent_tier: def.agent_tier,
            model: def.model.clone(),
            subagents: def
                .allowed_subagent_ids()
                .into_iter()
                .map(tinyagents_definition::SubagentEntry::AgentId)
                .collect(),
            tools: tinyagents_definition::ToolScope::Named(self.tools_for(def)),
            // The definition's rule layer (its `tool_rules` plus its
            // `disallowed_tools` as a deny), enforced by the harness gate on
            // the catalogue, `tool_search` and every call of a hosted run.
            tool_rules: crate::tools::rules::agent_rule_layer(def),
            ..Default::default()
        }
    }

    /// Tool names for `def`, after the definition's own denylist.
    fn tools_for(&self, def: &HostAgentDefinition) -> Vec<String> {
        Self::emit(self.resolved_scope(def))
    }

    /// Resolves the definition's own scope, applying `extra_tools` and the
    /// denylist.
    fn resolved_scope(&self, def: &HostAgentDefinition) -> ResolvedScope {
        match &def.tools {
            ToolScope::Named(named) => {
                let mut names = named.clone();
                if named.iter().any(|name| name == tinytools::TOOL_SEARCH_NAME) {
                    if let Some(deferred) = self.deferred_tools.as_deref() {
                        names.extend(deferred.iter().cloned());
                    }
                }
                if let Some(delegation_tools) = self.session_delegation_tools.as_deref() {
                    names.extend(delegation_tools.iter().cloned());
                }
                // A curated belt still has to reach the tools a compacted
                // result names (`juice_retrieve`, `juice_find`, …). The
                // session adds them to its visible set; without the same
                // names here the harness allowlist rejects every call to them
                // as an unknown tool. A zero-tool belt stays zero-tool.
                if !named.is_empty() {
                    if let (Some(config), Some(registered)) =
                        (self.config.as_deref(), self.registered_tools.as_deref())
                    {
                        for name in
                            crate::inference::tokenjuice::companion_tool_names(&def.id, config)
                        {
                            if registered.iter().any(|r| r == name) {
                                names.push(name.to_string());
                            }
                        }
                    }
                }
                // `extra_tools` is an "also include these" hook on top of a
                // named scope. Under `Wildcard` it is meaningless — everything
                // is already in scope.
                names.extend(def.extra_tools.iter().cloned());
                names.retain(|name| {
                    !crate::tools::rules::glob_list_matches(&def.disallowed_tools, name)
                });
                dedupe_preserving_order(&mut names);
                // Deliberately *not* collapsed to `Wildcard` when empty: an
                // agent configured with no tools, or one whose whole scope was
                // denied, must project as no tools rather than as everything.
                ResolvedScope::Named(names)
            }
            ToolScope::Wildcard => match self.registered_tools.as_deref() {
                // The hosted API treats an empty list as deny-all, so materialize
                // every wildcard scope rather than serializing it as an empty
                // vector. Apply the denylist while doing so.
                Some(registered) => {
                    let mut names: Vec<String> = registered
                        .iter()
                        .filter(|name| {
                            !crate::tools::rules::glob_list_matches(&def.disallowed_tools, name)
                        })
                        .cloned()
                        .collect();
                    dedupe_preserving_order(&mut names);
                    ResolvedScope::Named(names)
                }
                // Fail closed when the concrete session tool surface is absent.
                None => {
                    log::error!(
                        "[tinyagents][definitions] agent '{}' has a wildcard tool scope but no \
                         registered tool list was \
                         attached — failing closed to no tools. Call \
                         `with_registered_tools(..)` to project this definition.",
                        def.id
                    );
                    ResolvedScope::Named(Vec::new())
                }
            },
        }
    }

    /// Renders a resolved scope into the crate's `Vec<String>`, substituting the
    /// sentinel for a genuinely empty named scope.
    ///
    /// This is the single place the crate's "empty means unrestricted"
    /// convention is applied, so no caller can accidentally emit a bare empty
    /// vec that reads as "all tools".
    fn emit(scope: ResolvedScope) -> Vec<String> {
        match scope {
            ResolvedScope::Wildcard => Vec::new(),
            ResolvedScope::Named(mut names) if names.is_empty() => {
                names.push(NO_TOOLS_SENTINEL.to_string());
                names
            }
            ResolvedScope::Named(names) => names,
        }
    }

    /// Tier-checked delegate ids for `def`.
    ///
    /// Reuses [`tinyagents_definition::validate_tier_transition`]
    /// — the single source of truth `DefinitionSet::validate_tier_hierarchy` walks at boot —
    /// so this seam can never disagree with the host's boot-time validation.
    fn authorized_delegates(&self, def: &HostAgentDefinition) -> Vec<String> {
        if def.agent_tier == AgentTier::Worker {
            // the owner validator hard-fails a worker that lists any
            // agent id, so a worker's authorized set is empty by construction.
            return Vec::new();
        }

        let mut out = Vec::new();
        for id in def.allowed_subagent_ids() {
            let Some(child) = self.host_definition(&id) else {
                // Unknown child: the owner validator tolerates it
                // rather than failing, and the trait explicitly allows an
                // authorized id that does not resolve in this build.
                out.push(id);
                continue;
            };
            match tinyagents_definition::validate_tier_transition(def.agent_tier, child.agent_tier)
            {
                Ok(()) => out.push(id),
                Err(reason) => {
                    tracing::warn!(
                        target: "tinyagents",
                        parent = %def.id,
                        parent_tier = %def.agent_tier.as_str(),
                        child = %id,
                        child_tier = %child.agent_tier.as_str(),
                        %reason,
                        "[tinyagents] dropping tier-illegal declared subagent from the \
                         authorized delegate set"
                    );
                }
            }
        }
        out
    }
}

// ── Free helpers ──────────────────────────────────────────────────────────────

/// Drops repeated names while keeping first-occurrence order.
///
/// `extra_tools` may restate something already in the named scope; the crate
/// matches tool names against its registry, so a duplicate is harmless but
/// makes the projected list noisier than the host's own visible set.
fn dedupe_preserving_order(names: &mut Vec<String>) {
    let mut seen: HashSet<String> = HashSet::with_capacity(names.len());
    names.retain(|name| seen.insert(name.clone()));
}

// ── Trait impl ────────────────────────────────────────────────────────────────

#[async_trait]
impl DefinitionRegistry for OpenHumanDefinitionRegistry {
    /// Never returns `Err`. Every lookup path here is an in-memory map probe or
    /// a `Vec` scan over an already-loaded config, so there is no "failed to
    /// answer" case to distinguish — and OpenHuman's catalogue legitimately
    /// names agents this build compiled out.
    async fn resolve(&self, id: &str) -> Result<Option<AgentDefinition>> {
        Ok(self.host_definition(id).map(|def| self.project(&def)))
    }

    /// Harness definitions in registry insertion order, then enabled custom
    /// config agents in config order. Both sources are ordered containers, so
    /// the result is stable across calls and does not reshuffle a cached prompt
    /// prefix.
    async fn list(&self) -> Result<Vec<AgentDefinition>> {
        Ok(self
            .host_definitions()
            .iter()
            .map(|def| self.project(def))
            .collect())
    }

    /// Tier-checked delegate ids; empty for an unknown `id`.
    async fn delegates_for(&self, id: &str) -> Result<Vec<String>> {
        Ok(self
            .host_definition(id)
            .map(|def| self.authorized_delegates(&def))
            .unwrap_or_default())
    }
}

#[cfg(test)]
#[path = "definition_registry_tests.rs"]
mod tests;
