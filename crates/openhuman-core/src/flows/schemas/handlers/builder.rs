use super::super::*;
use super::{read_required, to_json};
use crate::core::runtime::DispatchCtx;

pub(in crate::flows::schemas) fn handle_build(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        // Optional streaming target: when the copilot passes its chat `thread_id`
        // the builder turn streams live text/tool/proposal events into that
        // thread (Phase B). Read + strip the transport-only keys before the rest
        // of the object is deserialized into the Copilot bus request.
        let stream = read_flow_stream_target(&params);
        // Deserialize the remaining param object into the stable Copilot bus
        // request shape. Stream keys are not part of that request.
        let req: tinyflows_bus::copilot::BuilderPromptRequest =
            serde_json::from_value(Value::Object(params))
                .map_err(|e| format!("invalid flows.build params: {e}"))?;
        to_json(ops::flows_build(&ctx, &config, req, stream).await?)
    })
}

pub(in crate::flows::schemas) fn handle_build_cancel(
    _ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let thread_id = read_required::<String>(&params, "thread_id")?;
        let request_id = params
            .get("request_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        to_json(ops::flows_build_cancel(thread_id.trim(), request_id.as_deref()).await?)
    })
}

pub(in crate::flows::schemas) fn handle_discover(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        // Optional streaming target for the Flow Scout run (Phase B) — same
        // `thread_id`/`request_id` convention as `flows.build`.
        let stream = read_flow_stream_target(&params);
        to_json(ops::flows_discover(&ctx, &config, stream).await?)
    })
}

/// Read the optional `thread_id` / `request_id` streaming params shared by
/// `flows.build` and `flows.discover` into an [`ops::FlowStreamTarget`].
/// Returns `None` (headless run) when no usable `thread_id` is present; a
/// missing `request_id` is filled with a fresh uuid inside `from_params`.
pub(in crate::flows::schemas) fn read_flow_stream_target(
    params: &Map<String, Value>,
) -> Option<ops::FlowStreamTarget> {
    let thread_id = params
        .get("thread_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let request_id = params
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    ops::FlowStreamTarget::from_params(thread_id, request_id)
}

pub(in crate::flows::schemas) fn handle_list_suggestions(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let status = params
            .get("status")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|status| match status {
                "dismissed" => tinyflows_bus::catalog::SuggestionStatus::Dismissed,
                "built" => tinyflows_bus::catalog::SuggestionStatus::Built,
                _ => tinyflows_bus::catalog::SuggestionStatus::New,
            });
        to_json(ops::flows_list_suggestions(&config, status).await?)
    })
}

pub(in crate::flows::schemas) fn handle_dismiss_suggestion(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_dismiss_suggestion(&config, id.trim()).await?)
    })
}

pub(in crate::flows::schemas) fn handle_mark_suggestion_built(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_mark_suggestion_built(&config, id.trim()).await?)
    })
}

pub(in crate::flows::schemas) fn handle_required_connections(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let graph = read_required::<Value>(&params, "graph")?;
        to_json(ops::flows_required_connections(&ctx, &config, graph).await?)
    })
}

pub(in crate::flows::schemas) fn handle_approval_manifest(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);
        let graph = params.get("graph").filter(|v| !v.is_null()).cloned();
        to_json(ops::flows_approval_manifest(&ctx, &config, id.as_deref(), graph).await?)
    })
}

pub(in crate::flows::schemas) fn handle_search_tool_catalog(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let query = read_required::<String>(&params, "query")?;
        let toolkit = params
            .get("toolkit")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(25);
        to_json(ops::flows_search_tool_catalog(&config, query.trim(), toolkit, limit).await?)
    })
}

pub(in crate::flows::schemas) fn handle_get_tool_contract(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let slug = read_required::<String>(&params, "slug")?;
        to_json(ops::flows_get_tool_contract(&config, slug.trim()).await?)
    })
}

pub(in crate::flows::schemas) fn handle_get_history(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(20);
        to_json(ops::flows_get_history(&config, id.trim(), limit)?)
    })
}

pub(in crate::flows::schemas) fn handle_rollback(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let id = read_required::<String>(&params, "id")?;
        let revision_id = read_required::<String>(&params, "revision_id")?;
        let expected_version = params
            .get("expected_version")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        to_json(
            ops::flows_rollback_with_context(
                &ctx,
                &config,
                id.trim(),
                revision_id.trim(),
                expected_version,
            )
            .await?,
        )
    })
}
