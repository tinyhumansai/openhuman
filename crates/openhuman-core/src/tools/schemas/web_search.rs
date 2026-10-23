//! Handlers for the search controllers: `tools_web_search`, `tools_web_answer`,
//! `tools_web_contents`, and the provider-pinned `tools_searxng_search` that
//! MCP clients use. All of them call the TinySearch module's role tools, so an
//! RPC caller gets the same provider choice and fallback as the agent.

use crate::core::runtime::DispatchCtx;
use serde_json::{json, Map, Value};

use crate::core::all::ControllerFuture;

fn required_text(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("missing or empty `{key}`"))
}

fn optional_text(params: &Map<String, Value>, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Run one role tool and shape the response for RPC callers.
async fn run_role_tool(
    ctx: &DispatchCtx,
    tool: &'static str,
    arguments: Value,
    method: &str,
) -> Result<Value, String> {
    let config = ctx.load_config().await?;
    run_role_tool_with(ctx, &config, tool, arguments, method).await
}

#[cfg(feature = "modules")]
async fn run_role_tool_with(
    ctx: &DispatchCtx,
    config: &crate::config::Config,
    tool: &'static str,
    arguments: Value,
    method: &str,
) -> Result<Value, String> {
    if !crate::search::is_active(config) {
        return Err("web search is disabled in settings".to_string());
    }
    // JSON-RPC, MCP, and agent callers share this dispatcher; enforce egress
    // policy here so non-agent routes cannot bypass LocalOnly mode.
    if let Some(message) = crate::search::tools::local_only_search_block(tool).await {
        return Err(message);
    }
    ensure_servable(config, tool, &arguments)?;
    let request = tinysearch_bus::ExecuteToolRequest {
        name: tool.to_string(),
        arguments,
    };
    let response = crate::modules::search::execute_tool(ctx, config, request)
        .await
        .map_err(|error| crate::search::tools::user_facing_error(&error))?;
    let payload = json!({
        "provider": tinysearch_bus::labels::provider_label(&response.provider),
        "provider_id": response.provider,
        "role": response.role,
        "results": response.results,
        "citations": response.citations,
        "answer": response.answer,
        "status": response.status,
        "fallback_from": response.fallback_from,
    });
    let log = vec![format!(
        "tools.{method}: provider={} results={} fallbacks={}",
        response.provider,
        response.results.len(),
        response.fallback_from.len()
    )];
    crate::core::Outcome::new(payload, log).into_cli_compatible_json()
}

/// Refuse early, without loading the module, when nothing can serve the call.
#[cfg(feature = "modules")]
fn ensure_servable(
    config: &crate::config::Config,
    tool: &str,
    arguments: &Value,
) -> Result<(), String> {
    use crate::search::providers::{effective_role_providers, resolve};
    let role = tinysearch_bus::role_for_tool(tool)
        .ok_or_else(|| format!("{tool} is not a search role tool"))?;
    let resolved = resolve(config);
    let usable = effective_role_providers(&resolved, config, role);
    let pinned = arguments.get("provider").and_then(Value::as_str);
    let servable = match pinned {
        Some(provider) => resolved
            .iter()
            .any(|p| p.id == provider && p.usable && p.roles.contains(&role)),
        None => !usable.is_empty(),
    };
    if servable {
        return Ok(());
    }
    tracing::debug!(
        tool,
        pinned = pinned.unwrap_or("auto"),
        "[rpc][tools.search] no usable provider"
    );
    Err(format!(
        "No web search provider is available for {}. Sign in to TinyHumans for the included \
         providers, or turn on a provider with your own key under Connections → Search.",
        pinned
            .map(|p| format!("`{p}`"))
            .unwrap_or_else(|| "this request".to_string())
    ))
}

#[cfg(not(feature = "modules"))]
async fn run_role_tool_with(
    _ctx: &DispatchCtx,
    _config: &crate::config::Config,
    _tool: &'static str,
    _arguments: Value,
    _method: &str,
) -> Result<Value, String> {
    Err("web search needs the modules feature, which this build does not include".to_string())
}

fn search_arguments(params: &Map<String, Value>, provider: Option<&str>) -> Result<Value, String> {
    let query = required_text(params, "query")?;
    let mut arguments = json!({ "query": query });
    if let Some(max) = params.get("max_results").and_then(Value::as_u64) {
        arguments["max_results"] = json!(max.clamp(1, 20));
    }
    if let Some(provider) = provider
        .map(str::to_string)
        .or_else(|| optional_text(params, "provider"))
    {
        arguments["provider"] = json!(provider);
    }
    let pinned = arguments
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("auto")
        .to_string();
    tracing::debug!(
        query_len = query.chars().count(),
        provider = %pinned,
        "[rpc][tools.search] request"
    );
    Ok(arguments)
}

pub(super) fn handle_web_search(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let arguments = search_arguments(&params, None)?;
        run_role_tool(
            &ctx,
            tinysearch_bus::tools::WEB_SEARCH,
            arguments,
            "web_search",
        )
        .await
    })
}

pub(super) fn handle_searxng_search(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let arguments = search_arguments(&params, Some("searxng"))?;
        run_role_tool(
            &ctx,
            tinysearch_bus::tools::WEB_SEARCH,
            arguments,
            "searxng_search",
        )
        .await
    })
}

pub(super) fn handle_web_answer(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let query = required_text(&params, "query")?;
        let mut arguments = json!({ "query": query });
        if let Some(depth) = optional_text(&params, "depth") {
            if depth != "quick" && depth != "deep" {
                return Err("`depth` must be quick or deep".to_string());
            }
            arguments["depth"] = json!(depth);
        }
        if let Some(provider) = optional_text(&params, "provider") {
            arguments["provider"] = json!(provider);
        }
        tracing::debug!(
            query_len = query.chars().count(),
            "[rpc][tools.web_answer] request"
        );
        run_role_tool(
            &ctx,
            tinysearch_bus::tools::WEB_ANSWER,
            arguments,
            "web_answer",
        )
        .await
    })
}

pub(super) fn handle_web_contents(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let urls = optional_string_array(&params, "urls")?;
        if urls.is_empty() {
            return Err("`urls` must list at least one URL".to_string());
        }
        let mut arguments = json!({ "urls": urls });
        if let Some(query) = optional_text(&params, "query") {
            arguments["query"] = json!(query);
        }
        if let Some(provider) = optional_text(&params, "provider") {
            arguments["provider"] = json!(provider);
        }
        tracing::debug!(urls = urls.len(), "[rpc][tools.web_contents] request");
        run_role_tool(
            &ctx,
            tinysearch_bus::tools::WEB_CONTENTS,
            arguments,
            "web_contents",
        )
        .await
    })
}

pub(super) fn optional_string_array(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let Some(value) = params.get(key) else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let items = value
        .as_array()
        .ok_or_else(|| format!("`{key}` must be an array of strings"))?;
    items
        .iter()
        .filter_map(|item| match item.as_str() {
            Some(value) => {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| Ok(trimmed.to_string()))
            }
            None => Some(Err(format!("`{key}` must contain only strings"))),
        })
        .collect()
}
