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
    origin: tinyflows_catalog::DraftOrigin,
) -> Result<Outcome<tinyflows_catalog::FlowDraft>, String> {
    let draft = draft_store::create_draft(config, flow_id, name, graph, origin)
        .map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(draft, "draft created"))
}

/// Reads a draft by id (errors if it does not exist).
pub fn flows_draft_get(
    config: &Config,
    id: &str,
) -> Result<Outcome<tinyflows_catalog::FlowDraft>, String> {
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
) -> Result<Outcome<tinyflows_catalog::FlowDraft>, String> {
    let draft =
        draft_store::update_draft(config, id, name, graph, flow_id).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(draft, "draft updated"))
}

/// Lists all drafts, newest-updated first.
pub fn flows_draft_list(
    config: &Config,
) -> Result<Outcome<Vec<tinyflows_catalog::FlowDraft>>, String> {
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
    let draft = draft_store::get_draft(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("draft '{id}' not found"))?;

    tracing::debug!(
        target: "flows",
        draft_id = %id,
        promotes_to = draft.flow_id.as_deref().unwrap_or("<new flow>"),
        "[flows] flows_draft_promote: promoting draft through the create/update gates"
    );

    let outcome = match &draft.flow_id {
        Some(flow_id) => {
            flows_update(
                config,
                flow_id,
                Some(draft.name.clone()),
                Some(draft.graph.clone()),
                require_approval,
                None,
            )
            .await?
        }
        None => {
            flows_create(
                config,
                draft.name.clone(),
                draft.graph.clone(),
                require_approval.unwrap_or(false),
            )
            .await?
        }
    };

    // Only remove the draft once the flow write succeeded.
    if let Err(e) = draft_store::delete_draft(config, id) {
        tracing::warn!(target: "flows", draft_id = %id, error = %e, "[flows] flows_draft_promote: flow saved but draft file could not be removed");
    }
    Ok(outcome)
}
