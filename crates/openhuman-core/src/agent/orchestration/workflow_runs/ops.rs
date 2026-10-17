//! OpenHuman's workflow catalog and ledger read adapters.
//!
//! Definitions, validation, scheduling and fan-out live in
//! `tinyagents-orchestration`. This module retains only product catalog
//! selection, live agent lookup and the configured persistence root.

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::json;
use std::sync::Arc;
use tinyagents_orchestration::workflow::{
    WorkflowDefinition, WorkflowDefinitionListResponse, WorkflowPhase,
};

use tinyagents_orchestration::workflow::SessionWorkflowStore;
use tinyagents_session::run_ledger::{
    WorkflowRun, WorkflowRunListRequest, WorkflowRunListResponse, WorkflowRunUpsert,
};

use crate::config::Config;

pub const PARALLEL_RESEARCH_ID: &str = "parallel_research_cross_check";

/// Product-selected workflow catalog. `safetyTier` remains opaque data here;
/// the host's admission adapter remains responsible for policy enforcement.
pub fn builtin_definitions() -> Vec<WorkflowDefinition> {
    vec![WorkflowDefinition {
        id: PARALLEL_RESEARCH_ID.to_owned(),
        name: "Parallel research with cross-checking".to_owned(),
        description: "Decompose a question into angles, research them in parallel, cross-check the claims with a critic, then synthesize a cited report. Read-only.".to_owned(),
        phases: vec![
            WorkflowPhase { name: "decompose".into(), description: "Break the question into independent research angles.".into(), agent_ids: vec!["planner".into()], depends_on: vec![] },
            WorkflowPhase { name: "research".into(), description: "Research each angle in parallel.".into(), agent_ids: vec!["planner".into(), "planner".into()], depends_on: vec!["decompose".into()] },
            WorkflowPhase { name: "cross_check".into(), description: "Adversarially cross-check the gathered claims.".into(), agent_ids: vec!["critic".into()], depends_on: vec!["research".into()] },
            WorkflowPhase { name: "synthesize".into(), description: "Synthesize a single cited report.".into(), agent_ids: vec!["summarizer".into()], depends_on: vec!["cross_check".into()] },
        ],
        default_concurrency: 2,
        max_children: 8,
        extensions: BTreeMap::from([("safetyTier".to_owned(), json!("read_only"))]),
    }]
}

pub fn definition_by_id(id: &str) -> Option<WorkflowDefinition> {
    builtin_definitions()
        .into_iter()
        .find(|definition| definition.id == id)
}

pub fn list_definitions() -> WorkflowDefinitionListResponse {
    let definitions = builtin_definitions();
    WorkflowDefinitionListResponse {
        count: definitions.len(),
        definitions,
    }
}

pub fn list_runs(
    config: &Config,
    request: &WorkflowRunListRequest,
) -> Result<WorkflowRunListResponse> {
    let request = request.clone();
    Ok(crate::agent::session_store::with_ledger(
        &config.workspace_dir,
        |ledger| async move { ledger.list_workflow_runs(&request).await },
    )?)
}

pub fn get_run(config: &Config, id: &str) -> Result<Option<WorkflowRun>> {
    get_workflow_run(config, id)
}

/// The workflow run `id` on the calling tenant's run ledger.
pub(super) fn get_workflow_run(config: &Config, id: &str) -> Result<Option<WorkflowRun>> {
    let id = id.to_string();
    Ok(crate::agent::session_store::with_ledger(
        &config.workspace_dir,
        |ledger| async move { ledger.get_workflow_run(&id).await },
    )?)
}

/// A host lifecycle transition (stop, resume) fenced on the revision the
/// caller observed; `None` when another writer moved the row first.
pub(super) fn compare_and_swap_workflow_run_lifecycle(
    config: &Config,
    upsert: WorkflowRunUpsert,
    expected_revision: u64,
) -> Result<Option<WorkflowRun>> {
    Ok(crate::agent::session_store::with_ledger(
        &config.workspace_dir,
        |ledger| async move {
            ledger
                .compare_and_swap_workflow_run_lifecycle(upsert, expected_revision)
                .await
        },
    )?)
}

/// The engine's workflow store over the calling tenant's run ledger.
pub(super) fn workflow_store(config: &Config) -> Result<Arc<SessionWorkflowStore>> {
    let ledger = crate::agent::session_store::require_ledger(&config.workspace_dir)?;
    Ok(Arc::new(SessionWorkflowStore::new(ledger)?))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
