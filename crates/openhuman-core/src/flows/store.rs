//! Host client for the TinyFlows module's Catalog contract.
//!
//! The module owns catalog schemas, migrations, revisions, and update rules.
//! This file only translates the existing core-facing flow types to Catalog
//! requests and back. Run lifecycle and checkpoint records are owned by the
//! Runs adapter, not this definition client.

use anyhow::{anyhow, Result};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tinyflows_bus::catalog::{
    self, DeleteWorkflowResult, FlowUpdateError as BusUpdateError, GetRevisionRequest,
    ListRevisionsRequest, ListWorkflowsResult, PutWorkflowRequest, PutWorkflowResponse,
    PutWorkflowResult, SetEnabledRequest, Workflow as Flow, WorkflowRevision,
};
use tinyflows_bus::catalog::{FlowSuggestion, SuggestionStatus};

use crate::config::Config;
use crate::modules::tinyflows as module;

pub(crate) fn call_catalog<T, A>(config: &Config, member: &'static str, args: A) -> Result<T>
where
    T: DeserializeOwned + Send + 'static,
    A: Serialize + Send + Sync + 'static,
{
    let config = config.clone();
    crate::storage::block_on_anyhow(async move {
        let proxy = module::catalog_proxy(&config)
            .await
            .map_err(anyhow::Error::msg)?;
        proxy
            .call(member, &args)
            .await
            .map_err(|error| anyhow!(error))
    })
}

fn put_result(response: PutWorkflowResponse) -> Result<PutWorkflowResult> {
    match response {
        PutWorkflowResponse::Ok(result) => Ok(result),
        PutWorkflowResponse::Error(error) => Err(anyhow!(match error {
            BusUpdateError::NotFound { message }
            | BusUpdateError::VersionConflict { message, .. }
            | BusUpdateError::Store { message } => message,
        })),
    }
}

/// Duplicate a workflow through the module's Catalog contract.
pub fn insert_duplicate_flow(config: &Config, source: &Flow, new_name: String) -> Result<Flow> {
    let storage = super::catalog_storage::register_blocking(config)?;
    let request = PutWorkflowRequest::Duplicate {
        storage,
        id: source.id.clone(),
        name: new_name,
    };
    let response: PutWorkflowResponse =
        call_catalog(config, catalog::members::PUT_WORKFLOW, (request,))?;
    put_result(response).map(|result| result.workflow)
}

/// Create a workflow through the module's Catalog contract.
pub fn create_flow(
    config: &Config,
    name: String,
    graph: Value,
    require_approval: bool,
    enabled: bool,
) -> Result<PutWorkflowResult> {
    let storage = super::catalog_storage::register_blocking(config)?;
    let request = PutWorkflowRequest::Create {
        storage,
        name,
        graph,
        require_approval,
    };
    let response: PutWorkflowResponse =
        call_catalog(config, catalog::members::PUT_WORKFLOW, (request,))?;
    let mut result = put_result(response)?;
    // This is an explicit caller choice to start disabled. Catalog still
    // authoritatively prevents automatic triggers from being born enabled.
    if !enabled && result.workflow.enabled {
        result.workflow = set_enabled(config, &result.workflow.id, false)?;
    }
    Ok(result)
}

/// Read one workflow by id.
pub fn get_flow(config: &Config, id: &str) -> Result<Option<Flow>> {
    let storage = super::catalog_storage::register_blocking(config)?;
    let workflow: Option<Flow> = call_catalog(
        config,
        catalog::members::GET_WORKFLOW,
        (storage, id.to_owned()),
    )?;
    Ok(workflow)
}

/// List workflows and their full graphs through the module's catalog client.
pub fn list_flows(config: &Config) -> Result<(Vec<Flow>, usize)> {
    let storage = super::catalog_storage::register_blocking(config)?;
    let listed: ListWorkflowsResult =
        call_catalog(config, catalog::members::LIST_WORKFLOWS, (storage.clone(),))?;
    let mut flows = Vec::with_capacity(listed.workflows.len());
    let mut skipped = listed.skipped_rows;
    for summary in listed.workflows {
        let workflow: Option<Flow> = call_catalog(
            config,
            catalog::members::GET_WORKFLOW,
            (storage.clone(), summary.id),
        )?;
        match workflow {
            Some(flow) => flows.push(flow),
            None => skipped += 1,
        }
    }
    Ok((flows, skipped))
}

/// List enabled workflows through the same module-owned catalog surface.
pub fn list_enabled_flows(config: &Config) -> Result<(Vec<Flow>, usize)> {
    let (flows, skipped) = list_flows(config)?;
    Ok((
        flows.into_iter().filter(|flow| flow.enabled).collect(),
        skipped,
    ))
}

/// Remove a workflow through the module's Catalog contract.
pub fn remove_flow(config: &Config, id: &str) -> Result<()> {
    let storage = super::catalog_storage::register_blocking(config)?;
    let _: DeleteWorkflowResult = call_catalog(
        config,
        catalog::members::DELETE_WORKFLOW,
        (storage, id.to_owned()),
    )?;
    Ok(())
}

/// Enable or disable a workflow through the module's Catalog contract.
pub fn set_enabled(config: &Config, id: &str, enabled: bool) -> Result<Flow> {
    let storage = super::catalog_storage::register_blocking(config)?;
    let request = SetEnabledRequest {
        storage,
        id: id.to_owned(),
        enabled,
    };
    let workflow: Flow = call_catalog(config, catalog::members::SET_ENABLED, (request,))?;
    Ok(workflow)
}

/// Update a workflow while preserving the module's optimistic concurrency.
#[allow(clippy::too_many_arguments)]
pub fn update_flow_graph(
    config: &Config,
    id: &str,
    name: String,
    graph: Value,
    require_approval: bool,
    _enabled_override: Option<bool>,
    force_disarm_if_automatic: bool,
    expected_updated_at: Option<&str>,
) -> std::result::Result<PutWorkflowResult, BusUpdateError> {
    let storage = super::catalog_storage::register_blocking(config).map_err(|error| {
        BusUpdateError::Store {
            message: error.to_string(),
        }
    })?;
    let request = PutWorkflowRequest::Update {
        storage,
        id: id.to_owned(),
        name,
        graph,
        require_approval,
        expected_version: expected_updated_at.map(str::to_owned),
        force_disarm_automatic: force_disarm_if_automatic,
    };
    let response: PutWorkflowResponse =
        call_catalog(config, catalog::members::PUT_WORKFLOW, (request,)).map_err(|error| {
            BusUpdateError::Store {
                message: error.to_string(),
            }
        })?;
    match response {
        PutWorkflowResponse::Ok(result) => Ok(result),
        PutWorkflowResponse::Error(error) => Err(error),
    }
}

/// List a workflow's committed revisions.
pub fn list_revisions(
    config: &Config,
    flow_id: &str,
    limit: usize,
) -> Result<Vec<WorkflowRevision>> {
    let storage = super::catalog_storage::register_blocking(config)?;
    let request = ListRevisionsRequest {
        storage,
        flow_id: flow_id.to_owned(),
        limit,
    };
    let revisions: Vec<WorkflowRevision> =
        call_catalog(config, catalog::members::LIST_REVISIONS, (request,))?;
    Ok(revisions)
}

/// Read one workflow revision by id.
pub fn revision_by_id(
    config: &Config,
    flow_id: &str,
    revision_id: &str,
) -> Result<Option<WorkflowRevision>> {
    let storage = super::catalog_storage::register_blocking(config)?;
    let request = GetRevisionRequest {
        storage,
        flow_id: flow_id.to_owned(),
        revision_id: revision_id.to_owned(),
    };
    let revision: Option<WorkflowRevision> =
        call_catalog(config, catalog::members::GET_REVISION, (request,))?;
    Ok(revision)
}

/// Upsert module-owned discovery suggestions.
pub fn upsert_suggestions(config: &Config, suggestions: &[FlowSuggestion]) -> Result<usize> {
    let storage = super::catalog_storage::register_blocking(config)?;
    let request = catalog::UpsertSuggestionsRequest {
        storage,
        suggestions: suggestions.to_vec(),
    };
    call_catalog(config, catalog::members::UPSERT_SUGGESTIONS, (request,))
}

/// List module-owned discovery suggestions.
pub fn list_suggestions(
    config: &Config,
    status: Option<SuggestionStatus>,
    limit: usize,
) -> Result<Vec<FlowSuggestion>> {
    let storage = super::catalog_storage::register_blocking(config)?;
    let request = catalog::ListSuggestionsRequest {
        storage,
        status,
        limit,
    };
    let suggestions: Vec<FlowSuggestion> =
        call_catalog(config, catalog::members::LIST_SUGGESTIONS, (request,))?;
    Ok(suggestions)
}

/// Set a suggestion's module-owned lifecycle status.
pub fn set_suggestion_status(config: &Config, id: &str, status: SuggestionStatus) -> Result<bool> {
    let storage = super::catalog_storage::register_blocking(config)?;
    let request = catalog::SetSuggestionStatusRequest {
        storage,
        id: id.to_owned(),
        status,
    };
    call_catalog(config, catalog::members::SET_SUGGESTION_STATUS, (request,))
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
