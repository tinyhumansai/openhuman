//! GitHub-repo listing and trigger handlers: `list_github_repos`,
//! `create_trigger`, `list_trigger_history`, `list_available_triggers`,
//! `list_triggers`, `enable_trigger`, `disable_trigger`.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
use crate::integrations::composio::ops;

use super::params::{
    CreateTriggerParams, EnableTriggerParams, ListAvailableTriggersParams, ListGithubReposParams,
    ListTriggersParams, TriggerHistoryParams,
};
use super::util::{read_required_non_empty, to_json};

pub(super) fn handle_list_github_repos(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload: ListGithubReposParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        to_json(ops::composio_list_github_repos(&config, payload.connection_id).await?)
    })
}

pub(super) fn handle_create_trigger(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload: CreateTriggerParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let slug = payload.slug.trim();
        if slug.is_empty() {
            return Err("invalid params: 'slug' must not be empty".to_string());
        }
        to_json(
            ops::composio_create_trigger(
                &config,
                slug,
                payload.connection_id,
                payload.trigger_config,
            )
            .await?,
        )
    })
}

pub(super) fn handle_list_trigger_history(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload: TriggerHistoryParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        to_json(ops::composio_list_trigger_history(&ctx, &config, payload.limit).await?)
    })
}

pub(super) fn handle_list_available_triggers(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload: ListAvailableTriggersParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let toolkit = payload.toolkit.trim();
        if toolkit.is_empty() {
            return Err("invalid params: 'toolkit' must not be empty".to_string());
        }
        to_json(
            ops::composio_list_available_triggers(&config, toolkit, payload.connection_id).await?,
        )
    })
}

pub(super) fn handle_list_triggers(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload: ListTriggersParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        to_json(ops::composio_list_triggers(&config, payload.toolkit).await?)
    })
}

pub(super) fn handle_enable_trigger(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload: EnableTriggerParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let connection_id = payload.connection_id.trim();
        let slug = payload.slug.trim();
        if connection_id.is_empty() {
            return Err("invalid params: 'connection_id' must not be empty".to_string());
        }
        if slug.is_empty() {
            return Err("invalid params: 'slug' must not be empty".to_string());
        }
        to_json(
            ops::composio_enable_trigger(&config, connection_id, slug, payload.trigger_config)
                .await?,
        )
    })
}

pub(super) fn handle_disable_trigger(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let trigger_id = read_required_non_empty(&params, "trigger_id")?;
        to_json(ops::composio_disable_trigger(&config, &trigger_id).await?)
    })
}
