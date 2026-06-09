//! Workflow-definition catalog + read surface over durable workflow runs.
//!
//! PR1 scope: expose the builtin [`WorkflowDefinition`]s, validate them
//! (structure + agent existence), and read durable [`WorkflowRun`]s from
//! `session_db::run_ledger`. No execution engine yet — starting / stopping /
//! resuming runs lands in a follow-up PR.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::Result;

use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::config::Config;
use crate::openhuman::session_db::run_ledger::{
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

    let mut seen: HashSet<&str> = HashSet::new();
    for phase in &def.phases {
        if !seen.insert(phase.name.as_str()) {
            errors.push(DefinitionError::DuplicatePhase {
                name: phase.name.clone(),
            });
        }
        if phase.agent_ids.is_empty() {
            errors.push(DefinitionError::EmptyPhase {
                phase: phase.name.clone(),
            });
        }
    }

    let names: HashSet<&str> = def.phases.iter().map(|p| p.name.as_str()).collect();
    for phase in &def.phases {
        for dep in &phase.depends_on {
            if !names.contains(dep.as_str()) {
                errors.push(DefinitionError::UnknownDependency {
                    phase: phase.name.clone(),
                    depends_on: dep.clone(),
                });
            }
        }
    }

    if has_cycle(def) {
        errors.push(DefinitionError::CyclicDependency);
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

/// Kahn's-algorithm cycle check over the phase dependency graph. Edges that
/// point at unknown phases are ignored here (reported separately).
fn has_cycle(def: &WorkflowDefinition) -> bool {
    let names: HashSet<&str> = def.phases.iter().map(|p| p.name.as_str()).collect();
    let mut indegree: HashMap<&str, usize> =
        def.phases.iter().map(|p| (p.name.as_str(), 0)).collect();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for phase in &def.phases {
        for dep in &phase.depends_on {
            let dep = dep.as_str();
            if names.contains(dep) {
                // edge dep -> phase
                adjacency.entry(dep).or_default().push(phase.name.as_str());
                *indegree.entry(phase.name.as_str()).or_insert(0) += 1;
            }
        }
    }

    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&n, _)| n)
        .collect();
    let mut visited = 0usize;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        if let Some(children) = adjacency.get(node) {
            for &child in children {
                let entry = indegree.get_mut(child).expect("child in indegree");
                *entry -= 1;
                if *entry == 0 {
                    queue.push_back(child);
                }
            }
        }
    }
    // Compare against the unique-node count, not `def.phases.len()`: the graph
    // is keyed by unique phase names, so duplicate names (reported separately as
    // `DuplicatePhase`) would otherwise trip a false `CyclicDependency`.
    visited != indegree.len()
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
    list_workflow_runs(config, request)
}

/// Get one durable workflow run by id (delegates to the run ledger).
pub fn get_run(config: &Config, id: &str) -> Result<Option<WorkflowRun>> {
    log::debug!(target: "workflow_run", "[workflow_run] get_run.entry id={id}");
    get_workflow_run(config, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_def() -> WorkflowDefinition {
        definition_by_id(PARALLEL_RESEARCH_ID).expect("builtin present")
    }

    #[test]
    fn builtin_is_structurally_valid() {
        assert!(validate_structure(&good_def()).is_empty());
    }

    #[test]
    fn builtin_agents_pass_when_all_known() {
        // Treat the four referenced agents as registered.
        let known = ["planner", "researcher", "critic", "summarizer"];
        let errors = validate_agents(&good_def(), |id| known.contains(&id));
        assert!(errors.is_empty(), "unexpected: {errors:?}");
    }

    #[test]
    fn unknown_agent_is_reported() {
        let errors = validate_agents(&good_def(), |id| id == "researcher");
        // planner, critic, summarizer are unknown -> 3 errors.
        assert_eq!(errors.len(), 3);
        assert!(errors.iter().any(
            |e| matches!(e, DefinitionError::UnknownAgent { agent_id, .. } if agent_id == "planner")
        ));
    }

    #[test]
    fn no_phases_is_rejected() {
        let mut def = good_def();
        def.phases.clear();
        assert_eq!(validate_structure(&def), vec![DefinitionError::NoPhases]);
    }

    #[test]
    fn duplicate_and_empty_phase_are_reported() {
        let def = WorkflowDefinition {
            phases: vec![
                WorkflowPhase {
                    name: "a".into(),
                    description: String::new(),
                    agent_ids: vec!["researcher".into()],
                    depends_on: vec![],
                },
                WorkflowPhase {
                    name: "a".into(),
                    description: String::new(),
                    agent_ids: vec![],
                    depends_on: vec![],
                },
            ],
            ..good_def()
        };
        let errors = validate_structure(&def);
        assert!(errors.contains(&DefinitionError::DuplicatePhase { name: "a".into() }));
        assert!(errors.contains(&DefinitionError::EmptyPhase { phase: "a".into() }));
    }

    #[test]
    fn unknown_dependency_is_reported() {
        let def = WorkflowDefinition {
            phases: vec![WorkflowPhase {
                name: "only".into(),
                description: String::new(),
                agent_ids: vec!["researcher".into()],
                depends_on: vec!["ghost".into()],
            }],
            ..good_def()
        };
        let errors = validate_structure(&def);
        assert!(errors.contains(&DefinitionError::UnknownDependency {
            phase: "only".into(),
            depends_on: "ghost".into(),
        }));
    }

    #[test]
    fn cycle_is_detected() {
        let def = WorkflowDefinition {
            phases: vec![
                WorkflowPhase {
                    name: "a".into(),
                    description: String::new(),
                    agent_ids: vec!["researcher".into()],
                    depends_on: vec!["b".into()],
                },
                WorkflowPhase {
                    name: "b".into(),
                    description: String::new(),
                    agent_ids: vec!["researcher".into()],
                    depends_on: vec!["a".into()],
                },
            ],
            ..good_def()
        };
        assert!(validate_structure(&def).contains(&DefinitionError::CyclicDependency));
    }

    #[test]
    fn duplicate_phase_names_do_not_report_false_cycle() {
        let def = WorkflowDefinition {
            phases: vec![
                WorkflowPhase {
                    name: "a".into(),
                    description: String::new(),
                    agent_ids: vec!["researcher".into()],
                    depends_on: vec![],
                },
                WorkflowPhase {
                    name: "a".into(),
                    description: String::new(),
                    agent_ids: vec!["researcher".into()],
                    depends_on: vec![],
                },
            ],
            ..good_def()
        };
        let errors = validate_structure(&def);
        assert!(errors.contains(&DefinitionError::DuplicatePhase { name: "a".into() }));
        assert!(
            !errors.contains(&DefinitionError::CyclicDependency),
            "duplicate names must not trip a false cycle: {errors:?}"
        );
    }

    #[test]
    fn zero_concurrency_is_rejected() {
        let def = WorkflowDefinition {
            default_concurrency: 0,
            max_children: 0,
            ..good_def()
        };
        assert!(
            validate_structure(&def).contains(&DefinitionError::InvalidConcurrency {
                default_concurrency: 0,
                max_children: 0,
            })
        );
    }

    #[test]
    fn list_definitions_returns_builtins() {
        let resp = list_definitions();
        assert_eq!(resp.count, 1);
        assert_eq!(resp.definitions[0].id, PARALLEL_RESEARCH_ID);
    }
}
