use serde_json::{json, Map, Value};

use crate::core::all;
use crate::openhuman::agent::harness::AgentDefinitionRegistry;
use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::inference::provider::traits::build_tool_instructions_text;
use crate::openhuman::security::{SecurityPolicy, ToolOperation};

use super::super::write_dispatch;
use super::params::{build_rpc_params, validate_controller_params};
use super::specs::{
    base_tool_specs, list_tools_result_for_config, list_tools_result_from_specs, searxng_tool_spec,
    tool_specs,
};
use super::types::ToolCallError;

pub async fn list_tools_result() -> Value {
    match config_rpc::load_config_with_timeout().await {
        Ok(config) => list_tools_result_for_config(&config),
        Err(err) => {
            log::warn!(
                "[mcp_server] tools/list config load failed; omitting config-gated tools: {err}"
            );
            list_tools_result_from_specs(base_tool_specs())
        }
    }
}

pub async fn call_tool(
    name: &str,
    arguments: Value,
    client_info: &str,
) -> Result<Value, ToolCallError> {
    let spec = tool_specs()
        .into_iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| ToolCallError::InvalidParams(format!("unknown MCP tool `{name}`")))?;

    let audit_arguments = arguments.clone();
    let mut params = match build_rpc_params(spec.name, arguments) {
        Ok(params) => params,
        Err(err) => {
            if write_dispatch::is_write_tool(spec.name) {
                write_dispatch::audit_write_rejection_without_config(
                    spec.name,
                    &audit_arguments,
                    client_info,
                    err.message(),
                );
            }
            return Err(err);
        }
    };
    match spec.name {
        "core.list_tools" => {
            enforce_read_policy(spec.name).await?;
            return list_core_tools().await;
        }
        "core.tool_instructions" => {
            enforce_read_policy(spec.name).await?;
            return core_tool_instructions().await;
        }
        "agent.list_subagents" => {
            enforce_read_policy(spec.name).await?;
            return list_subagents().await;
        }
        "agent.run_subagent" => {
            enforce_act_policy(spec.name).await?;
            return run_subagent_tool(&params).await;
        }
        "memory.store" | "memory.note" | "tree.tag" => {
            let config = write_dispatch::load_write_config(spec.name).await?;
            if let Err(err) = write_dispatch::enforce_write_policy_for_config(spec.name, &config) {
                write_dispatch::audit_write_rejection(
                    &config,
                    spec.name,
                    &audit_arguments,
                    Some(&params),
                    client_info,
                    &err,
                );
                return Err(err);
            }
            params.insert(
                "source_type".to_string(),
                Value::String(client_info.to_string()),
            );
            if let Err(err) = validate_controller_params(&spec, &params) {
                write_dispatch::audit_write_rejection(
                    &config,
                    spec.name,
                    &audit_arguments,
                    Some(&params),
                    client_info,
                    &err,
                );
                return Err(err);
            }
            return write_dispatch::dispatch_write_tool(
                spec.name,
                &params,
                &audit_arguments,
                client_info,
                &config,
            )
            .await;
        }
        _ => {}
    }

    validate_controller_params(&spec, &params)?;
    enforce_read_policy(spec.name).await?;

    let rpc_method = spec.rpc_method.ok_or_else(|| {
        ToolCallError::Internal(format!(
            "MCP tool `{}` is missing its RPC mapping",
            spec.name
        ))
    })?;

    log::debug!(
        "[mcp_server] tools/call dispatch tool={} rpc_method={} arg_keys={:?}",
        spec.name,
        rpc_method,
        params.keys().collect::<Vec<_>>()
    );

    match all::try_invoke_registered_rpc(rpc_method, params).await {
        Some(Ok(value)) => {
            log::debug!("[mcp_server] tools/call success tool={}", spec.name);
            Ok(tool_success(value))
        }
        Some(Err(message)) => {
            log::warn!(
                "[mcp_server] tools/call handler error tool={} error={}",
                spec.name,
                message
            );
            Ok(tool_error(format!("{} failed: {message}", spec.name)))
        }
        None => {
            log::error!(
                "[mcp_server] tools/call mapping missing registered RPC method tool={} rpc_method={}",
                spec.name,
                rpc_method
            );
            Ok(tool_error(format!(
                "{} is unavailable: mapped RPC method `{}` is not registered",
                spec.name, rpc_method
            )))
        }
    }
}

async fn enforce_read_policy(tool_name: &str) -> Result<(), ToolCallError> {
    // Config-load failure is an internal/server issue (disk error, corrupt
    // config), not bad client input — report it as `-32603 Internal error`
    // rather than `-32602 Invalid params`.
    let config = match config_rpc::load_config_with_timeout().await {
        Ok(config) => config,
        Err(err) => {
            log::warn!(
                "[mcp_server] enforce_read_policy config load failed tool={tool_name} error={err}"
            );
            return Err(ToolCallError::Internal(format!(
                "failed to load config: {err}"
            )));
        }
    };
    let policy =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    // A policy denial *is* something the caller can act on (toggle autonomy,
    // approve the tool) — keep that as `InvalidParams` so clients surface the
    // reason text instead of a generic internal-error banner.
    policy
        .enforce_tool_operation(ToolOperation::Read, tool_name)
        .map_err(ToolCallError::InvalidParams)
}

async fn enforce_act_policy(tool_name: &str) -> Result<(), ToolCallError> {
    let config = match config_rpc::load_config_with_timeout().await {
        Ok(config) => config,
        Err(err) => {
            log::warn!(
                "[mcp_server] enforce_act_policy config load failed tool={tool_name} error={err}"
            );
            return Err(ToolCallError::Internal(format!(
                "failed to load config: {err}"
            )));
        }
    };
    let policy =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    policy
        .enforce_tool_operation(ToolOperation::Act, tool_name)
        .map_err(ToolCallError::InvalidParams)
}

async fn load_config_and_init_registry() -> Result<crate::openhuman::config::Config, ToolCallError>
{
    let config = config_rpc::load_config_with_timeout()
        .await
        .map_err(|err| ToolCallError::Internal(format!("failed to load config: {err}")))?;
    AgentDefinitionRegistry::init_global(&config.workspace_dir).map_err(|err| {
        ToolCallError::Internal(format!(
            "failed to initialise AgentDefinitionRegistry: {err}"
        ))
    })?;
    Ok(config)
}

async fn build_orchestrator_agent() -> Result<Agent, ToolCallError> {
    let config = load_config_and_init_registry().await?;
    let mut agent = Agent::from_config_for_agent(&config, "orchestrator").map_err(|err| {
        ToolCallError::Internal(format!("failed to build orchestrator agent: {err}"))
    })?;
    agent.fetch_connected_integrations().await;
    let _ = agent.refresh_delegation_tools();
    Ok(agent)
}

async fn list_core_tools() -> Result<Value, ToolCallError> {
    let agent = build_orchestrator_agent().await?;
    let tools = agent
        .tool_specs()
        .iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "description": spec.description,
                "parameters": spec.parameters,
            })
        })
        .collect::<Vec<_>>();
    Ok(tool_success(json!({ "tools": tools })))
}

async fn core_tool_instructions() -> Result<Value, ToolCallError> {
    let agent = build_orchestrator_agent().await?;
    Ok(tool_text_success(build_tool_instructions_text(
        agent.tool_specs(),
    )))
}

async fn list_subagents() -> Result<Value, ToolCallError> {
    let config = load_config_and_init_registry().await?;
    let registry = AgentDefinitionRegistry::global().ok_or_else(|| {
        ToolCallError::Internal("AgentDefinitionRegistry missing after init".to_string())
    })?;

    let definitions = registry
        .list()
        .into_iter()
        .map(|def| {
            json!({
                "id": def.id,
                "display_name": def.display_name(),
                "when_to_use": def.when_to_use,
                "temperature": def.temperature,
                "max_iterations": def.max_iterations,
                "sandbox_mode": def.sandbox_mode,
                "tool_scope": def.tools,
                "subagents": def.subagents,
                "source": def.source,
            })
        })
        .collect::<Vec<_>>();

    let summary = format!(
        "# OpenHuman Subagents\n\nWorkspace: `{}`\n\n{}",
        config.workspace_dir.display(),
        definitions
            .iter()
            .map(|def| {
                let id = def.get("id").and_then(Value::as_str).unwrap_or("<unknown>");
                let when = def.get("when_to_use").and_then(Value::as_str).unwrap_or("");
                format!("- **{id}**: {when}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    );

    Ok(json!({
        "content": [{
            "type": "text",
            "text": summary,
        }],
        "structuredContent": {
            "definitions": definitions,
        }
    }))
}

async fn run_subagent_tool(params: &Map<String, Value>) -> Result<Value, ToolCallError> {
    use super::super::subagent_depth;
    use super::params::required_non_empty_string;

    let agent_id = required_non_empty_string(params, "agent_id")?;
    let prompt = required_non_empty_string(params, "prompt")?;
    if agent_id == "integrations_agent" {
        return Err(ToolCallError::InvalidParams(
            "agent.run_subagent does not yet support `integrations_agent`; first-level MCP support is currently limited to standalone agents that do not require toolkit binding".to_string(),
        ));
    }

    // Bound nested recursion per delegation chain (CC → run_subagent → CC → …).
    // `current_depth()` is the depth of THIS chain (carried across the loopback
    // MCP hop via the depth header); the subagent we're about to spawn sits one
    // level deeper. Refuse before spawning if the child would exceed the cap —
    // unrelated parallel callers each track their own chain, so they never trip
    // each other.
    // Guard BEFORE incrementing so a forged/clamped depth at the cap cannot
    // overflow `chain_depth + 1` (panic in debug, wrap-to-0 in release).
    let chain_depth = subagent_depth::current_depth();
    if chain_depth >= subagent_depth::MAX_SUBAGENT_DEPTH {
        return Err(ToolCallError::Internal(format!(
            "agent.run_subagent delegation depth limit reached (chain depth {chain_depth} ≥ {}); refusing to spawn another nested subagent",
            subagent_depth::MAX_SUBAGENT_DEPTH
        )));
    }
    let child_depth = chain_depth + 1;

    let config = load_config_and_init_registry().await?;
    let mut agent = Agent::from_config_for_agent(&config, &agent_id).map_err(|err| {
        ToolCallError::InvalidParams(format!("failed to build agent `{agent_id}`: {err}"))
    })?;
    agent.set_event_context(
        format!("mcp:{}:{}", agent_id, uuid::Uuid::new_v4()),
        "mcp_server",
    );
    agent.fetch_connected_integrations().await;
    let _ = agent.refresh_delegation_tools();

    // The MCP server surface exposes openhuman agents to remote MCP
    // clients. Treat callers as ExternalChannel — their prompt text is
    // remote-controlled and any external_effect tool the agent tries to
    // run must route through the gate's audit + TTL-deny path.
    let origin = crate::openhuman::agent::turn_origin::AgentTurnOrigin::ExternalChannel {
        channel: "mcp_server".to_string(),
        // MCP server callers don't carry a per-user identity at this
        // layer — the calling MCP client is the addressing primitive.
        // Leave sender unset; the gate's per-channel TTL-deny still
        // gates any external_effect tool the agent tries to run.
        sender: None,
        reply_target: agent_id.clone(),
        message_id: uuid::Uuid::new_v4().to_string(),
    };
    // Run the subagent one level deeper in the chain, so its own Claude Code
    // turns stamp `child_depth` onto any grandchildren they spawn. The agent
    // turn future is large; box it onto the heap so this tool (and the
    // `handle_json_value` dispatch above it) keeps a small stack frame.
    let response = Box::pin(subagent_depth::scope(
        child_depth,
        crate::openhuman::agent::turn_origin::with_origin(origin, agent.run_single(&prompt)),
    ))
    .await
    .map_err(|err| ToolCallError::Internal(format!("subagent `{agent_id}` failed: {err}")))?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": response,
        }],
        "structuredContent": {
            "agent_id": agent_id,
            "response": response,
        }
    }))
}

pub fn tool_success(value: Value) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
        }]
    })
}

fn tool_text_success(text: String) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": text,
        }]
    })
}

pub fn tool_error(message: String) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": message,
        }],
        "isError": true
    })
}

#[cfg(test)]
mod depth_tests {
    use super::super::super::subagent_depth::{current_depth, scope, MAX_SUBAGENT_DEPTH};

    #[tokio::test]
    async fn child_depth_is_bounded_per_chain() {
        // The dispatch refuses when `current_depth() >= MAX` (guarding before the
        // `+1` so a clamped depth at the cap can't overflow). At depth MAX the
        // next subagent is refused; below it, allowed. (Parallel unrelated chains
        // each start at 0 — no interference.)
        assert_eq!(current_depth(), 0, "top level starts at depth 0");
        scope(MAX_SUBAGENT_DEPTH, async {
            assert!(
                current_depth() >= MAX_SUBAGENT_DEPTH,
                "at the cap, spawning a deeper child must be refused"
            );
        })
        .await;
        scope(MAX_SUBAGENT_DEPTH - 1, async {
            assert!(
                current_depth() < MAX_SUBAGENT_DEPTH,
                "one below the cap, a child is still allowed"
            );
        })
        .await;
    }
}
