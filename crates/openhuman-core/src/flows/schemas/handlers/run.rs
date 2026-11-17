use super::super::*;
use super::{read_required, to_json};
use crate::core::runtime::DispatchCtx;

pub(in crate::flows::schemas) fn handle_run(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        let input = params.get("input").cloned().unwrap_or(Value::Null);
        let inputs = read_declared_inputs(&params)?;
        to_json(
            ops::flows_run(
                &ctx,
                &config,
                id.trim(),
                input,
                inputs,
                tinyflows_bus::runs::FlowRunTrigger::Rpc,
            )
            .await?,
        )
    })
}

pub(in crate::flows::schemas) fn handle_run_detached(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        let input = params.get("input").cloned().unwrap_or(Value::Null);
        let inputs = read_declared_inputs(&params)?;
        to_json(
            ops::flows_run_detached(
                &ctx,
                &config,
                id.trim(),
                input,
                inputs,
                tinyflows_bus::runs::FlowRunTrigger::Rpc,
            )
            .await?,
        )
    })
}

/// Reads the optional `inputs` param — values for the flow's declared workflow
/// inputs, keyed by name.
///
/// Absent or `null` means "supplied nothing", which is valid for a flow whose
/// inputs are all optional or defaulted. A present-but-non-object value is a
/// caller error rejected here, before it reaches `ops`, so the message names the
/// parameter rather than surfacing as a confusing per-input complaint.
pub(in crate::flows::schemas) fn read_declared_inputs(
    params: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    match params.get("inputs") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(other) => Err(format!(
            "param 'inputs' must be an object keyed by declared input name, got {}",
            match other {
                Value::Array(_) => "an array",
                Value::String(_) => "a string",
                Value::Number(_) => "a number",
                Value::Bool(_) => "a boolean",
                _ => "a non-object",
            }
        )),
    }
}

pub(in crate::flows::schemas) fn handle_resume(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        let thread_id = read_required::<String>(&params, "thread_id")?;
        let approvals: Vec<String> = params
            .get("approvals")
            .filter(|v| !v.is_null())
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("invalid 'approvals': {e}"))?
            .unwrap_or_default();
        let rejections: Vec<String> = params
            .get("rejections")
            .filter(|v| !v.is_null())
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("invalid 'rejections': {e}"))?
            .unwrap_or_default();
        to_json(
            ops::flows_resume(
                &ctx,
                &config,
                id.trim(),
                thread_id.trim(),
                approvals,
                rejections,
            )
            .await?,
        )
    })
}

pub(in crate::flows::schemas) fn handle_cancel_run(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let run_id = read_required::<String>(&params, "run_id")?;
        to_json(ops::flows_cancel_run(&config, run_id.trim()).await?)
    })
}

pub(in crate::flows::schemas) fn handle_list_runs(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(20);
        to_json(ops::flows_list_runs(&config, id.trim(), limit).await?)
    })
}

pub(in crate::flows::schemas) fn handle_list_all_runs(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(100);
        to_json(ops::flows_list_all_runs(&config, limit).await?)
    })
}

pub(in crate::flows::schemas) fn handle_get_run(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let run_id = read_required::<String>(&params, "run_id")?;
        to_json(ops::flows_get_run(&config, run_id.trim()).await?)
    })
}

pub(in crate::flows::schemas) fn handle_prune_runs(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_prune_runs(&config, id.trim()).await?)
    })
}
