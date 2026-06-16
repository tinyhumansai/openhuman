//! RPC handler functions for `openhuman.skill_registry_*` controllers.

use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
use crate::openhuman::skill_registry::ops;
use crate::rpc::RpcOutcome;

use super::controller_schemas::all_skill_registry_controller_schemas;
use super::wire_types::{
    BrowseParams, BrowseResult, CategoriesResult, InstallParams, InstallResult, SchemasResult,
    SearchParams, SearchResult, SourcesResult, UninstallParams, UninstallResult,
};

fn deserialize_params<T: serde::de::DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

pub(super) fn handle_browse(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<BrowseParams>(params)?;
        tracing::debug!(
            force_refresh = p.force_refresh,
            "[skill_registry][rpc] browse"
        );
        let entries = ops::browse_catalog(p.force_refresh).await?;
        tracing::debug!(count = entries.len(), "[skill_registry][rpc] browse result");
        to_json(RpcOutcome::new(BrowseResult { entries }, Vec::new()))
    })
}

pub(super) fn handle_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<SearchParams>(params)?;
        tracing::debug!(
            query = %p.query,
            source = ?p.source,
            category = ?p.category,
            "[skill_registry][rpc] search"
        );
        let entries =
            ops::search_catalog(&p.query, p.source.as_deref(), p.category.as_deref()).await?;
        tracing::debug!(count = entries.len(), "[skill_registry][rpc] search result");
        to_json(RpcOutcome::new(SearchResult { entries }, Vec::new()))
    })
}

pub(super) fn handle_sources(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        let sources = ops::list_sources().await?;
        to_json(RpcOutcome::new(SourcesResult { sources }, Vec::new()))
    })
}

pub(super) fn handle_categories(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        let categories = ops::list_categories().await?;
        to_json(RpcOutcome::new(CategoriesResult { categories }, Vec::new()))
    })
}

pub(super) fn handle_install(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InstallParams>(params)?;
        tracing::info!(
            entry_id = %p.entry_id,
            "[skill_registry][rpc] install"
        );

        let catalog = ops::browse_catalog(false).await?;
        let entry = catalog
            .iter()
            .find(|e| e.id == p.entry_id)
            .ok_or_else(|| {
                format!(
                    "entry '{}' not found in catalog. Run skill_registry_browse with force_refresh first.",
                    p.entry_id
                )
            })?;

        let workspace = crate::openhuman::workflows::schemas::resolve_workspace_dir().await;
        let outcome = ops::install_from_catalog(&workspace, entry).await?;

        to_json(RpcOutcome::new(
            InstallResult {
                url: outcome.url,
                stdout: outcome.stdout,
                stderr: outcome.stderr,
                new_skills: outcome.new_skills,
            },
            Vec::new(),
        ))
    })
}

pub(super) fn handle_schemas(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        to_json(RpcOutcome::new(
            SchemasResult {
                schemas: all_skill_registry_controller_schemas(),
            },
            Vec::new(),
        ))
    })
}

pub(super) fn handle_uninstall(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<UninstallParams>(params)?;
        tracing::info!(
            name = %payload.name,
            "[skill_registry][rpc] uninstall"
        );
        let workflow_params = crate::openhuman::workflows::ops_install::UninstallWorkflowParams {
            name: payload.name,
        };
        let outcome =
            crate::openhuman::workflows::ops_install::uninstall_workflow(workflow_params, None)?;
        to_json(RpcOutcome::new(
            UninstallResult {
                name: outcome.name,
                removed_path: outcome.removed_path,
                scope: outcome.scope,
            },
            Vec::new(),
        ))
    })
}
