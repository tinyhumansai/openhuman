//! Declarative workflow-definition types for durable multi-agent runs (#3375).
//!
//! A [`WorkflowDefinition`] is a static, declarative phase graph — NOT an
//! arbitrary script. Each phase names the agents it fans out to and the phases
//! it depends on, so the runtime (added in a follow-up PR) can schedule phases
//! in dependency order with bounded concurrency. Keeping definitions
//! declarative avoids the security surface of arbitrary in-process script
//! execution.
//!
//! This PR ships the definition model + the read surface (list definitions,
//! list/get durable runs from `session_db::run_ledger`). The live execution
//! engine is deferred to a follow-up.

use serde::Serialize;

/// Safety tier of a workflow — governs what its child agents may do.
///
/// Only [`WorkflowSafetyTier::ReadOnly`] workflows ship first; edit-capable
/// tiers gate on explicit user approval once the engine lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowSafetyTier {
    /// Child agents may only read / research — no file writes, no external
    /// side effects beyond fetching.
    ReadOnly,
    /// Child agents may take standard non-destructive actions.
    Standard,
    /// Child agents may edit files (requires worktree isolation + approval).
    EditCapable,
}

impl WorkflowSafetyTier {
    /// Stable wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowSafetyTier::ReadOnly => "read_only",
            WorkflowSafetyTier::Standard => "standard",
            WorkflowSafetyTier::EditCapable => "edit_capable",
        }
    }
}

/// One phase of a workflow: a set of agents fanned out in parallel once every
/// phase it `depends_on` has completed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPhase {
    /// Phase name, unique within a definition; referenced by `depends_on`.
    pub name: String,
    /// Human-readable purpose of the phase.
    pub description: String,
    /// Agent definition ids spawned (in parallel) during this phase.
    pub agent_ids: Vec<String>,
    /// Names of phases that must complete before this one starts.
    pub depends_on: Vec<String>,
}

/// A declarative, repeatable workflow definition.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDefinition {
    /// Stable definition id (e.g. `parallel_research_cross_check`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the workflow does and when to use it.
    pub description: String,
    /// Ordered list of phases (dependency edges encoded via `depends_on`).
    pub phases: Vec<WorkflowPhase>,
    /// Default max agents run concurrently within a phase.
    pub default_concurrency: u32,
    /// Hard cap on total child agents across the whole run.
    pub max_children: u32,
    /// Safety tier (drives approval + isolation policy at run time).
    pub safety_tier: WorkflowSafetyTier,
}

/// Response wrapper for the list-definitions controller.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDefinitionListResponse {
    /// Available workflow definitions (builtins for now).
    pub definitions: Vec<WorkflowDefinition>,
    /// Total count.
    pub count: usize,
}

/// A validation problem found in a [`WorkflowDefinition`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DefinitionError {
    /// A phase referenced an agent id not present in the registry.
    UnknownAgent { phase: String, agent_id: String },
    /// A `depends_on` named a phase that does not exist.
    UnknownDependency { phase: String, depends_on: String },
    /// Two phases share the same name.
    DuplicatePhase { name: String },
    /// A phase has no agents.
    EmptyPhase { phase: String },
    /// The dependency graph contains a cycle.
    CyclicDependency,
    /// The definition declares no phases.
    NoPhases,
    /// `default_concurrency` or `max_children` is zero — an executor would
    /// deadlock or trivially fail to launch any child.
    InvalidConcurrency {
        default_concurrency: u32,
        max_children: u32,
    },
}
