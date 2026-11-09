use super::super::*;
use super::{read_required, to_json};
use crate::core::runtime::DispatchCtx;

pub(in crate::flows::schemas) fn handle_create(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let name = read_required::<String>(&params, "name")?;
        let graph = read_required::<Value>(&params, "graph")?;
        let require_approval = params
            .get("require_approval")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Opt-in strict mode (F3): run the same author hard-gates an agent save
        // must pass, before persisting. Default off — the human canvas save
        // path stays permissive.
        if params
            .get("strict")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            ops::strict_gate(&ctx, &config, &graph).await?;
        }
        to_json(ops::flows_create_with_context(&ctx, &config, name, graph, require_approval).await?)
    })
}

pub(in crate::flows::schemas) fn handle_validate(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let graph = read_required::<Value>(&params, "graph")?;
        to_json(ops::flows_validate(&ctx, &config, graph).await)
    })
}

pub(in crate::flows::schemas) fn handle_import(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let graph = read_required::<Value>(&params, "graph")?;
        let format = params
            .get("format")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value::<String>(v.clone()))
            .transpose()
            .map_err(|e| format!("invalid 'format': {e}"))?;
        to_json(ops::flows_import(&ctx, &config, graph, format).await?)
    })
}

pub(in crate::flows::schemas) fn handle_duplicate(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_duplicate(&config, id.trim()).await?)
    })
}

pub(in crate::flows::schemas) fn handle_get(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_get(&config, id.trim()).await?)
    })
}

pub(in crate::flows::schemas) fn handle_list(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(ops::flows_list(&config).await?)
    })
}

pub(in crate::flows::schemas) fn handle_list_connections(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(ops::flows_list_connections(&ctx, &config).await?)
    })
}

pub(in crate::flows::schemas) fn handle_update(
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
        let require_approval = params.get("require_approval").and_then(Value::as_bool);
        let expected_version = params
            .get("expected_version")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        // Opt-in strict mode (F3): when a new graph is supplied, run the same
        // author hard-gates an agent save must pass, before persisting.
        if params
            .get("strict")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            if let Some(graph_json) = graph.as_ref() {
                ops::strict_gate(&ctx, &config, graph_json).await?;
            }
        }
        to_json(
            ops::flows_update_with_context(
                &ctx,
                &config,
                id.trim(),
                name,
                graph,
                require_approval,
                expected_version,
            )
            .await?,
        )
    })
}

pub(in crate::flows::schemas) fn handle_delete(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_delete(&config, id.trim()).await?)
    })
}

pub(in crate::flows::schemas) fn handle_set_enabled(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        let enabled = params
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| "missing required param 'enabled'".to_string())?;
        to_json(ops::flows_set_enabled(&config, id.trim(), enabled).await?)
    })
}
