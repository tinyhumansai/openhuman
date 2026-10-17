//! `mcp_clients` handler implementations — deserialise params and delegate
//! to `ops.rs`.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;

use super::params::{
    read_optional, read_optional_string, read_optional_u32, read_required, to_json,
};

// ── Handler implementations ──────────────────────────────────────────────────

pub(super) fn handle_registry_search(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let query = read_optional_string(&params, "query")?;
        let transport = read_optional_string(&params, "transport")?;
        let page = read_optional_u32(&params, "page")?;
        let page_size = read_optional_u32(&params, "page_size")?;
        to_json(
            crate::mcp::registry::ops::mcp_clients_registry_search(
                &config, query, transport, page, page_size,
            )
            .await?,
        )
    })
}

pub(super) fn handle_registry_get(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let qualified_name = read_required::<String>(&params, "qualified_name")?;
        to_json(crate::mcp::registry::ops::mcp_clients_registry_get(&config, qualified_name).await?)
    })
}

pub(super) fn handle_installed_list(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        let config = ctx.load_config().await?;
        to_json(crate::mcp::registry::ops::mcp_clients_installed_list(&config).await?)
    })
}

pub(super) fn handle_update_env(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let server_id = read_required::<String>(&params, "server_id")?;
        let env = read_required::<std::collections::HashMap<String, String>>(&params, "env")?;
        to_json(crate::mcp::registry::ops::mcp_clients_update_env(&config, server_id, env).await?)
    })
}

pub(super) fn handle_uninstall(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let server_id = read_required::<String>(&params, "server_id")?;
        to_json(crate::mcp::registry::ops::mcp_clients_uninstall(&config, server_id).await?)
    })
}

pub(super) fn handle_detect_auth(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let server_id = read_required::<String>(&params, "server_id")?;
        to_json(crate::mcp::registry::ops::mcp_clients_detect_auth(&config, server_id).await?)
    })
}

pub(super) fn handle_oauth_begin(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let server_id = read_required::<String>(&params, "server_id")?;
        to_json(crate::mcp::registry::ops::mcp_clients_oauth_begin(&config, server_id).await?)
    })
}

pub(super) fn handle_connect(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let server_id = read_required::<String>(&params, "server_id")?;
        to_json(crate::mcp::registry::ops::mcp_clients_connect(&config, server_id).await?)
    })
}

pub(super) fn handle_disconnect(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let server_id = read_required::<String>(&params, "server_id")?;
        to_json(crate::mcp::registry::ops::mcp_clients_disconnect(&config, server_id).await?)
    })
}

pub(super) fn handle_set_enabled(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let server_id = read_required::<String>(&params, "server_id")?;
        let enabled = read_required::<bool>(&params, "enabled")?;
        to_json(
            crate::mcp::registry::ops::mcp_clients_set_enabled(&config, server_id, enabled).await?,
        )
    })
}

pub(super) fn handle_status(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        let config = ctx.load_config().await?;
        to_json(crate::mcp::registry::ops::mcp_clients_status(&config).await?)
    })
}

pub(super) fn handle_list_tools(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let server_id = read_required::<String>(&params, "server_id")?;
        to_json(
            crate::mcp::registry::ops::mcp_clients_list_tools(
                &config,
                server_id,
                crate::mcp::registry::ops::Caller::Rpc,
            )
            .await?,
        )
    })
}

pub(super) fn handle_tool_call(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let server_id = read_required::<String>(&params, "server_id")?;
        let tool_name = read_required::<String>(&params, "tool_name")?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or(Value::Object(Map::new()));
        let config = ctx.load_config().await?;
        to_json(
            crate::mcp::registry::ops::mcp_clients_tool_call(
                &config, server_id, tool_name, arguments,
            )
            .await?,
        )
    })
}

pub(super) fn handle_config_get(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        let config = ctx.load_config().await?;
        to_json(crate::mcp::registry::config_ops::mcp_clients_config_get(&config).await?)
    })
}

pub(super) fn handle_config_set(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        // The whole document is the parameter set: `{ "mcpServers": { … } }`
        // is what a user pastes, and wrapping it in another key would make the
        // wire shape differ from the file shape for no reason.
        let doc = Value::Object(params);
        to_json(crate::mcp::registry::config_ops::mcp_clients_config_set(&config, doc).await?)
    })
}

pub(super) fn handle_registry_settings_get(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let _ = params;
        let config = ctx.load_config().await?;
        to_json(crate::mcp::registry::ops::mcp_clients_registry_settings_get(&config).await?)
    })
}

pub(super) fn handle_registry_settings_set(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let smithery_api_key = read_optional::<String>(&params, "smithery_api_key")?;
        let mcp_official_base = read_optional::<String>(&params, "mcp_official_base")?;
        let mcp_official_token = read_optional::<String>(&params, "mcp_official_token")?;
        let mut config = ctx.load_config().await?;
        to_json(
            crate::mcp::registry::ops::mcp_clients_registry_settings_set(
                &mut config,
                smithery_api_key,
                mcp_official_base,
                mcp_official_token,
            )
            .await?,
        )
    })
}
