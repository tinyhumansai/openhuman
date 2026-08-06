//! Host implementation of the TinyAgents **agent catalogue** seam.
//!
//! Adapts two OpenHuman domains onto
//! [`tinyagents::harness::host::DefinitionRegistry`]:
//!
//! * [`crate::openhuman::agent::harness::definition::AgentDefinitionRegistry`] —
//!   the harness registry of built-in ([`load_builtins`](crate::openhuman::agent::registry::agents::load_builtins)-parsed
//!   `agent.toml`)
//!   and workspace-override agent definitions, plus the config-backed
//!   user-authored fallback
//!   ([`crate::openhuman::agent::registry::find_custom_in_config`] →
//!   [`crate::openhuman::agent::registry::definition_from_registry_entry`]).
//! * [`crate::openhuman::agent::profiles::AgentProfile`] — the active personality,
//!   whose `allowed_tools` list is a **restriction** on the resolved
//!   definition's tool surface.
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
//! this: `orchestrator/agent.toml` lists `mcp_agent` in `subagents` even in a
//! build with the `mcp` feature off, and both existing resolution sites
//! tolerate it (`collect_orchestrator_tools` warns and skips;
//! [`validate_tier_hierarchy`](crate::openhuman::agent::registry::agents::validate_tier_hierarchy) explicitly `continue`s past unknown ids). So
//! `OpenHumanDefinitionRegistry::resolve` returns `Ok(None)` for every miss
//! and this adapter has no error path at all.
//!
//! **2. Declared vs authorized subagents.** The crate's
//! `AgentDefinition::subagents` is only what an agent *declares*;
//! `delegates_for` must return the **authorized** set. OpenHuman's authority is
//! the tier hierarchy, so `delegates_for` applies
//! [`validate_tier_transition`](crate::openhuman::agent::harness::definition::validate_tier_transition)
//! — the same single source of truth
//! [`validate_tier_hierarchy`](crate::openhuman::agent::registry::agents::validate_tier_hierarchy) walks at boot — per declared pair. A `Worker`
//! parent yields an empty list; a tier-illegal child is dropped. Unknown child
//! ids are **kept**, matching both [`validate_tier_hierarchy`](crate::openhuman::agent::registry::agents::validate_tier_hierarchy)'s `continue` and
//! the trait's note that an authorized id may still fail to resolve.
//!
//! **3. `ToolScope::Wildcard` has no crate representation.** The crate models
//! tools as an explicit `Vec<String>` in which **empty means unrestricted**,
//! matching the session builder ("an empty `visible` set means no filter" —
//! `agent/harness/session/builder/factory.rs`). So an unrestricted wildcard
//! agent maps to an empty `tools` vec.
//!
//! That one value must not be made to carry three meanings. Three distinct
//! situations would otherwise all collapse onto "empty", and each would read
//! back as *every tool*:
//!
//! * a named scope configured with **no** tools,
//! * a named scope whose every entry the denylist removed,
//! * a wildcard scope carrying a denylist the crate cannot express.
//!
//! [`ResolvedScope`] therefore models wildcard-ness explicitly and never infers
//! it from emptiness. A genuinely empty scope emits
//! [`PROFILE_NO_TOOLS_SENTINEL`] — an unregistered name that matches nothing —
//! and a wildcard-with-denylist is materialized against
//! [`Self::with_registered_tools`], failing closed when that is absent.
//!
//! **4. `SubagentEntry::Skills` entries are omitted.** A `{ skills = "*" }`
//! entry is not an agent id — it collapses into the single
//! `delegate_to_integrations_agent` tool. Emitting a synthetic id here would
//! invent a delegate the host never authorized.
//!
//! **5. Profile model overrides are deliberately not applied.**
//! `AgentProfile::model_override` has no verified host consumer on the
//! definition path (the web-chat `model_override` request parameter is a
//! different value, applied to `Config::default_model`), and the model seam is
//! `ModelResolver`'s, not the catalogue's. See the `TODO(phase4)` below.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents::error::Result;
use tinyagents::harness::host::{AgentDefinition, DefinitionRegistry};

use crate::openhuman::agent::harness::definition::{
    AgentDefinition as HostAgentDefinition, AgentDefinitionRegistry, AgentTier, ModelSpec,
    SubagentEntry, ToolScope,
};
use crate::openhuman::agent::profiles::AgentProfile;
use crate::openhuman::agent::registry::{definition_from_registry_entry, find_custom_in_config};
use crate::openhuman::config::Config;

/// Sentinel inserted when a profile allowlist and a definition's named scope
/// are disjoint.
///
/// Copied verbatim from the session builder
/// (`agent/harness/session/builder/factory.rs`), where it exists because an
/// empty tool set is the "all tools" sentinel: a disjoint intersection must
/// stay non-empty with an unregistered name so it permits zero tools rather
/// than accidentally broadening to everything.
const PROFILE_NO_TOOLS_SENTINEL: &str = "__profile_no_tools__";

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
/// [`Config`] (for the user-authored custom-agent fallback), and an optional
/// active [`AgentProfile`] (for the tool restriction), and never mutates any of
/// them. That matches the trait's rationale — a catalogue the runtime could
/// mutate would let a turn grant itself a delegate or a tool.
#[derive(Debug)]
pub struct OpenHumanDefinitionRegistry {
    /// Built-in + workspace-override definitions.
    registry: RegistryHandle,
    /// Config snapshot used only for the enabled-custom-agent fallback. When
    /// absent, custom config agents are simply not in the catalogue — an
    /// honest miss, never an error.
    config: Option<Arc<Config>>,
    /// Active personality. Its `allowed_tools` narrows every projected tool
    /// list, exactly as the session builder narrows the visible tool set.
    profile: Option<Arc<AgentProfile>>,
    /// Every tool name registered for this session, used to materialize a
    /// [`ToolScope::Wildcard`] definition that also carries a denylist.
    ///
    /// The crate models tools as an explicit `Vec<String>` with no wildcard
    /// marker, so a denylist can only be honoured against a concrete list. When
    /// this is absent, a wildcard definition whose `disallowed_tools` is
    /// non-empty cannot be projected faithfully and [`Self::tools_for`] fails
    /// closed rather than re-granting the denied tools.
    registered_tools: Option<Arc<Vec<String>>>,
}

/// Outcome of resolving a definition's own scope, before the profile allowlist.
///
/// Modelled explicitly because the crate's `Vec<String>` overloads *empty* to
/// mean "unrestricted". Inferring wildcard from emptiness is what let a
/// denylist-emptied scope, an explicitly tool-less scope, and a true wildcard
/// all collapse onto the same value.
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
            profile: None,
            registered_tools: None,
        }
    }

    /// Adapts the process-wide registry, or `None` when
    /// [`AgentDefinitionRegistry::init_global`] has not run yet.
    ///
    /// Returning `Option` rather than lazily initialising keeps boot ordering
    /// the host's decision: silently building a builtins-only registry here
    /// would mask a missing workspace-override load.
    pub fn from_global() -> Option<Self> {
        AgentDefinitionRegistry::global().map(|registry| Self {
            registry: RegistryHandle::Global(registry),
            config: None,
            profile: None,
            registered_tools: None,
        })
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

    /// Attaches the active personality whose `allowed_tools` restricts every
    /// projected tool list.
    pub fn with_profile(mut self, profile: Arc<AgentProfile>) -> Self {
        self.profile = Some(profile);
        self
    }

    /// Attaches the session's registered tool names.
    ///
    /// Required to project a [`ToolScope::Wildcard`] definition that also
    /// carries a `disallowed_tools` denylist: the crate has no wildcard marker,
    /// so "everything except these" can only be expressed by materializing the
    /// list. Without it such a definition fails closed — see [`Self::tools_for`].
    pub fn with_registered_tools(mut self, tools: Arc<Vec<String>>) -> Self {
        self.registered_tools = Some(tools);
        self
    }

    /// Resolves `id` to a **host** definition: harness registry first, then the
    /// enabled custom-agent config fallback.
    ///
    /// Mirrors the lookup order the agent factory uses
    /// (`agent/harness/session/builder/factory.rs` falls back to
    /// [`find_custom_in_config`] on a harness-registry miss). The disabled and
    /// `Default`-source filters live inside [`find_custom_in_config`] and are
    /// deliberately not re-implemented here.
    fn host_definition(&self, id: &str) -> Option<HostAgentDefinition> {
        let id = id.trim();
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
            name: def.display_name().to_string(),
            // `when_to_use` is exactly the trait's "capability summary shown to
            // a delegating parent" — the same string the harness feeds into a
            // synthesised `delegate_*` tool description.
            description: def.when_to_use.clone(),
            model: model_for(&def.model),
            subagents: declared_subagent_ids(def),
            tools: self.tools_for(def),
        }
    }

    /// Tool names for `def`, after the definition's own denylist and the active
    /// profile's allowlist.
    ///
    /// Both filters mirror the session builder rather than reinventing policy:
    /// a profile's tool selection is *a restriction on the resolved definition,
    /// never a replacement for it*.
    fn tools_for(&self, def: &HostAgentDefinition) -> Vec<String> {
        let scope = self.resolved_scope(def);

        let Some(allowed) = self
            .profile
            .as_deref()
            .and_then(|profile| profile.allowed_tools.as_ref())
            .filter(|tools| !tools.is_empty())
        else {
            return Self::emit(scope);
        };

        let profile_visible: Vec<String> = allowed
            .iter()
            .map(|tool| tool.trim().to_string())
            .filter(|tool| !tool.is_empty())
            .collect();
        if profile_visible.is_empty() {
            return Self::emit(scope);
        }

        let mut names = match scope {
            // A true wildcard has no denylist left to honour (see
            // `resolved_scope`), so the profile allowlist *is* the visible set.
            ResolvedScope::Wildcard => return profile_visible,
            ResolvedScope::Named(names) => names,
        };

        let allowed_set: HashSet<&str> = profile_visible.iter().map(String::as_str).collect();
        names.retain(|name| allowed_set.contains(name.as_str()));
        Self::emit(ResolvedScope::Named(names))
    }

    /// Resolves the definition's own scope, applying `extra_tools` and the
    /// denylist, without consulting the profile.
    fn resolved_scope(&self, def: &HostAgentDefinition) -> ResolvedScope {
        match &def.tools {
            ToolScope::Named(named) => {
                let mut names = named.clone();
                // `extra_tools` is an "also include these" hook on top of a
                // named scope. Under `Wildcard` it is meaningless — everything
                // is already in scope.
                names.extend(def.extra_tools.iter().cloned());
                names.retain(|name| !disallows_tool(&def.disallowed_tools, name));
                dedupe_preserving_order(&mut names);
                // Deliberately *not* collapsed to `Wildcard` when empty: an
                // agent configured with no tools, or one whose whole scope was
                // denied, must project as no tools rather than as everything.
                ResolvedScope::Named(names)
            }
            ToolScope::Wildcard if def.disallowed_tools.is_empty() => ResolvedScope::Wildcard,
            ToolScope::Wildcard => match self.registered_tools.as_deref() {
                // "Everything except these" is only expressible against a
                // concrete list, so materialize and filter.
                Some(registered) => {
                    let mut names: Vec<String> = registered
                        .iter()
                        .filter(|name| !disallows_tool(&def.disallowed_tools, name))
                        .cloned()
                        .collect();
                    dedupe_preserving_order(&mut names);
                    ResolvedScope::Named(names)
                }
                // Fail closed. Emitting the wildcard here would silently
                // re-grant every denied tool — for shipped definitions that
                // means specialist-only routes like `polymarket` / `kalshi` /
                // `tinyplace_*` becoming generally available. An agent with no
                // tools is a visible, debuggable failure; a silently widened
                // one is not.
                None => {
                    log::error!(
                        "[tinyagents][definitions] agent '{}' has a wildcard tool scope with a \
                         non-empty denylist ({} entries) but no registered tool list was \
                         attached — failing closed to no tools. Call \
                         `with_registered_tools(..)` to project this definition.",
                        def.id,
                        def.disallowed_tools.len()
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
                names.push(PROFILE_NO_TOOLS_SENTINEL.to_string());
                names
            }
            ResolvedScope::Named(names) => names,
        }
    }

    /// Tier-checked delegate ids for `def`.
    ///
    /// Reuses [`crate::openhuman::agent::harness::definition::validate_tier_transition`]
    /// — the single source of truth [`validate_tier_hierarchy`](crate::openhuman::agent::registry::agents::validate_tier_hierarchy) walks at boot —
    /// so this seam can never disagree with the host's boot-time validation.
    fn authorized_delegates(&self, def: &HostAgentDefinition) -> Vec<String> {
        if def.agent_tier == AgentTier::Worker {
            // `validate_tier_hierarchy` hard-fails a worker that lists any
            // agent id, so a worker's authorized set is empty by construction.
            return Vec::new();
        }

        let mut out = Vec::new();
        for id in declared_subagent_ids(def) {
            let Some(child) = self.host_definition(&id) else {
                // Unknown child: `validate_tier_hierarchy` `continue`s past it
                // rather than failing, and the trait explicitly allows an
                // authorized id that does not resolve in this build.
                out.push(id);
                continue;
            };
            match crate::openhuman::agent::harness::definition::validate_tier_transition(
                def.agent_tier,
                child.agent_tier,
            ) {
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

/// Declared subagent **agent ids** only.
///
/// [`SubagentEntry::Skills`] entries are skipped: they are a wildcard that
/// collapses to the single `delegate_to_integrations_agent` tool, not an agent
/// the parent may address by id.
fn declared_subagent_ids(def: &HostAgentDefinition) -> Vec<String> {
    def.subagents
        .iter()
        .filter_map(|entry| match entry {
            SubagentEntry::AgentId(id) => Some(id.clone()),
            SubagentEntry::Skills(_) => None,
        })
        .collect()
}

/// Maps a host [`ModelSpec`] onto the crate's `Option<String>` model pin.
///
/// [`ModelSpec::Inherit`] becomes `None`, which is precisely the crate's "no
/// preference, the session default applies". `Hint` is resolved through
/// [`ModelSpec::resolve`] (whose `parent_model` argument is unused for the hint
/// arm) so the `{hint}-v1` naming convention has one implementation.
fn model_for(spec: &ModelSpec) -> Option<String> {
    match spec {
        ModelSpec::Inherit => None,
        ModelSpec::Exact(name) => Some(name.clone()),
        ModelSpec::Hint(_) => Some(spec.resolve("")),
    }
}

/// Whether `name` is blocked by a definition's `disallowed_tools`.
///
/// Mirrors the private `definition_disallows_tool` in
/// `agent/harness/session/builder/factory.rs`, including its trailing-`*`
/// prefix-match form. Duplicated rather than imported because that helper is
/// module-private and Phase 4 must not edit existing files.
///
/// TODO(phase4): make `definition_disallows_tool` `pub(crate)` in
/// `agent/harness/session/builder/factory.rs` and delete this copy, so the
/// denylist grammar has one implementation.
fn disallows_tool(disallowed: &[String], name: &str) -> bool {
    disallowed.iter().any(|entry| {
        if let Some(prefix) = entry.strip_suffix('*') {
            name.starts_with(prefix)
        } else {
            entry == name
        }
    })
}

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

// TODO(phase4): `AgentProfile::model_override` is not applied to the projected
// `model` field. It has no verified consumer on the host definition path today
// (`web_chat::session::build_session_agent` applies a *request* `model_override`
// to `Config::default_model`, which is a different value), and per-session model
// choice belongs to the `ModelResolver` seam rather than the catalogue. If the
// host does want a personality to re-pin an agent's model, it likely belongs in
// the `ModelResolver` adapter reading `profiles::AgentProfile::model_override`.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::registry::types::{
        AgentRegistryEntry, AgentRegistrySource, AgentSubagentPolicy,
    };

    fn builtins() -> OpenHumanDefinitionRegistry {
        OpenHumanDefinitionRegistry::builtins_only()
    }

    /// A synthetic host definition built through the public
    /// [`definition_from_registry_entry`] constructor, so the test never
    /// hand-rolls the harness struct's ~25 fields.
    fn synthetic(id: &str, tier: AgentTier, subagents: &[&str]) -> HostAgentDefinition {
        let entry = AgentRegistryEntry {
            id: id.to_string(),
            name: format!("{id} display"),
            description: format!("Use {id} for testing."),
            source: AgentRegistrySource::Custom,
            enabled: true,
            model: None,
            system_prompt: None,
            tool_allowlist: Vec::new(),
            tool_denylist: Vec::new(),
            subagents: AgentSubagentPolicy::from_allowlist(
                subagents.iter().map(|s| s.to_string()).collect(),
            ),
            tags: Vec::new(),
            metadata: serde_json::Value::Null,
        };
        let mut def = definition_from_registry_entry(&entry);
        def.agent_tier = tier;
        def
    }

    fn registry_of(defs: Vec<HostAgentDefinition>) -> OpenHumanDefinitionRegistry {
        let mut registry = AgentDefinitionRegistry::default();
        for def in defs {
            registry.insert(def);
        }
        OpenHumanDefinitionRegistry::new(Arc::new(registry))
    }

    // ── the absence contract ──────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_returns_none_for_an_unknown_id_without_erroring() {
        // THE contract of this trait: OpenHuman's orchestrator TOML lists
        // subagents that a feature-gated build compiles out, and both existing
        // resolution sites tolerate that. An `Err` here would turn ordinary
        // build variance into a failed run.
        let resolved = builtins()
            .resolve("definitely_not_a_real_agent")
            .await
            .expect("an unknown id must not be an error");
        assert_eq!(resolved, None);
    }

    #[tokio::test]
    async fn a_declared_subagent_that_does_not_resolve_is_still_authorized() {
        // The two halves of the contract together: `delegates_for` keeps an id
        // it cannot resolve (authorization and compiled-in-ness are
        // independent), and resolving that id is `Ok(None)`, not `Err`.
        let registry = registry_of(vec![synthetic(
            "lead",
            AgentTier::Chat,
            &["compiled_out_agent"],
        )]);

        let delegates = registry.delegates_for("lead").await.expect("delegates");
        assert_eq!(delegates, vec!["compiled_out_agent".to_string()]);
        assert_eq!(
            registry
                .resolve("compiled_out_agent")
                .await
                .expect("an unresolvable delegate must not error"),
            None
        );
    }

    #[tokio::test]
    async fn delegates_for_is_empty_for_an_unknown_id() {
        let delegates = builtins()
            .delegates_for("definitely_not_a_real_agent")
            .await
            .expect("delegates");
        assert!(delegates.is_empty());
    }

    // ── tier checking ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn delegates_for_drops_tier_illegal_children() {
        // `chat -> chat` and `reasoning -> reasoning` are the two forbidden
        // same-tier hops; `chat -> worker` is legal.
        let registry = registry_of(vec![
            synthetic("lead", AgentTier::Chat, &["other_chat", "a_worker"]),
            synthetic("other_chat", AgentTier::Chat, &[]),
            synthetic("a_worker", AgentTier::Worker, &[]),
        ]);

        let delegates = registry.delegates_for("lead").await.expect("delegates");
        assert_eq!(delegates, vec!["a_worker".to_string()]);
    }

    #[tokio::test]
    async fn a_worker_parent_authorizes_nothing() {
        // `validate_tier_hierarchy` hard-fails a worker that lists any agent
        // id, so the authorized set must be empty rather than the raw list.
        let registry = registry_of(vec![
            synthetic("leaf", AgentTier::Worker, &["a_worker"]),
            synthetic("a_worker", AgentTier::Worker, &[]),
        ]);

        assert!(registry
            .delegates_for("leaf")
            .await
            .expect("delegates")
            .is_empty());
        // The *declared* list is still reported on the definition — declared
        // and authorized are different questions.
        let def = registry
            .resolve("leaf")
            .await
            .expect("resolve")
            .expect("leaf exists");
        assert_eq!(def.subagents, vec!["a_worker".to_string()]);
    }

    #[tokio::test]
    async fn authorized_delegates_agree_with_boot_time_validation() {
        // Cross-check against the host's own boot validator over the real
        // built-in set: every id `delegates_for` returns must be one
        // `validate_tier_hierarchy` accepts (it accepts the whole builtin
        // catalogue, so nothing may be dropped there).
        let builtin_defs = crate::openhuman::agent::registry::agents::load_builtins()
            .expect("built-in TOML must parse");
        crate::openhuman::agent::registry::agents::validate_tier_hierarchy(&builtin_defs)
            .expect("built-ins satisfy the hierarchy");

        let registry = builtins();
        for def in &builtin_defs {
            let declared = declared_subagent_ids(def);
            let authorized = registry
                .delegates_for(&def.id)
                .await
                .expect("delegates never error");
            for id in &authorized {
                assert!(
                    declared.contains(id),
                    "{} authorized `{id}` which it never declared",
                    def.id
                );
            }
            if def.agent_tier == AgentTier::Worker {
                assert!(
                    authorized.is_empty(),
                    "worker `{}` must authorize no delegates",
                    def.id
                );
            }
        }
    }

    // ── projection ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn orchestrator_projects_its_identity_fields() {
        let def = builtins()
            .resolve("orchestrator")
            .await
            .expect("resolve")
            .expect("the orchestrator is always a built-in");
        assert_eq!(def.id, "orchestrator");
        assert!(!def.name.is_empty());
        assert!(
            !def.description.is_empty(),
            "description is the delegating parent's capability summary"
        );
        assert!(
            !def.subagents.is_empty(),
            "the orchestrator declares delegates"
        );
        assert!(
            !def.tools.is_empty(),
            "the orchestrator has a named tool scope"
        );
    }

    #[test]
    fn model_spec_maps_inherit_to_no_preference() {
        assert_eq!(model_for(&ModelSpec::Inherit), None);
        assert_eq!(
            model_for(&ModelSpec::Exact("neocortex-mk1".into())),
            Some("neocortex-mk1".to_string())
        );
        // Hints go through `ModelSpec::resolve`, which is the one place the
        // `{hint}-v1` convention lives.
        assert_eq!(
            model_for(&ModelSpec::Hint("reasoning".into())),
            Some("reasoning-v1".to_string())
        );
    }

    #[test]
    fn skills_wildcard_entries_are_not_agent_ids() {
        use crate::openhuman::agent::harness::definition::SkillsWildcard;
        let mut def = synthetic("lead", AgentTier::Chat, &["a_worker"]);
        def.subagents.push(SubagentEntry::Skills(SkillsWildcard {
            skills: "*".to_string(),
        }));
        assert_eq!(declared_subagent_ids(&def), vec!["a_worker".to_string()]);
    }

    #[test]
    fn denylist_supports_exact_and_prefix_forms() {
        let denied = vec!["file_write".to_string(), "storage_*".to_string()];
        assert!(disallows_tool(&denied, "file_write"));
        assert!(disallows_tool(&denied, "storage_delete_file"));
        assert!(!disallows_tool(&denied, "file_read"));
    }

    /// A wildcard scope with nothing denied is the one case where the crate's
    /// "empty means unrestricted" marker is the faithful projection.
    #[test]
    fn an_undenied_wildcard_scope_projects_the_unrestricted_marker() {
        let mut def = synthetic("wide", AgentTier::Worker, &[]);
        def.tools = ToolScope::Wildcard;
        def.disallowed_tools = Vec::new();

        assert!(
            registry_of(vec![def.clone()])
                .project(&def)
                .tools
                .is_empty(),
            "an undenied wildcard is genuinely unrestricted"
        );
    }

    /// `tools_agent` ships `disallowed_tools = ["polymarket", "kalshi",
    /// "tinyplace_*"]` on a wildcard scope, precisely so those route through
    /// their specialist agents. Projecting it as the unrestricted marker would
    /// hand all three back.
    #[tokio::test]
    async fn a_wildcard_denylist_is_materialized_against_the_registered_tools() {
        let registered = Arc::new(vec![
            "file_read".to_string(),
            "polymarket".to_string(),
            "kalshi".to_string(),
            "tinyplace_post".to_string(),
        ]);
        let def = builtins()
            .host_definition("tools_agent")
            .expect("tools_agent is a built-in");
        assert!(
            !def.disallowed_tools.is_empty(),
            "this test is meaningless if tools_agent stops denying anything"
        );

        let projected = OpenHumanDefinitionRegistry::builtins_only()
            .with_registered_tools(registered)
            .resolve("tools_agent")
            .await
            .expect("resolve")
            .expect("tools_agent is a built-in");

        assert_eq!(projected.tools, vec!["file_read".to_string()]);
        for denied in ["polymarket", "kalshi", "tinyplace_post"] {
            assert!(
                !projected.tools.iter().any(|t| t == denied),
                "{denied} is reserved for its specialist route"
            );
        }
    }

    /// Without a registered-tool list the denylist cannot be expressed, so the
    /// projection must fail closed rather than widen to everything.
    #[tokio::test]
    async fn a_wildcard_denylist_without_registered_tools_fails_closed() {
        let projected = builtins()
            .resolve("tools_agent")
            .await
            .expect("resolve")
            .expect("tools_agent is a built-in");

        assert_eq!(
            projected.tools,
            vec![PROFILE_NO_TOOLS_SENTINEL.to_string()],
            "an unexpressible denylist must not read back as 'all tools'"
        );
    }

    /// An agent configured with an empty allowlist wants *no* tools. The empty
    /// vec would say the opposite.
    #[test]
    fn an_explicitly_tool_less_named_scope_projects_the_no_tools_sentinel() {
        let mut def = synthetic("toolless", AgentTier::Worker, &[]);
        def.tools = ToolScope::Named(Vec::new());
        def.extra_tools = Vec::new();

        assert_eq!(
            registry_of(vec![def.clone()]).project(&def).tools,
            vec![PROFILE_NO_TOOLS_SENTINEL.to_string()]
        );
    }

    /// Same requirement when the denylist is what emptied the scope.
    #[test]
    fn a_named_scope_emptied_by_its_denylist_projects_the_no_tools_sentinel() {
        let mut def = synthetic("denied", AgentTier::Worker, &[]);
        def.tools = ToolScope::Named(vec!["polymarket".to_string()]);
        def.extra_tools = Vec::new();
        def.disallowed_tools = vec!["polymarket".to_string()];

        assert_eq!(
            registry_of(vec![def.clone()]).project(&def).tools,
            vec![PROFILE_NO_TOOLS_SENTINEL.to_string()]
        );
    }

    #[test]
    fn named_scope_drops_denied_tools_and_keeps_extras() {
        let mut def = synthetic("worker", AgentTier::Worker, &[]);
        def.tools = ToolScope::Named(vec!["file_read".into(), "file_write".into()]);
        def.extra_tools = vec!["grep".into(), "file_read".into()];
        def.disallowed_tools = vec!["file_write".into()];

        let registry = registry_of(vec![def.clone()]);
        assert_eq!(
            registry.project(&def).tools,
            vec!["file_read".to_string(), "grep".to_string()]
        );
    }

    // ── profile restriction ───────────────────────────────────────────────

    fn profile_allowing(tools: &[&str]) -> Arc<AgentProfile> {
        let mut profile = crate::openhuman::agent::profiles::built_in_profiles()
            .into_iter()
            .next()
            .expect("at least one built-in profile ships");
        profile.allowed_tools = Some(tools.iter().map(|t| t.to_string()).collect());
        Arc::new(profile)
    }

    #[test]
    fn profile_allowlist_narrows_a_named_scope() {
        let mut def = synthetic("worker", AgentTier::Worker, &[]);
        def.tools = ToolScope::Named(vec!["file_read".into(), "grep".into()]);

        let registry = registry_of(vec![def.clone()]).with_profile(profile_allowing(&["grep"]));
        assert_eq!(registry.project(&def).tools, vec!["grep".to_string()]);
    }

    #[test]
    fn a_disjoint_profile_allowlist_yields_zero_tools_not_all_tools() {
        // The failure mode the host's sentinel exists to prevent: an empty list
        // reads as "unrestricted", so a disjoint intersection must stay
        // non-empty with an unregistered name.
        let mut def = synthetic("worker", AgentTier::Worker, &[]);
        def.tools = ToolScope::Named(vec!["file_read".into()]);

        let registry =
            registry_of(vec![def.clone()]).with_profile(profile_allowing(&["something_else"]));
        assert_eq!(
            registry.project(&def).tools,
            vec![PROFILE_NO_TOOLS_SENTINEL.to_string()]
        );
    }

    #[test]
    fn profile_allowlist_becomes_the_visible_set_for_a_wildcard_agent() {
        let mut def = synthetic("worker", AgentTier::Worker, &[]);
        def.tools = ToolScope::Wildcard;

        let registry =
            registry_of(vec![def.clone()]).with_profile(profile_allowing(&[" grep ", ""]));
        assert_eq!(registry.project(&def).tools, vec!["grep".to_string()]);
    }

    // ── config-backed custom agents ───────────────────────────────────────

    fn config_with(entries: Vec<AgentRegistryEntry>) -> Arc<Config> {
        let mut config = Config::default();
        config.agent_registry.entries = entries;
        Arc::new(config)
    }

    fn custom_entry(id: &str, enabled: bool) -> AgentRegistryEntry {
        AgentRegistryEntry {
            id: id.to_string(),
            name: "Finance Analyst".to_string(),
            description: "Handles finance questions.".to_string(),
            source: AgentRegistrySource::Custom,
            enabled,
            model: Some("hint:reasoning".to_string()),
            system_prompt: Some("Do finance work.".to_string()),
            tool_allowlist: vec!["memory_recall".to_string()],
            tool_denylist: Vec::new(),
            subagents: AgentSubagentPolicy::default(),
            tags: Vec::new(),
            metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn an_enabled_custom_config_agent_resolves_and_lists() {
        let registry =
            registry_of(Vec::new()).with_config(config_with(vec![custom_entry("finance", true)]));

        let def = registry
            .resolve("finance")
            .await
            .expect("resolve")
            .expect("an enabled custom agent is in the catalogue");
        assert_eq!(def.name, "Finance Analyst");
        assert_eq!(def.model.as_deref(), Some("reasoning-v1"));
        assert_eq!(def.tools, vec!["memory_recall".to_string()]);

        let listed = registry.list().await.expect("list");
        assert_eq!(
            listed.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            vec!["finance"]
        );
    }

    #[tokio::test]
    async fn a_disabled_custom_config_agent_is_a_miss_not_an_error() {
        // The disabled filter lives in `find_custom_in_config`; this pins that
        // the adapter routes through it rather than reading entries directly.
        let registry =
            registry_of(Vec::new()).with_config(config_with(vec![custom_entry("finance", false)]));

        assert_eq!(registry.resolve("finance").await.expect("resolve"), None);
        assert!(registry.list().await.expect("list").is_empty());
    }

    #[tokio::test]
    async fn a_harness_definition_shadows_a_same_id_config_entry() {
        let registry = registry_of(vec![synthetic("finance", AgentTier::Worker, &[])])
            .with_config(config_with(vec![custom_entry("finance", true)]));

        let def = registry
            .resolve("finance")
            .await
            .expect("resolve")
            .expect("present");
        assert_eq!(def.name, "finance display", "harness definition must win");
        assert_eq!(registry.list().await.expect("list").len(), 1);
    }

    // ── list stability ────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_is_stable_across_calls() {
        let registry = builtins();
        let first = registry.list().await.expect("list");
        let second = registry.list().await.expect("list");
        assert!(!first.is_empty());
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn is_usable_as_a_trait_object() {
        let registry: Box<dyn DefinitionRegistry> = Box::new(builtins());
        assert!(registry
            .resolve("orchestrator")
            .await
            .expect("resolve")
            .is_some());
    }

    #[tokio::test]
    async fn an_empty_catalogue_misses_everything_without_erroring() {
        let registry = registry_of(Vec::new());
        assert_eq!(registry.resolve("anything").await.expect("resolve"), None);
        assert!(registry.list().await.expect("list").is_empty());
        assert!(registry
            .delegates_for("anything")
            .await
            .expect("delegates")
            .is_empty());
    }
}
