use super::super::*;
use super::{read_required, to_json};
use crate::core::runtime::DispatchCtx;

pub(in crate::flows::schemas) fn handle_draft_create(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let name = read_required::<String>(&params, "name")?;
        let graph = read_required::<Value>(&params, "graph")?;
        let flow_id = params
            .get("flow_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let origin = params
            .get("origin")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_value(Value::String(s.to_string())).ok())
            .unwrap_or(tinyflows_bus::catalog::DraftOrigin::Canvas);
        to_json(ops::flows_draft_create(
            &config, flow_id, name, graph, origin,
        )?)
    })
}

pub(in crate::flows::schemas) fn handle_draft_get(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_draft_get(&config, id.trim())?)
    })
}

pub(in crate::flows::schemas) fn handle_draft_update(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        let name = params
            .get("name")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| format!("invalid 'name': {e}"))?;
        let graph = params.get("graph").filter(|v| !v.is_null()).cloned();
        // A present `flow_id` (even null) re-links the draft; absent leaves it.
        let flow_id = parse_draft_update_flow_id(&params)?;
        to_json(ops::flows_draft_update(
            &config,
            id.trim(),
            name,
            graph,
            flow_id,
        )?)
    })
}

pub(in crate::flows::schemas) fn handle_draft_list(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(ops::flows_draft_list(&config)?)
    })
}

pub(in crate::flows::schemas) fn handle_draft_delete(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_draft_delete(&config, id.trim())?)
    })
}

pub(in crate::flows::schemas) fn handle_draft_promote(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        let require_approval = params.get("require_approval").and_then(Value::as_bool);
        to_json(
            ops::flows_draft_promote_with_context(&ctx, &config, id.trim(), require_approval)
                .await?,
        )
    })
}

/// Parses `draft_update`'s `flow_id` param (R-m7). The outer `Option`
/// mirrors `ops::flows_draft_update`'s "present vs absent" contract — absent
/// leaves the draft's existing link untouched; the inner `Option` is the new
/// link (`None` unlinks).
///
/// A present-but-non-string `flow_id` (a number, or an object from a buggy
/// client) is REJECTED rather than silently coerced into `Some(None)` via
/// `Value::as_str()` returning `None` on a type mismatch — that shape used
/// to be indistinguishable from an explicit `flow_id: null` unlink, and
/// `update_draft` treats `Some(None)` as exactly that: unlinking the draft
/// from its flow. A later `draft_promote` then creates a brand-new flow
/// instead of updating the one the caller actually meant.
pub(in crate::flows::schemas) fn parse_draft_update_flow_id(
    params: &Map<String, Value>,
) -> Result<Option<Option<String>>, String> {
    match params.get("flow_id") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => {
            let s = s.trim();
            Ok(Some(if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }))
        }
        Some(other) => Err(format!(
            "invalid 'flow_id': expected a string or null, got {other}"
        )),
    }
}
