//! RPC handler functions for `openhuman.skill_registry_*` controllers.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
use crate::core::Outcome;
use crate::skills::catalog::ops;
use crate::skills::ops_install::ScanAcknowledgement;

use super::controller_schemas::all_skill_registry_controller_schemas;
use super::wire_types::{
    CatalogParams, CatalogResult, CategoriesResult, EntryParams, InstallParams, InstallResult,
    SchemasResult, SourcesResult, UninstallParams, UninstallResult,
};

fn deserialize_params<T: serde::de::DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn registry_error(error: tinyskills::RegistryError) -> String {
    ops::registry_error_message(&error)
}

async fn catalog(method: &'static str, params: Map<String, Value>) -> Result<Value, String> {
    let query = deserialize_params::<CatalogParams>(params)?.into_query();
    tracing::debug!(
        method,
        text = %query.text,
        upstreams = ?query.upstreams,
        categories = ?query.categories,
        page = ?query.page,
        page_size = ?query.page_size,
        force_refresh = query.force_refresh,
        "[skill_registry][rpc] catalog read"
    );
    let page: CatalogResult = ops::catalog_page(&query).await.map_err(registry_error)?;
    tracing::debug!(
        method,
        total = page.total,
        returned = page.entries.len(),
        freshness = ?page.freshness,
        "[skill_registry][rpc] catalog result"
    );
    to_json(Outcome::new(page, Vec::new()))
}

pub(super) fn handle_browse(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(catalog("browse", params))
}

pub(super) fn handle_search(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(catalog("search", params))
}

pub(super) fn handle_sources(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        let facets = ops::catalog_facets().await.map_err(registry_error)?;
        to_json(Outcome::new(
            SourcesResult {
                sources: facets.upstreams.iter().map(|f| f.value.clone()).collect(),
                facets: facets.upstreams,
                freshness: facets.freshness,
            },
            Vec::new(),
        ))
    })
}

pub(super) fn handle_categories(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        let facets = ops::catalog_facets().await.map_err(registry_error)?;
        to_json(Outcome::new(
            CategoriesResult {
                categories: facets.categories.iter().map(|f| f.value.clone()).collect(),
                facets: facets.categories,
                freshness: facets.freshness,
            },
            Vec::new(),
        ))
    })
}

pub(super) fn handle_detail(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<EntryParams>(params)?;
        tracing::debug!(entry_id = %p.entry_id, "[skill_registry][rpc] detail");
        let detail = ops::catalog_detail(&p.entry_id)
            .await
            .map_err(registry_error)?;
        to_json(Outcome::new(detail, Vec::new()))
    })
}

pub(super) fn handle_install(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InstallParams>(params)?;
        tracing::info!(
            entry_id = %p.entry_id,
            acknowledged = p.acknowledged_digest.is_some(),
            "[skill_registry][rpc] install"
        );

        let workspace = crate::skills::schemas::resolve_workspace_dir().await;
        let outcome: InstallResult = ops::install_from_catalog(
            &workspace,
            &p.entry_id,
            ScanAcknowledgement::from_user_digest(p.acknowledged_digest.clone()),
        )
        .await
        .map_err(|error| error.to_string())?;
        tracing::info!(
            entry_id = %p.entry_id,
            status = outcome.status(),
            "[skill_registry][rpc] install result"
        );

        to_json(Outcome::new(outcome, Vec::new()))
    })
}

pub(super) fn handle_schemas(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        to_json(Outcome::new(
            SchemasResult {
                schemas: all_skill_registry_controller_schemas(),
            },
            Vec::new(),
        ))
    })
}

pub(super) fn handle_uninstall(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<UninstallParams>(params)?;
        tracing::info!(
            name = %payload.name,
            "[skill_registry][rpc] uninstall"
        );
        let workflow_params =
            crate::skills::ops_install::UninstallWorkflowParams { name: payload.name };
        let outcome = crate::skills::ops_install::uninstall_workflow(workflow_params, None)?;
        to_json(Outcome::new(
            UninstallResult {
                name: outcome.name,
                removed_path: outcome.removed_path,
                scope: outcome.scope,
            },
            Vec::new(),
        ))
    })
}
