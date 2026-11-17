//! Registration of flow authoring, execution, discovery, and memory tools.

use super::builder_tools_connection_reads::FlowsConnectionProvider;
use super::copilot_bus::CopilotToolCatalog;
use super::memory_tools::{FlowMemoryRecallTool, FlowMemoryRememberTool};
use crate::config::Config;
use crate::security::SecurityPolicy;
use std::sync::Arc;
use tinytools::Tool;

fn push_copilot_tool(
    tools: &mut Vec<Box<dyn Tool>>,
    catalog: Option<&CopilotToolCatalog>,
    name: &str,
) {
    let Some(catalog) = catalog else {
        return;
    };
    match catalog.tool(name) {
        Ok(tool) => tools.push(Box::new(tool)),
        Err(error) => {
            tracing::error!(tool = name, %error, "[flows] Copilot tool missing from module catalogue")
        }
    }
}

/// Build the flow tool family in its persisted catalogue order.
pub(crate) fn tools(config: Arc<Config>, security: &Arc<SecurityPolicy>) -> Vec<Box<dyn Tool>> {
    let copilot = match CopilotToolCatalog::load(config.clone()) {
        Ok(catalog) => Some(catalog),
        Err(error) => {
            tracing::warn!(%error, "[flows] TinyFlows Copilot tools unavailable");
            None
        }
    };
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    // The owning module supplies these descriptors and executes every call.
    // Keep their existing catalogue order because transcripts persist it.
    for name in [
        "propose_workflow",
        "revise_workflow",
        "edit_workflow",
        "validate_workflow",
        "get_flow_history",
        "list_flow_runs",
        "resume_flow_run",
        "cancel_flow_run",
        "create_workflow",
        "duplicate_flow",
        "list_flows",
        "get_flow",
        "get_flow_run",
    ] {
        push_copilot_tool(&mut tools, copilot.as_ref(), name);
    }

    tools.push(Box::new(
        crate::integrations::composio::builder_tools::ListFlowConnectionsTool::new(
            config.clone(),
            Arc::new(FlowsConnectionProvider),
        ),
    ));
    tools.push(Box::new(
        crate::integrations::composio::builder_tools::SearchToolCatalogTool::new(config.clone()),
    ));
    tools.push(Box::new(
        crate::integrations::composio::builder_tools::GetToolContractTool::new(config.clone()),
    ));
    tools.push(Box::new(
        crate::integrations::composio::builder_tools::GetToolOutputSampleTool::new(config.clone()),
    ));

    push_copilot_tool(&mut tools, copilot.as_ref(), "list_agent_definitions");
    tools.push(Box::new(
        crate::integrations::composio::builder_tools::ListConnectableToolkitsTool::new(
            config.clone(),
            Arc::new(FlowsConnectionProvider),
        ),
    ));
    for name in [
        "list_node_kinds",
        "get_node_kind_contract",
        "dry_run_workflow",
        "run_flow",
        "save_workflow",
        "suggest_workflows",
    ] {
        push_copilot_tool(&mut tools, copilot.as_ref(), name);
    }

    // Per-flow sandboxed memory remains a host-owned tool family.
    tools.push(Box::new(FlowMemoryRecallTool::new()));
    tools.push(Box::new(FlowMemoryRememberTool::new(security.clone())));
    tools
}

#[cfg(test)]
#[path = "registration_tests.rs"]
mod tests;
