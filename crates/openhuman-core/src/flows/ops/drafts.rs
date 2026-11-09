use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Core-managed local drafts (F5) — the shared agent/canvas working copy
// ─────────────────────────────────────────────────────────────────────────────

/// Creates a new draft (a durable, non-live working copy) from a graph.
pub fn flows_draft_create(
    config: &Config,
    flow_id: Option<String>,
    name: String,
    graph: Value,
    origin: tinyflows_bus::catalog::DraftOrigin,
) -> Result<Outcome<tinyflows_bus::catalog::FlowDraft>, String> {
    let draft = draft_store::create_draft(config, flow_id, name, graph, origin)
        .map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(draft, "draft created"))
}

/// Create a draft from the Copilot bus's portable JSON payload.
pub(crate) fn flows_draft_create_from_copilot(
    config: &Config,
    flow_id: Option<String>,
    name: String,
    graph: Value,
    origin: Value,
) -> Result<Value, String> {
    let origin =
        serde_json::from_value(origin).map_err(|error| format!("bad `origin`: {error}"))?;
    let draft = flows_draft_create(config, flow_id, name, graph, origin)?.value;
    serde_json::to_value(draft).map_err(|error| error.to_string())
}

/// Reads a draft by id (errors if it does not exist).
pub fn flows_draft_get(
    config: &Config,
    id: &str,
) -> Result<Outcome<tinyflows_bus::catalog::FlowDraft>, String> {
    let draft = draft_store::get_draft(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("draft '{id}' not found"))?;
    Ok(Outcome::single_log(draft, format!("draft loaded: {id}")))
}

/// Patches a draft's `name`/`graph`/`flow_id` (any `Some` applied) and bumps
/// `updated_at`.
pub fn flows_draft_update(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph: Option<Value>,
    flow_id: Option<Option<String>>,
) -> Result<Outcome<tinyflows_bus::catalog::FlowDraft>, String> {
    let draft =
        draft_store::update_draft(config, id, name, graph, flow_id).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(draft, "draft updated"))
}

/// Lists all drafts, newest-updated first.
pub fn flows_draft_list(
    config: &Config,
) -> Result<Outcome<Vec<tinyflows_bus::catalog::FlowDraft>>, String> {
    let drafts = draft_store::list_drafts(config).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(drafts, "drafts listed"))
}

/// Deletes a draft by id (idempotent — reports whether a file was removed).
pub fn flows_draft_delete(config: &Config, id: &str) -> Result<Outcome<Value>, String> {
    let deleted = draft_store::delete_draft(config, id).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(
        json!({ "id": id, "deleted": deleted }),
        "draft deleted",
    ))
}

/// Promotes a draft into a saved flow, then removes the draft file.
///
/// Runs the SAME create/update gates as a normal save (structural validation,
/// the forced `require_approval` floor for side-effect graphs, born-disabled
/// for automatic triggers) — a draft is never a back-door around them. A draft
/// with a `flow_id` updates that flow; otherwise it creates a new one. The
/// draft file is deleted only on a successful promote.
pub async fn flows_draft_promote(
    config: &Config,
    id: &str,
    require_approval: Option<bool>,
) -> Result<Outcome<Flow>, String> {
    flows_draft_promote_with_context(
        &crate::core::runtime::DispatchCtx::ambient(),
        config,
        id,
        require_approval,
    )
    .await
}

pub(crate) async fn flows_draft_promote_with_context(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    id: &str,
    require_approval: Option<bool>,
) -> Result<Outcome<Flow>, String> {
    ctx.enter(async move {
        let callbacks = crate::flows::authoring_host::register(Arc::new(config.clone()), ctx.clone())
            .await?;
        let storage = crate::flows::catalog_storage::register(ctx, config).await?;
        let proxy = crate::modules::tinyflows::catalog_proxy(config).await?;
        let response: tinyflows_bus::catalog::PromoteDraftResponse = proxy
            .call(
                tinyflows_bus::catalog::members::PROMOTE_DRAFT,
                &(
                    tinyflows_bus::catalog::PromoteDraftRequest {
                        storage: storage.storage(),
                        host: callbacks.draft_promotion.clone(),
                        id: id.to_owned(),
                        require_approval,
                    },
                ),
            )
            .await
            .map_err(|error| error.to_string())?;

        match response {
            tinyflows_bus::catalog::PromoteDraftResponse::Error(error) => Err(error.message),
            tinyflows_bus::catalog::PromoteDraftResponse::Ok(result) => {
                let flow: Flow = result.workflow;
                let mut logs = result
                    .metadata
                    .get("logs")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if let Some(warning) = result.cleanup_warning {
                    tracing::warn!(target: "flows", draft_id = %id, %warning, "[flows] promoted draft cleanup failed");
                    logs.push(warning);
                }
                Ok(Outcome::new(flow, logs))
            }
        }
    })
    .await
}
