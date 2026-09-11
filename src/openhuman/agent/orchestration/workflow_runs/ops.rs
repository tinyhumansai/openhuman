//! Workflow-definition catalog + read surface over durable workflow runs.
//!
//! PR1 scope: expose the builtin [`WorkflowDefinition`]s, validate them
//! (structure + agent existence), and read durable [`WorkflowRun`]s from
//! `tinyagents_session::run_ledger`. No execution engine yet — starting / stopping /
//! resuming runs lands in a follow-up PR.

use anyhow::Result;

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::config::Config;
use tinyagents_graph::dag::{validate_dag, DagIssue, DagNode};
use tinyagents_session::run_ledger::{
    get_workflow_run, list_workflow_runs, WorkflowRun, WorkflowRunListRequest,
    WorkflowRunListResponse,
};

use super::types::{
    DefinitionError, WorkflowDefinition, WorkflowDefinitionListResponse, WorkflowPhase,
    WorkflowSafetyTier,
};

/// Id of the first shipped (read-only) workflow.
pub const PARALLEL_RESEARCH_ID: &str = "parallel_research_cross_check";

/// All builtin workflow definitions.
///
/// First (and only) shipped workflow is a read-only "parallel research with
/// cross-checking" pipeline: decompose the question, fan out researchers,
/// have a critic cross-check the claims, then synthesize a cited report.
pub fn builtin_definitions() -> Vec<WorkflowDefinition> {
    vec![WorkflowDefinition {
        id: PARALLEL_RESEARCH_ID.to_string(),
        name: "Parallel research with cross-checking".to_string(),
        description: "Decompose a question into angles, research them in parallel, cross-check \
                      the claims with a critic, then synthesize a cited report. Read-only."
            .to_string(),
        phases: vec![
            WorkflowPhase {
                name: "decompose".to_string(),
                description: "Break the question into independent research angles.".to_string(),
                agent_ids: vec!["planner".to_string()],
                depends_on: vec![],
            },
            WorkflowPhase {
                name: "research".to_string(),
                description: "Research each angle in parallel.".to_string(),
                agent_ids: vec!["researcher".to_string(), "researcher".to_string()],
                depends_on: vec!["decompose".to_string()],
            },
            WorkflowPhase {
                name: "cross_check".to_string(),
                description: "Adversarially cross-check the gathered claims.".to_string(),
                agent_ids: vec!["critic".to_string()],
                depends_on: vec!["research".to_string()],
            },
            WorkflowPhase {
                name: "synthesize".to_string(),
                description: "Synthesize a single cited report.".to_string(),
                agent_ids: vec!["summarizer".to_string()],
                depends_on: vec!["cross_check".to_string()],
            },
        ],
        default_concurrency: 2,
        max_children: 8,
        safety_tier: WorkflowSafetyTier::ReadOnly,
    }]
}

/// Look up one builtin definition by id.
pub fn definition_by_id(id: &str) -> Option<WorkflowDefinition> {
    builtin_definitions().into_iter().find(|d| d.id == id)
}

/// List available workflow definitions (builtins for now).
pub fn list_definitions() -> WorkflowDefinitionListResponse {
    let definitions = builtin_definitions();
    WorkflowDefinitionListResponse {
        count: definitions.len(),
        definitions,
    }
}

/// Validate a definition's structure (registry-independent).
///
/// Checks: at least one phase; unique phase names; non-empty phases;
/// `depends_on` references existing phases; no dependency cycles.
pub fn validate_structure(def: &WorkflowDefinition) -> Vec<DefinitionError> {
    log::debug!(
        target: "workflow_run",
        "[workflow_run] validate_structure.entry id={} phases={}",
        def.id,
        def.phases.len()
    );
    let mut errors = Vec::new();
    if def.phases.is_empty() {
        errors.push(DefinitionError::NoPhases);
        log::debug!(target: "workflow_run", "[workflow_run] validate_structure.exit id={} errors=1 reason=no_phases", def.id);
        return errors;
    }

    for phase in &def.phases {
        if phase.agent_ids.is_empty() {
            errors.push(DefinitionError::EmptyPhase {
                phase: phase.name.clone(),
            });
        }
    }

    // Unique names, landed `depends_on` edges and acyclicity are the shared
    // dependency-DAG question; `tinyagents_graph::dag` owns the algorithm and
    // this maps its structural issues back onto the host's error vocabulary.
    // A self-dependency arrives as `DagIssue::Cycle`, which is what the
    // hand-rolled Kahn pass here reported too.
    let nodes: Vec<DagNode<'_>> = def
        .phases
        .iter()
        .map(|p| DagNode::new(p.name.as_str(), p.depends_on.iter().map(String::as_str)))
        .collect();
    for issue in validate_dag(&nodes) {
        errors.push(match issue {
            DagIssue::DuplicateNode { id } => DefinitionError::DuplicatePhase { name: id },
            DagIssue::UnknownDependency { node, depends_on } => {
                DefinitionError::UnknownDependency {
                    phase: node,
                    depends_on,
                }
            }
            DagIssue::Cycle => DefinitionError::CyclicDependency,
        });
    }

    if def.default_concurrency == 0 || def.max_children == 0 {
        errors.push(DefinitionError::InvalidConcurrency {
            default_concurrency: def.default_concurrency,
            max_children: def.max_children,
        });
    }

    log::debug!(
        target: "workflow_run",
        "[workflow_run] validate_structure.exit id={} errors={}",
        def.id,
        errors.len()
    );
    errors
}

/// Validate that every agent referenced by a definition is resolvable through
/// the provided lookup. Kept generic so it is testable without the global
/// registry.
pub fn validate_agents<F>(def: &WorkflowDefinition, is_known: F) -> Vec<DefinitionError>
where
    F: Fn(&str) -> bool,
{
    log::debug!(
        target: "workflow_run",
        "[workflow_run] validate_agents.entry id={} phases={}",
        def.id,
        def.phases.len()
    );
    let mut errors = Vec::new();
    for phase in &def.phases {
        for agent_id in &phase.agent_ids {
            if !is_known(agent_id) {
                errors.push(DefinitionError::UnknownAgent {
                    phase: phase.name.clone(),
                    agent_id: agent_id.clone(),
                });
            }
        }
    }
    log::debug!(
        target: "workflow_run",
        "[workflow_run] validate_agents.exit id={} unknown={}",
        def.id,
        errors.len()
    );
    errors
}

/// Full validation against the live agent registry.
///
/// Always runs the structural checks. Agent-existence checks run only when the
/// registry is initialized, so callers in a registry-less context (e.g. early
/// boot, some tests) are not given false `UnknownAgent` errors.
pub fn validate_definition(def: &WorkflowDefinition) -> Vec<DefinitionError> {
    log::debug!(target: "workflow_run", "[workflow_run] validate_definition.entry id={}", def.id);
    let mut errors = validate_structure(def);
    match AgentDefinitionRegistry::global() {
        Some(registry) => {
            errors.extend(validate_agents(def, |id| registry.get(id).is_some()));
        }
        None => {
            log::debug!(
                target: "workflow_run",
                "[workflow_run][registry] validate_definition.skip_agents id={} reason=registry_uninitialized",
                def.id
            );
        }
    }
    log::debug!(
        target: "workflow_run",
        "[workflow_run] validate_definition.exit id={} errors={}",
        def.id,
        errors.len()
    );
    errors
}

/// List durable workflow runs (delegates to the run ledger).
pub fn list_runs(
    config: &Config,
    request: &WorkflowRunListRequest,
) -> Result<WorkflowRunListResponse> {
    log::debug!(
        target: "workflow_run",
        "[workflow_run] list_runs.entry definition={:?} status={:?}",
        request.definition_id,
        request.status
    );
    Ok(list_workflow_runs(&config.workspace_dir, request)?)
}

/// Get one durable workflow run by id (delegates to the run ledger).
pub fn get_run(config: &Config, id: &str) -> Result<Option<WorkflowRun>> {
    log::debug!(target: "workflow_run", "[workflow_run] get_run.entry id={id}");
    Ok(get_workflow_run(&config.workspace_dir, id)?)
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
